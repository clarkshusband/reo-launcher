use std::sync::Arc;

use arc_swap::ArcSwap;
use reo_events::EventBus;
use reqwest::{ClientBuilder, Response};
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;

use crate::config::NetConfig;
use crate::error::{RequestError, body_snippet};
use crate::request::HttpRequest;
use crate::response::{ResponseExt, ResponseOptions};

const MAX_THROTTLE_RETRIES: u32 = 6;

const MAX_INFLIGHT_REQUESTS: usize = 64;

/// Caps a whole JSON or metadata exchange
/// The client itself has none since it would also cut off a large download on
/// a slow link which the read timeout already guards against stalling
pub(crate) const API_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);

const CURSEFORGE_API_HOST: &str = "api.curseforge.com";

fn retry_after(response: &Response) -> Option<std::time::Duration> {
    let raw = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .to_string();
    let secs: u64 = raw.parse().ok()?;
    Some(std::time::Duration::from_secs(secs.min(60)))
}

fn backoff_delay(attempt: u32) -> std::time::Duration {
    let base = 500u64.saturating_mul(1 << attempt.min(6));
    let capped = base.min(30_000);
    let jitter = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis() as u64)
        .unwrap_or(0))
        % 250;
    std::time::Duration::from_millis(capped + jitter)
}

/// The key belongs to the API alone the CDN and website never need it
fn is_curseforge_api(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }

    url.host_str().is_some_and(|host| {
        host.trim_end_matches('.')
            .eq_ignore_ascii_case(CURSEFORGE_API_HOST)
    })
}

/// reqwest only strips its own sensitive headers on a cross-host redirect so
/// the key would follow one anywhere
fn redirect_leaks_curseforge_key(previous: &[reqwest::Url], next: &reqwest::Url) -> bool {
    previous.first().is_some_and(is_curseforge_api) && !is_curseforge_api(next)
}

/// Stops rather than follows so the caller sees the redirect as a failed status
fn redirect_policy() -> reqwest::redirect::Policy {
    let fallback = reqwest::redirect::Policy::default();

    reqwest::redirect::Policy::custom(move |attempt| {
        if redirect_leaks_curseforge_key(attempt.previous(), attempt.url()) {
            tracing::warn!(
                next = %attempt.url(),
                "not following a CurseForge API redirect off the API host"
            );
            attempt.stop()
        } else {
            fallback.redirect(attempt)
        }
    })
}

fn apply_curseforge_auth(
    request: &mut reqwest::Request,
    api_key: &str,
) -> Result<(), RequestError> {
    const API_KEY_HEADER: reqwest::header::HeaderName =
        reqwest::header::HeaderName::from_static("x-api-key");

    if !is_curseforge_api(request.url()) {
        return Ok(());
    }

    let headers = request.headers_mut();
    if headers.contains_key(&API_KEY_HEADER) {
        return Ok(());
    }

    let mut value = reqwest::header::HeaderValue::try_from(api_key)?;
    value.set_sensitive(true);
    headers.insert(API_KEY_HEADER, value);

    Ok(())
}

#[derive(Clone)]
pub struct RequestClient {
    client: reqwest::Client,
    semaphore: Arc<Semaphore>,
    /// Shared across every clone so a settings save is visible to in-flight
    /// handles
    /// `ArcSwap` rather than a lock which would serialise requests
    config: Arc<ArcSwap<NetConfig>>,
}

impl RequestClient {
    pub fn http(&self) -> &reqwest::Client {
        &self.client
    }

    #[must_use]
    pub fn config(&self) -> arc_swap::Guard<Arc<NetConfig>> {
        self.config.load()
    }

    pub fn set_config(&self, config: NetConfig) {
        self.config.store(Arc::new(config));
    }

