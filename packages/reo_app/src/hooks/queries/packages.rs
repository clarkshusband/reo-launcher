use std::time::Duration;

use freya::query::{Query, QueryCapability, UseQuery, use_query};
use reo_common::domain::GameLoader;
use reo_common::search::normalize_query;
use reo_content::packages::types::{
    CategoryTag, DEFAULT_PAGE_SIZE, GameSide, Page, ProjectDetail, ProjectSummary, SearchFilters,
    SearchSort, VersionSummary,
};
use reo_content::packages::{CachedPackageMeta, ContentType, ProviderId};
use reo_core::LauncherError;

pub const BROWSE_PAGE_SIZE: usize = DEFAULT_PAGE_SIZE;
/// Multiples of 12 so the grid's 2, 3 and 4 columns always end on a full row. CurseForge caps a
/// page at 50
pub const BROWSE_PAGE_SIZES: [usize; 3] = [12, 24, 48];
pub const VERSIONS_PAGE_SIZE: usize = 20;
pub const ALL_VERSIONS: usize = usize::MAX;

const VERSIONS_STALE: Duration = Duration::from_secs(5 * 60);

pub fn content_type_for_slug(slug: &str) -> ContentType {
    match slug {
        "shader" => ContentType::Shader,
        "texture" => ContentType::ResourcePack,
        "datapack" => ContentType::DataPack,
        "modpack" => ContentType::Modpack,
        _ => ContentType::Mod,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackageSearchQuery;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageSearchKeys {
    pub provider: ProviderId,
    pub content_type: ContentType,
    pub query: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<GameLoader>,
    pub categories: Vec<String>,
    pub sides: Vec<GameSide>,
    pub open_source: bool,
    pub sort: SearchSort,
    pub page: usize,
    pub page_size: usize,
}

impl QueryCapability for PackageSearchQuery {
    type Ok = Page<ProjectSummary>;
    type Err = LauncherError;
    type Keys = PackageSearchKeys;

    #[tracing::instrument(name = "package_search", level = "debug", skip(self, keys), fields(provider = ?keys.provider, page = keys.page))]
    async fn run(&self, keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let state = crate::launcher::state()?;
        let provider = state.services.packages.get(keys.provider)?;
        Ok(provider
            .search(
                &SearchFilters {
                    query: (!keys.query.is_empty()).then(|| keys.query.clone()),
                    content_type: Some(keys.content_type),
                    game_versions: (!keys.game_versions.is_empty())
                        .then(|| keys.game_versions.clone()),
                    loaders: (!keys.loaders.is_empty()).then(|| keys.loaders.clone()),
                    categories: (!keys.categories.is_empty()).then(|| keys.categories.clone()),
                    sides: (!keys.sides.is_empty()).then(|| keys.sides.clone()),
                    open_source: keys.open_source,
                    sort: Some(keys.sort),
                    offset: Some(keys.page * keys.page_size),
                    limit: Some(keys.page_size),
                },
                &state.services.content(),
            )
            .await?)
    }
}

pub fn use_package_search(keys: PackageSearchKeys) -> UseQuery<PackageSearchQuery> {
    use_query(Query::new(
        PackageSearchKeys {
            // The key is also the cache key without normalizing "sodium" and
            // "sodium " are two entries and a trailing space refetches for nothing
            query: normalize_query(&keys.query),
            ..keys
        },
        PackageSearchQuery,
    ))
}

pub fn search_items(query: &UseQuery<PackageSearchQuery>) -> Vec<ProjectSummary> {
    super::state::settled_or_loading(query)
        .map(|page| page.items)
        .unwrap_or_default()
}

pub fn search_total(query: &UseQuery<PackageSearchQuery>) -> usize {
    super::state::settled_or_loading(query)
        .map(|page| page.total)
        .unwrap_or(0)
}

pub fn search_pending(query: &UseQuery<PackageSearchQuery>) -> bool {
    super::state::query_is_busy(query)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackageProjectQuery;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageProjectKeys {
    pub provider: ProviderId,
    pub project_id: String,
}

impl QueryCapability for PackageProjectQuery {
    type Ok = ProjectDetail;
    type Err = LauncherError;
    type Keys = PackageProjectKeys;

    async fn run(&self, keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let state = crate::launcher::state()?;
        let provider = state.services.packages.get(keys.provider)?;
        Ok(provider
            .get_project_with_body(&keys.project_id, &state.services.content())
            .await?)
    }
}

pub fn use_package_project(
    provider: ProviderId,
    project_id: String,
) -> UseQuery<PackageProjectQuery> {
    use_query(Query::new(
        PackageProjectKeys {
            provider,
            project_id,
        },
        PackageProjectQuery,
    ))
}

pub fn project_detail(query: &UseQuery<PackageProjectQuery>) -> Option<ProjectDetail> {
    super::state::settled_or_loading(query)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackageMetaBatchQuery;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageMetaBatchKeys {
    pub provider: ProviderId,
    pub project_ids: Vec<String>,
}

impl QueryCapability for PackageMetaBatchQuery {
    type Ok = std::collections::HashMap<String, CachedPackageMeta>;
    type Err = LauncherError;
    type Keys = PackageMetaBatchKeys;

    #[tracing::instrument(name = "package_meta_batch", level = "debug", skip(self, keys), fields(provider = ?keys.provider, ids = keys.project_ids.len()))]
    async fn run(&self, keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let state = crate::launcher::state()?;
        Ok(reo_content::packages::fetch_package_meta(
            &state.services.content(),
            keys.provider,
            &keys.project_ids,
        )
        .await?)
    }
}

pub fn use_package_meta_batch(
    provider: ProviderId,
    mut project_ids: Vec<String>,
) -> UseQuery<PackageMetaBatchQuery> {
    project_ids.sort();
    project_ids.dedup();
    use_query(Query::new(
        PackageMetaBatchKeys {
            provider,
            project_ids,
        },
        PackageMetaBatchQuery,
    ))
}

pub fn package_meta_batch(
    query: &UseQuery<PackageMetaBatchQuery>,
) -> std::collections::HashMap<String, CachedPackageMeta> {
    super::state::settled_or_loading(query).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackageVersionsQuery;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageVersionsKeys {
    pub provider: ProviderId,
    pub project_id: String,
    pub game_version: Option<String>,
    pub loader: Option<GameLoader>,
    pub page: usize,
    pub page_size: usize,
}

impl QueryCapability for PackageVersionsQuery {
    type Ok = Page<VersionSummary>;
    type Err = LauncherError;
    type Keys = PackageVersionsKeys;

    async fn run(&self, keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let state = crate::launcher::state()?;
        let keys = keys.clone();
        crate::launcher::off_ui(async move {
            Ok(state
                .services
                .packages
                .get(keys.provider)?
                .list_versions(
                    &keys.project_id,
                    keys.game_version.as_deref(),
                    keys.loader,
                    keys.page.saturating_mul(keys.page_size),
                    keys.page_size,
                    &state.services.content(),
                )
                .await?)
        })
        .await
    }
}

pub fn use_package_versions_when(
    enabled: bool,
    provider: ProviderId,
    project_id: String,
    game_version: Option<String>,
    loader: Option<GameLoader>,
    page: usize,
    page_size: usize,
) -> UseQuery<PackageVersionsQuery> {
    let keys = enabled.then_some(PackageVersionsKeys {
        provider,
        project_id,
        game_version,
        loader,
        page,
        page_size,
    });

    use_query(Query::new(keys, PackageVersionsQuery).stale_time(VERSIONS_STALE))
}

pub fn version_list(query: &UseQuery<PackageVersionsQuery>) -> Vec<VersionSummary> {
    super::state::settled_or_loading(query)
        .map(|page| page.items)
        .unwrap_or_default()
}

pub fn versions_total(query: &UseQuery<PackageVersionsQuery>) -> usize {
    super::state::settled_or_loading(query)
        .map(|page| page.total)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackageCategoriesQuery;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackageCategoriesKeys {
    pub provider: ProviderId,
    pub content_type: ContentType,
}

impl QueryCapability for PackageCategoriesQuery {
    type Ok = Vec<CategoryTag>;
    type Err = LauncherError;
    type Keys = PackageCategoriesKeys;

    #[tracing::instrument(name = "package_categories", level = "debug", skip(self, keys), fields(provider = ?keys.provider))]
    async fn run(&self, keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let state = crate::launcher::state()?;
        let provider = state.services.packages.get(keys.provider)?;
        Ok(provider
            .list_categories(keys.content_type, &state.services.content())
            .await?)
    }
}

pub fn use_package_categories(
    provider: ProviderId,
    content_type: ContentType,
) -> UseQuery<PackageCategoriesQuery> {
    use_query(Query::new(
        PackageCategoriesKeys {
            provider,
            content_type,
        },
        PackageCategoriesQuery,
    ))
}

pub fn category_list(query: &UseQuery<PackageCategoriesQuery>) -> Vec<CategoryTag> {
    super::state::settled_or_loading(query).unwrap_or_default()
}
