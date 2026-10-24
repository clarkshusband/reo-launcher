use reqwest::Response;
use uuid::Uuid;

use crate::error::RequestError;
use reo_events::{EventBus, GroupedProgressChild};

const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Clone, Default)]
pub struct ResponseOptions {
    pub notify: Option<ResponseNotifyOptions>,
}

#[derive(Clone)]
pub struct ResponseNotifyOptions {
    child: Option<GroupedProgressChild>,
    standalone_label: Option<String>,
    standalone_id: Option<Uuid>,
    done_label: Option<String>,
}

impl ResponseNotifyOptions {
    pub fn grouped(child: GroupedProgressChild) -> Self {
        Self {
            child: Some(child),
            standalone_label: None,
            standalone_id: None,
            done_label: None,
        }
    }

    pub fn standalone(label: impl Into<String>) -> Self {
        Self {
            child: None,
            standalone_label: Some(label.into()),
            standalone_id: None,
            done_label: None,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.standalone_id = Some(id);
        self
    }

    pub fn done_label(mut self, label: impl Into<String>) -> Self {
        self.done_label = Some(label.into());
        self
    }

    #[must_use]
    pub fn child(&self) -> Option<&GroupedProgressChild> {
        self.child.as_ref()
    }

    /// Pins the standalone notification id so a retried request keeps updating
    /// one notification instead of opening a fresh UI entry per attempt
    #[must_use]
    pub fn pinned(mut self) -> Self {
        if self.standalone_label.is_some() && self.standalone_id.is_none() {
            self.standalone_id = Some(Uuid::new_v4());
        }
        self
    }
}

#[async_trait::async_trait]
pub trait ResponseExt {
    async fn stream(
        self,
        options: ResponseOptions,
        events: &EventBus,
    ) -> Result<impl futures_lite::Stream<Item = Result<bytes::Bytes, RequestError>>, RequestError>;
}

/// One event per chunk would bury the UI in tens of thousands of updates so
/// this samples them and still makes sure the bar lands on complete
struct ProgressReporter {
    events: EventBus,
    child: Option<GroupedProgressChild>,
    standalone: Option<(Uuid, String)>,
    done_label: Option<String>,
    /// `None` without a Content-Length as with chunked or decompressed bodies
    total: Option<u64>,
    current: u64,
    last_emit: Option<std::time::Instant>,
    /// Bytes arrived since the last event
    pending: bool,
}

impl ProgressReporter {
    fn report(&self, ended: bool) {
        if let Some(child) = &self.child {
            // `None` keeps whatever total the child was created with
            child.set_progress(self.current, self.total);
        } else if let Some((id, label)) = &self.standalone {
            let done = self.total.map_or(ended, |total| self.current >= total);
            let label = if done {
                self.done_label.as_deref().unwrap_or(label.as_str())
            } else {
                label.as_str()
            };
            let (current, total) = match self.total {
                Some(total) => (self.current, total),
                // Without a known size the end of the body is what completes it
                None if ended => (self.current.max(1), self.current.max(1)),
                // 0 is how the notification layer shows progress with no known end
                None => (self.current, 0),
            };
            self.events.progress(*id, label, current, total);
        }
    }

    fn advance(&mut self, len: u64) {
        self.current += len;

        let now = std::time::Instant::now();
        let reached = self.total.is_some_and(|total| self.current >= total);
        let due = reached
            || self
                .last_emit
                .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);

        if due {
            self.last_emit = Some(now);
            self.pending = false;
            self.report(false);
        } else {
            self.pending = true;
        }
    }

    fn end(&mut self) {
        if self.pending || self.total.is_none() {
            self.pending = false;
            self.report(true);
        }
    }
}

#[async_trait::async_trait]
impl ResponseExt for Response {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn stream(
        self,
        options: ResponseOptions,
        events: &EventBus,
    ) -> Result<impl futures_lite::Stream<Item = Result<bytes::Bytes, RequestError>>, RequestError>
    {
        let child = options.notify.as_ref().and_then(|n| n.child.clone());
        let standalone = options.notify.as_ref().and_then(|n| {
            let label = n.standalone_label.clone()?;
            Some((n.standalone_id.unwrap_or_else(Uuid::new_v4), label))
        });
        let done_label = options.notify.as_ref().and_then(|n| n.done_label.clone());

        let mut reporter = ProgressReporter {
            events: events.clone(),
            child,
            standalone,
            done_label,
            total: self.content_length().filter(|&len| len > 0),
            current: 0,
            last_emit: None,
            pending: false,
        };
        reporter.report(false);

        // The trailing `None` marks the end of the body so an unknown total
        // can still be reported as complete
        let body = futures_lite::StreamExt::chain(
            futures_lite::StreamExt::map(self.bytes_stream(), Some),
            futures_lite::stream::once(None),
        );
        let stream = futures_lite::StreamExt::filter_map(body, move |item| match item {
            Some(Ok(chunk)) => {
                reporter.advance(chunk.len() as u64);
                Some(Ok(chunk))
            }
            Some(Err(err)) => Some(Err(RequestError::from(err))),
            None => {
                reporter.end();
                None
            }
        });

        Ok(stream)
    }
}