    pub fn new(config: NetConfig) -> Result<Self, RequestError> {
        let mut builder = ClientBuilder::new()
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30))
            .redirect(redirect_policy())
            .tls_backend_rustls()
            .user_agent(format!(
                "Reo Launcher {} ({})",
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_HOMEPAGE")
            ));

        builder = builder.no_hickory_dns();

        let client = builder.build()?;

        Ok(Self {
            client,
            semaphore: Arc::new(Semaphore::new(MAX_INFLIGHT_REQUESTS)),
            config: Arc::new(ArcSwap::from_pointee(config)),
        })
    }
}

impl RequestClient {
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn send(&self, request: impl Into<HttpRequest>) -> Result<Response, RequestError> {
        let mut request: HttpRequest = request.into();
        let mut retries = 0;
        let mut throttle_retries = 0u32;

        if reo_common::consent::blocks_url(request.request.url().as_str()) {
            let url = request.request.url().to_string();
            tracing::debug!(%url, "request withheld: terms and privacy policy declined");
            return Err(RequestError::ConsentRequired { url });
        }

        apply_curseforge_auth(&mut request.request, &self.config.load().curseforge_api_key)?;

        let cloned_backup = request.request.try_clone();
        let cloneable = cloned_backup.is_some();

        let max_retries = if cloneable {
            request.options.max_retries
        } else {
            0
        };

        let mut active_request = request.request;

        tracing::debug!(
            method = %active_request.method(),
            url = %active_request.url(),
            "dispatching http request"
        );

        let res = loop {
            let permit = if request.options.use_semaphore {
                self.semaphore.acquire().await.ok()
            } else {
                None
            };

            match self.client.execute(active_request).await {
                Ok(response) => {
                    let status = response.status();
                    let retryable = status.as_u16() == 429 || status.is_server_error();

                    if retryable && cloneable && throttle_retries < MAX_THROTTLE_RETRIES {
                        throttle_retries += 1;
                        let delay = retry_after(&response)
                            .unwrap_or_else(|| backoff_delay(throttle_retries));
                        tracing::warn!(
                            status = status.as_u16(),
                            attempt = throttle_retries,
                            delay_ms = delay.as_millis() as u64,
                            url = %response.url(),
                            "rate limited / transient error; backing off"
                        );
                        drop(permit);
                        tokio::time::sleep(delay).await;
                        active_request = cloned_backup.as_ref().unwrap().try_clone().unwrap();
                        continue;
                    }

                    break response;
                }
                Err(err) => {
                    if retries < max_retries {
                        retries += 1;

                        let current_backup = cloned_backup.as_ref().unwrap();

                        active_request = current_backup.try_clone().unwrap();

                        tokio::time::sleep(std::time::Duration::from_millis(500 * retries as u64))
                            .await;

                        continue;
                    }

                    crate::status::note_request_result(false);
                    return Err(RequestError::ReqwestError(err));
                }
            }
        };

        crate::status::note_request_result(true);
        Ok(res)
    }

