use reo_db::dao::artifact as artifact_dao;
use reo_db::models::ArtifactRow;

use super::paths::{artifact_absolute_path, cache_file_path, relative_cache_path};
use crate::ctx::ContentCtx;
use crate::error::{ContentError, ContentResult};
use crate::packages::PackageError;
use crate::packages::types::{ExternalFile, VersionDetail, VersionFile};
use reo_common::domain::{ContentType, ProviderId};
use reo_events::GroupedProgressChild;
use polyio::{normalize_hash, sha1_file};

#[tracing::instrument(level = "debug", skip(child, ctx))]
pub async fn ensure_artifact_file(
    hash: &str,
    url: &str,
    dest: &std::path::Path,
    child: Option<&GroupedProgressChild>,
    ctx: &ContentCtx,
) -> ContentResult<u64> {
    if reo_net::matches_on_disk(dest, hash).await {
        return Ok(polyio::stat(dest).await?.len());
    }

    tracing::debug!("downloading artifact file");
    // Modrinth and CurseForge both index by SHA-1
    let expected = polyio::Checksum::sha1(hash);
    reo_net::download_verified(
        &ctx.net,
        &ctx.events,
        url,
        dest,
        Some(&expected),
        0,
        child
            .cloned()
            .map(reo_net::ResponseNotifyOptions::grouped),
    )
    .await?;

    Ok(polyio::stat(dest).await?.len())
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(level = "debug", skip(version, file, child, ctx))]
pub async fn download_version_file(
    provider: ProviderId,
    project_id: &str,
    version: &VersionDetail,
    content_type: ContentType,
    file: &VersionFile,
    force: bool,
    child: Option<&GroupedProgressChild>,
    ctx: &ContentCtx,
) -> ContentResult<ArtifactRow> {
    let hash = normalize_hash(&file.sha1);
    if !force && let Some(row) = artifact_dao::get_artifact_by_hash(&ctx.db, &hash).await? {
        let path = artifact_absolute_path(&row.path)?;
        if path.exists() {
            let disk = sha1_file(&path).await?;
            if disk == hash {
                return Ok(row);
            }
        }
    }

    let mut dest = cache_file_path(
        content_type,
        provider,
        project_id,
        &version.version_id,
        &file.file_name,
    )?;

    let refreshed;
    let (version, file) = if file.url.is_empty() && !reo_net::matches_on_disk(&dest, &hash).await
    {
        refreshed = refetch_download(provider, project_id, version, file, &hash, ctx).await?;
        dest = cache_file_path(
            content_type,
            provider,
            project_id,
            &refreshed.0.version_id,
            &refreshed.1.file_name,
        )?;
        (&refreshed.0, &refreshed.1)
    } else {
        (version, file)
    };

    let size = ensure_artifact_file(&hash, &file.url, &dest, child, ctx).await?;
    let stored_path = relative_cache_path(&dest)?;

    let published_at = version.published.to_rfc3339();

    let row = artifact_dao::insert_artifact(
        &ctx.db,
        &hash,
        content_type as i64,
        &stored_path,
        &file.file_name,
        Some(size as i64),
    )
    .await?;

    artifact_dao::upsert_provider_release(
        &ctx.db,
        provider as i64,
        project_id,
        &version.version_id,
        &hash,
        &version.name,
        &version.version_number,
        Some(published_at.as_str()),
        &serde_json::to_string(&version.game_versions)?,
        &serde_json::to_string(&version.loaders)?,
    )
    .await?;

    Ok(row)
}

/// A release rebuilt from the cache keeps no download link so the provider is asked
/// again
/// CurseForge sends none for files whose author turned off third-party downloads
async fn refetch_download(
    provider: ProviderId,
    project_id: &str,
    version: &VersionDetail,
    file: &VersionFile,
    hash: &str,
    ctx: &ContentCtx,
) -> ContentResult<(VersionDetail, VersionFile)> {
    let fresh = ctx
        .providers
        .get(provider)?
        .get_version(project_id, &version.version_id, ctx)
        .await?;

    // Pinned by hash another file of the same version would change what is installed
    let found = fresh
        .files
        .iter()
        .find(|candidate| normalize_hash(&candidate.sha1) == hash)
        .cloned();

    match found {
        Some(found) if !found.url.is_empty() => Ok((fresh, found)),
        Some(_) if provider == ProviderId::CurseForge => Err(PackageError::DownloadBlocked {
            file_name: file.file_name.clone(),
            page_url: format!("https://www.curseforge.com/projects/{project_id}"),
        }
        .into()),
        Some(_) => Err(ContentError::InvalidData {
            reason: format!("{} has no download link on {provider:?}", file.file_name),
        }),
        None => Err(ContentError::InvalidData {
            reason: format!("{} is no longer served by {provider:?}", file.file_name),
        }),
    }
}

#[tracing::instrument(level = "debug", skip(file, child, ctx), fields(name = %file.name))]
pub async fn download_external(
    file: &ExternalFile,
    force: bool,
    child: Option<&GroupedProgressChild>,
    ctx: &ContentCtx,
) -> ContentResult<ArtifactRow> {
    let hash = normalize_hash(&file.sha1);
    if !force && let Some(row) = artifact_dao::get_artifact_by_hash(&ctx.db, &hash).await? {
        let path = artifact_absolute_path(&row.path)?;
        if path.exists() {
            return Ok(row);
        }
    }

    let dest = cache_file_path(
        file.content_type,
        ProviderId::Local,
        "external",
        hash.get(..16).unwrap_or(&hash),
        &file.name,
    )?;
    let size = ensure_artifact_file(&hash, &file.url, &dest, child, ctx).await?;
    let stored_path = relative_cache_path(&dest)?;

    artifact_dao::insert_artifact(
        &ctx.db,
        &hash,
        file.content_type as i64,
        &stored_path,
        &file.name,
        Some(size as i64),
    )
    .await
    .map_err(Into::into)
}