    #[tracing::instrument(level = "debug", skip(self, request, options, events), fields(dest = %dest.as_ref().display()))]
    pub async fn download_file(
        &self,
        request: impl Into<HttpRequest>,
        dest: impl AsRef<std::path::Path> + Send,
        options: ResponseOptions,
        events: &EventBus,
    ) -> Result<(), RequestError> {
        let res = self.send(request).await?;

        // Without this an error body is written to disk under the requested file's name
        let status = res.status();
        if !status.is_success() {
            let url = res.url().to_string();
            let bytes = res.bytes().await?;
            return Err(RequestError::HttpStatus {
                status: status.as_u16(),
                url,
                snippet: body_snippet(&bytes),
            });
        }

        let size_hint = res.content_length();
        let http_stream = res.stream(options, events).await?;
        let http_stream = std::pin::pin!(http_stream);

        polyio::write_stream(dest, http_stream, size_hint).await?;

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn send_as<T: DeserializeOwned>(
        &self,
        request: impl Into<HttpRequest>,
    ) -> Result<T, RequestError> {
        let mut request: HttpRequest = request.into();
        if request.request.timeout().is_none() {
            *request.request.timeout_mut() = Some(API_REQUEST_TIMEOUT);
        }

        let res = self.send(request).await?;
        let status = res.status();
        let url = res.url().to_string();
        let bytes = res.bytes().await?;

        if !status.is_success() {
            return Err(RequestError::HttpStatus {
                status: status.as_u16(),
                url,
                snippet: body_snippet(&bytes),
            });
        }

        serde_json::from_slice(&bytes).map_err(|err| RequestError::DeserializeError {
            source: err,
            type_name: std::any::type_name::<T>().to_string(),
            url,
            status: status.as_u16(),
            snippet: body_snippet(&bytes),
        })
    }

    #[tracing::instrument(level = "debug", skip(self, body, extra_headers), fields(method = %method, %url))]
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<serde_json::Value>,
        extra_headers: &[(&str, &str)],
    ) -> Result<T, RequestError> {
        let mut request = reqwest::Request::new(method, url);
        for (name, value) in extra_headers {
            request.headers_mut().insert(
                reqwest::header::HeaderName::try_from(*name)?,
                reqwest::header::HeaderValue::try_from(*value)?,
            );
        }

        if let Some(body) = body {
            let bytes = serde_json::to_vec(&body).map_err(RequestError::SerializeError)?;
            request.headers_mut().insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            *request.body_mut() = Some(bytes.into());
        }

        self.send_as(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(url: &str) -> reqwest::Request {
        reqwest::Request::new(reqwest::Method::GET, url.parse().unwrap())
    }

    fn api_key_of(url: &str) -> Option<String> {
        let mut req = request(url);
        apply_curseforge_auth(&mut req, reo_common::constants::CURSEFORGE_API_KEY).unwrap();
        req.headers()
            .get("x-api-key")
            .map(|v| v.to_str().unwrap().to_string())
    }

    #[test]
    fn attaches_key_to_the_curseforge_api() {
        for url in [
            "https://api.curseforge.com/v1/mods/search",
            "https://API.curseforge.com./v1/mods/1",
        ] {
            assert_eq!(
                api_key_of(url).as_deref(),
                Some(reo_common::constants::CURSEFORGE_API_KEY),
                "expected key on {url}"
            );
        }
    }

    #[test]
    fn ignores_unrelated_and_insecure_hosts() {
        for url in [
            "https://api.modrinth.com/v2/search",
            "https://cdn.modrinth.com/data/x/y.jar",
            "https://notcurseforge.com/x",
            "https://evil-forgecdn.net.attacker.com/x",
            "https://api.curseforge.com.attacker.com/x",
            "http://api.curseforge.com/v1/mods",
            "https://media.forgecdn.net/avatars/1/2/icon.png",
            "https://edge.forgecdn.net/files/1/2/mod.jar",
            "https://www.curseforge.com/minecraft",
        ] {
            assert_eq!(api_key_of(url), None, "unexpected key on {url}");
        }
    }

    #[test]
    fn a_keyed_request_is_not_redirected_off_the_api_host() {
        let parsed = |url: &str| -> reqwest::Url { url.parse().unwrap() };
        let api = [parsed("https://api.curseforge.com/v1/mods/1/files/2/download-url")];

        assert!(redirect_leaks_curseforge_key(
            &api,
            &parsed("https://edge.forgecdn.net/files/1/2/mod.jar")
        ));
        assert!(!redirect_leaks_curseforge_key(
            &api,
            &parsed("https://api.curseforge.com/v1/mods/1")
        ));
        assert!(!redirect_leaks_curseforge_key(
            &[parsed("https://www.curseforge.com/api/v1/mods/1/files/2/download")],
            &parsed("https://edge.forgecdn.net/files/1/2/mod.jar")
        ));
    }

    #[test]
    fn does_not_override_explicit_key() {
        let mut req = request("https://api.curseforge.com/v1/mods");
        req.headers_mut().insert(
            "x-api-key",
            reqwest::header::HeaderValue::from_static("mine"),
        );
        apply_curseforge_auth(&mut req, reo_common::constants::CURSEFORGE_API_KEY).unwrap();
        assert_eq!(req.headers()["x-api-key"], "mine");
    }
}
