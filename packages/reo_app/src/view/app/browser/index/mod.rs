use std::time::Duration;

use freya::animation::{
    AnimNum, Ease, Function, OnChange, OnCreation, use_animation_with_dependencies,
};
use freya::prelude::*;
use freya::router::RouterContext;
use reo_common::domain::GameLoader;
use reo_common::parse_mc_version;
use reo_common::search::normalize_query;
use reo_content::packages::types::ProjectSummary;
use reo_content::packages::types::SearchSort;
use reo_content::packages::{ContentType, ProviderId};
use reo_core::VersionMetadata;
use reo_core::clusters::Cluster;
use reo_core::settings::ViewLayout;

use crate::components::{
    Dropdown, Icon, IconType, Pagination, ScrollArea, Segment, SegmentedControl, TextInput,
};
use crate::hooks::use_cluster;
use crate::hooks::{
    BROWSE_PAGE_SIZES, BrowserUiState, PackageSearchKeys, bundle_overrides_map,
    bundles_with_status_items, category_list, cluster_content_items, content_type_for_slug,
    pick_version_metadata, search_items, search_pending, search_total, settled_or_loading,
    use_browser_compat, use_browser_state_store, use_browser_type, use_bundle_overrides,
    use_bundles_with_status, use_cluster_content, use_clusters, use_debounced,
    use_package_categories, use_package_search, use_versions, use_view_state, versions_metadata,
};
use crate::routes::Route;
use crate::theme::colors;
use crate::ui::{border_all_color, grid_columns_for_width};
use crate::view::app::cluster::supports_datapacks;

use super::{
    EnableButton, EnableVariant, InstallSource, Installed, ModpackVersionPrompt, PackageBanner,
    Thumbnail, WorldInstallPrompt, installed_map, minecraft_choices, preferred_version,
};
use crate::utils::{abbreviate_number, sort_clusters_for_home};

mod cards;
mod sidebar;
mod skeletons;
use cards::{empty_state, grid_row, list_row};
use sidebar::{BrowseFilters, FilterSidebar, SIDEBAR_W};
use skeletons::{SkeletonListRow, skeleton_grid_row};

const SCROLLBAR_GUTTER: f32 = 18.;
const PROVIDER_LABELS_W: f32 = 900.;
/// Under this the toolbar has no room for the pager, which then only sits under the results
const TOOLBAR_PAGER_W: f32 = 620.;
const CARD_H: f32 = 226.;
const BANNER_H: f32 = 74.;
const CARD_ICON: f32 = 46.;
/// How far the package icon drops past the banner into the card body
const CARD_ICON_OVERHANG: f32 = 14.;
const MAX_CARD_W: f32 = 300.;
const LIST_ROW_H: f32 = 112.;
const LIST_ICON: f32 = 72.;
const GRID_SPACING: f32 = 16.;
const LIST_SPACING: f32 = 8.;
const SEARCH_DEBOUNCE_MS: u64 = 250;

const SORTS: [(SearchSort, &str); 4] = [
    (SearchSort::Relevance, "Relevance"),
    (SearchSort::Downloads, "Downloads"),
    (SearchSort::Newest, "Newest"),
    (SearchSort::Updated, "Updated"),
];

const BROWSE_TYPES: [(&str, &str); 5] = [
    ("mod", "Mods"),
    ("texture", "Resource Packs"),
    ("datapack", "Data Packs"),
    ("shader", "Shaders"),
    ("modpack", "Modpacks"),
];

const DATAPACK_SLUG: &str = "datapack";
const TEXTURE_SLUG: &str = "texture";
const LOADER_SLUGS: [&str; 2] = ["mod", "shader"];

pub(crate) fn browsable_type(package_type: &str, cluster: &Cluster) -> String {
    if cluster.lacks_mod_loader() {
        let unsupported = LOADER_SLUGS.contains(&package_type)
            || (package_type == DATAPACK_SLUG && !supports_datapacks(&cluster.mc_version));
        return if unsupported {
            TEXTURE_SLUG
        } else {
            package_type
        }
        .to_string();
    }
    if package_type == DATAPACK_SLUG && !supports_datapacks(&cluster.mc_version) {
        BROWSE_TYPES[0].0.to_string()
    } else {
        package_type.to_string()
    }
}

fn type_title(package_type: &str) -> &'static str {
    BROWSE_TYPES
        .iter()
        .find(|(slug, _)| *slug == package_type)
        .map_or("Mods", |(_, title)| *title)
}

fn state_key(cluster_id: i64, package_type: &str) -> String {
    if content_type_for_slug(package_type) == ContentType::Modpack {
        package_type.to_string()
    } else {
        format!("{cluster_id}:{package_type}")
    }
}

pub(crate) fn encode_package_id(provider: ProviderId, id: &str) -> String {
    format!("{}:{}", provider as u8, id)
}

#[derive(PartialEq)]
pub struct Browser {
    pub cluster_id: i64,
    pub package_type: String,
    /// Entered from the navbar so the cluster to install into is picked here
    pub pick_cluster: bool,
}

impl Component for Browser {
    fn render(&self) -> impl IntoElement {
        let mut last_type = use_browser_type();
        use_side_effect_with_deps(&self.package_type, move |package_type| {
            last_type.set_if_modified(package_type.clone());
        });

        let redirect = use_cluster(self.cluster_id)
            .filter(Cluster::lacks_mod_loader)
            .map(|c| browsable_type(&self.package_type, &c))
            .filter(|package_type| *package_type != self.package_type)
            .map(|package_type| Route::Browser {
                cluster_id: self.cluster_id,
                package_type,
                pick_cluster: self.pick_cluster,
            });
        use_side_effect_with_deps(&redirect, move |redirect: &Option<Route>| {
            if let Some(route) = redirect.clone() {
                let _ = RouterContext::get().replace(route);
            }
        });

        BrowserBody {
            cluster_id: self.cluster_id,
            package_type: self.package_type.clone(),
            pick_cluster: self.pick_cluster,
            key: DiffKey::None,
        }
        .key((self.cluster_id, self.package_type.as_str()))
    }
}

#[derive(PartialEq)]
struct BrowserBody {
    cluster_id: i64,
    package_type: String,
    pick_cluster: bool,
    key: DiffKey,
}

impl KeyExt for BrowserBody {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for BrowserBody {
    fn render(&self) -> impl IntoElement {
        let cluster_id = self.cluster_id;
        let package_type = self.package_type.clone();
        let content_type = content_type_for_slug(&package_type);

        let store = use_browser_state_store();
        let state_key = state_key(cluster_id, &package_type);
        let saved = store.peek().get(&state_key).cloned().unwrap_or_default();

        let query = use_state(|| saved.query.clone());
        let provider = use_state(|| saved.provider);
        let view_mode = use_view_state(&format!("browser.{package_type}")).layout;
        let compatible_only = use_browser_compat();
        let filters = BrowseFilters {
            categories: use_state(|| saved.categories.clone()),
            game_versions: use_state(|| saved.game_versions.clone()),
            all_versions: use_state(|| saved.all_versions),
            loaders: use_state(|| saved.loaders.clone()),
            sides: use_state(|| saved.sides.clone()),
            open_source: use_state(|| saved.open_source),
        };
        let sort = use_state(|| saved.sort);
        let page_size = use_state(|| saved.page_size);
        let page = use_state(|| saved.page);

        {
            let mut store = store;
            let key = state_key.clone();
            use_side_effect(move || {
                let snapshot = BrowserUiState {
                    query: query.read().clone(),
                    provider: *provider.read(),
                    categories: filters.categories.read().clone(),
                    sort: *sort.read(),
                    loaders: filters.loaders.read().clone(),
                    game_versions: filters.game_versions.read().clone(),
                    all_versions: *filters.all_versions.read(),
                    sides: filters.sides.read().clone(),
                    open_source: *filters.open_source.read(),
                    page_size: *page_size.read(),
                    page: *page.read(),
                };
                store.write().insert(key.clone(), snapshot);
            });
        }

        // Debounce the search query so typing doesn't fire a request per keystroke
        let debounced_query = use_debounced(
            query.read().clone(),
            Duration::from_millis(SEARCH_DEBOUNCE_MS),
        );

        let cluster = use_cluster(cluster_id);
        let provider_id = *provider.read();
        let compat = *compatible_only.read();
        let cats = filters.categories.read().clone();

        let targets_cluster = content_type != ContentType::Modpack;
        let compat_cluster = cluster.as_ref().filter(|_| compat && targets_cluster);
        let loader_filterable = matches!(content_type, ContentType::Mod | ContentType::Modpack);
        let locked_version = compat_cluster.map(|c| c.mc_version.clone());
        let locked_loader = compat_cluster
            .filter(|_| content_type == ContentType::Mod)
            .map(|c| c.mc_loader);

        let game_versions = match &locked_version {
            Some(version) => vec![version.clone()],
            None => filters.game_versions.read().clone(),
        };
        let loaders = match locked_loader {
            Some(loader) => vec![loader],
            None if loader_filterable => filters.loaders.read().clone(),
            None => Vec::new(),
        };
        let modrinth = provider_id == ProviderId::Modrinth;
        let sides = if modrinth && loader_filterable {
            filters.sides.read().clone()
        } else {
            Vec::new()
        };
        let open_source = modrinth && *filters.open_source.read();
        let size = *page_size.read();
        let sort_by = *sort.read();

        let mut page_state = page;
        // Normalised like the search key so a query differing only by a space does not reset the page
        let signature = format!(
            "{provider_id:?}|{}|{compat}|{}|{sort_by:?}|{game_versions:?}|{loaders:?}|{sides:?}|{open_source}|{size}",
            normalize_query(&debounced_query.read()).to_lowercase(),
            cats.join(",")
        );
        let mut last_signature = use_state(|| signature.clone());
        if *last_signature.peek() != signature {
            last_signature.set(signature);
            if *page_state.peek() != 0 {
                page_state.set(0);
            }
        }

        let search = use_package_search(PackageSearchKeys {
            provider: provider_id,
            content_type,
            query: debounced_query.read().clone(),
            game_versions,
            loaders,
            categories: cats,
            sides,
            open_source,
            sort: sort_by,
            page: *page.read(),
            page_size: size,
        });
        let categories_query = use_package_categories(provider_id, content_type);
        let all_categories = category_list(&categories_query);

        let installed = installed_map(
            cluster_content_items(&use_cluster_content(cluster_id, content_type)),
            &bundles_with_status_items(&use_bundles_with_status(cluster_id)),
            &bundle_overrides_map(&use_bundle_overrides(cluster_id)),
        );
        let installed = if matches!(content_type, ContentType::DataPack | ContentType::Modpack) {
            Default::default()
        } else {
            installed
        };

        let packages = search_items(&search);
        let total = search_total(&search);
        let pending = search_pending(&search);
        let current_page = *page.read();

        // A page switch reports no total until it settles holding the last count keeps the pager in place
        let live_pages = (total > 0).then(|| total.div_ceil(size).max(1));
        let mut settled_pages = use_state(|| live_pages);
        if !pending {
            settled_pages.set_if_modified(live_pages);
        }
        let pages = if pending {
            *settled_pages.read()
        } else {
            live_pages
        };

        let mode = *view_mode.read();

        let mut grid_width = use_state(|| 0f32);
        let top_width = use_state(|| 0f32);
        let toolbar_width = use_state(|| 0f32);
        let cols = grid_columns_for_width(
            (*grid_width.read() - SCROLLBAR_GUTTER).max(0.),
            MAX_CARD_W,
            GRID_SPACING,
        );

        let fade_dep = (current_page, packages.is_empty(), pending, mode);
        let fade = use_animation_with_dependencies(&fade_dep, |conf, _| {
            conf.on_creation(OnCreation::Run);
            conf.on_change(OnChange::Rerun);
            AnimNum::new(0., 1.)
                .time(240)
                .ease(Ease::Out)
                .function(Function::Cubic)
        });
        let fade_opacity = fade.read().value();

        let sa = ScrollArea::new()
            .reset_key(current_page as u64)
            .padding(Gaps::new(0., SCROLLBAR_GUTTER, 0., 0.));

        let pkg = package_type.clone();
        let results = if !packages.is_empty() {
            match mode {
                ViewLayout::Grid => {
                    let rows: Vec<Vec<ProjectSummary>> =
                        packages.chunks(cols).map(|c| c.to_vec()).collect();

                    sa.lazy(rows.len(), CARD_H, GRID_SPACING, move |i| {
                        grid_row(rows[i].clone(), cluster_id, &pkg, &installed, cols).into_element()
                    })
                }
                ViewLayout::List => sa.lazy(packages.len(), LIST_ROW_H, LIST_SPACING, move |i| {
                    list_row(packages[i].clone(), cluster_id, &pkg, &installed).into_element()
                }),
            }
        } else if pending {
            match mode {
                ViewLayout::Grid => {
                    let rows = size.div_ceil(cols);
                    sa.lazy(rows, CARD_H, GRID_SPACING, move |_| {
                        skeleton_grid_row(cols).into_element()
                    })
                }
                ViewLayout::List => sa.lazy(size, LIST_ROW_H, LIST_SPACING, |_| {
                    SkeletonListRow.into_element()
                }),
            }
        } else {
            sa.children([empty_state().into_element()])
        }
        .into_element();

        let target = targets_cluster
            .then(|| {
                let picker = self.pick_cluster.then(|| ClusterPicker {
                    cluster_id,
                    package_type: package_type.clone(),
                });
                install_target(picker, cluster.as_ref().map(|c| c.name.clone()))
            })
            .flatten();

        let measured_toolbar = *toolbar_width.read();
        let pager_in_toolbar = measured_toolbar == 0. || measured_toolbar >= TOOLBAR_PAGER_W;

        rect()
            .vertical()
            .width(Size::fill())
            .height(Size::fill())
            .overflow(Overflow::Clip)
            .padding(Gaps::new(0., 40., 40., 40.))
            .spacing(16.)
            .child(top_bar(
                TypeTabs {
                    cluster_id,
                    package_type: package_type.clone(),
                    pick_cluster: self.pick_cluster,
                },
                provider,
                top_width,
            ))
            .child(
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .content(Content::Flex)
                    .spacing(20.)
                    .child(
                        rect()
                            .width(Size::px(SIDEBAR_W))
                            .height(Size::fill())
                            .child(FilterSidebar {
                                target,
                                content_type,
                                provider: provider_id,
                                categories: all_categories,
                                filters,
                                locked_version,
                                locked_loader,
                            }),
                    )
                    .child(
                        rect()
                            .vertical()
                            .content(Content::Flex)
                            .width(Size::flex(1.0))
                            .height(Size::fill())
                            .spacing(12.)
                            .child(
                                TextInput::new(query)
                                    .width(Size::fill())
                                    .placeholder(format!(
                                        "Search {}...",
                                        type_title(&package_type).to_lowercase()
                                    ))
                                    .leading(
                                        Icon::new(IconType::SearchMd)
                                            .size(14.)
                                            .color(colors::fg_secondary())
                                            .into_element(),
                                    ),
                            )
                            .child(results_toolbar(
                                sort,
                                page_size,
                                view_mode,
                                pager_in_toolbar
                                    .then_some(pages)
                                    .flatten()
                                    .map(|pages| (page, pages, pending)),
                                toolbar_width,
                            ))
                            .child(
                                rect()
                                    .width(Size::fill())
                                    .height(Size::flex(1.0))
                                    .child(results)
                                    .opacity(fade_opacity)
                                    .on_sized(move |event: Event<SizedEventData>| {
                                        let w = event.data().area.width();
                                        if (w - *grid_width.peek()).abs() > 0.5 {
                                            *grid_width.write() = w;
                                        }
                                    }),
                            )
                            .maybe_child(pages.map(|pages| {
                                Pagination::new(page, pages)
                                    .enabled(!pending)
                                    .into_element()
                            })),
                    ),
            )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

fn top_bar(tabs: TypeTabs, provider: State<ProviderId>, mut width: State<f32>) -> impl IntoElement {
    let measured = *width.read();
    let show_provider_labels = measured == 0. || measured >= PROVIDER_LABELS_W;

    rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(12.)
        .content(Content::Flex)
        .on_sized(move |event: Event<SizedEventData>| {
            let w = event.data().area.width();
            if (w - *width.peek()).abs() > 0.5 {
                *width.write() = w;
            }
        })
        .child(tabs)
        .child(rect().width(Size::flex(1.0)))
        .child(
            SegmentedControl::new(provider)
                .no_tint()
                .segments(ProviderId::REMOTE_PROVIDERS.iter().map(move |provider| {
                    let segment = Segment::new(*provider).icon(IconType::from(*provider));
                    if show_provider_labels {
                        segment.label(provider.to_string())
                    } else {
                        segment
                    }
                }))
                .into_element(),
        )
}

/// Sits above the filters so the instance that pins the version and loader is named next to them
fn install_target(picker: Option<ClusterPicker>, cluster_name: Option<String>) -> Option<Element> {
    let control = match picker {
        Some(picker) => picker.into_element(),
        // Without the picker the cluster is fixed so its name is read-only
        None => label()
            .text(cluster_name?)
            .font_size(13.)
            .font_weight(FontWeight::MEDIUM)
            .max_lines(1)
            .width(Size::fill())
            .color(colors::fg_primary())
            .into_element(),
    };

    Some(
        rect()
            .vertical()
            .width(Size::fill())
            .spacing(8.)
            .padding(Gaps::new_all(12.))
            .corner_radius(CornerRadius::new_all(12.))
            .background(colors::component_bg())
            .border(border_all_color(1., colors::component_border()))
            .child(
                label()
                    .text("Installing to")
                    .font_size(11.)
                    .font_weight(FontWeight::MEDIUM)
                    .color(colors::fg_secondary()),
            )
            .child(control)
            .into_element(),
    )
}

fn results_toolbar(
    sort: State<SearchSort>,
    page_size: State<usize>,
    view_mode: State<ViewLayout>,
    pager: Option<(State<usize>, usize, bool)>,
    mut width: State<f32>,
) -> impl IntoElement {
    rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(8.)
        .content(Content::Flex)
        .on_sized(move |event: Event<SizedEventData>| {
            let w = event.data().area.width();
            if (w - *width.peek()).abs() > 0.5 {
                *width.write() = w;
            }
        })
        .child(SortPicker { sort })
        .child(PageSizePicker { page_size })
        .child(
            SegmentedControl::new(view_mode)
                .equal_width(30.)
                .icon_size(15.)
                .segment(Segment::new(ViewLayout::List).icon(IconType::LayoutTop))
                .segment(Segment::new(ViewLayout::Grid).icon(IconType::DotsGrid)),
        )
        .child(rect().width(Size::flex(1.0)))
        .maybe_child(pager.map(|(page, pages, pending)| {
            Pagination::new(page, pages)
                .window(3)
                .enabled(!pending)
                .into_element()
        }))
}

fn toolbar_caption(text: &str) -> Element {
    label()
        .text(text.to_string())
        .font_size(12.)
        .color(colors::fg_secondary())
        .into_element()
}

#[derive(PartialEq)]
struct SortPicker {
    sort: State<SearchSort>,
}

impl Component for SortPicker {
    fn render(&self) -> impl IntoElement {
        let mut sort = self.sort;
        let current = *sort.read();

        let labels: Vec<String> = SORTS.iter().map(|(_, l)| (*l).to_string()).collect();
        let selected = SORTS
            .iter()
            .find(|(s, _)| *s == current)
            .map_or("Relevance", |(_, l)| *l);

        Dropdown::new(selected, labels)
            .width(Size::px(170.))
            .height(Size::px(30.))
            .leading(toolbar_caption("Sort by:"))
            .on_select(move |idx: usize| {
                if let Some((picked, _)) = SORTS.get(idx) {
                    sort.set(*picked);
                }
            })
    }
}

#[derive(PartialEq)]
struct PageSizePicker {
    page_size: State<usize>,
}

impl Component for PageSizePicker {
    fn render(&self) -> impl IntoElement {
        let mut page_size = self.page_size;
        let labels: Vec<String> = BROWSE_PAGE_SIZES.iter().map(usize::to_string).collect();

        Dropdown::new(page_size.read().to_string(), labels)
            .width(Size::px(96.))
            .height(Size::px(30.))
            .leading(toolbar_caption("View:"))
            .on_select(move |idx: usize| {
                if let Some(size) = BROWSE_PAGE_SIZES.get(idx) {
                    page_size.set(*size);
                }
            })
    }
}

/// Falls back to the bare version while the manifest hasn't arrived or doesn't cover it
fn version_name(metadata: &[VersionMetadata], cluster: &Cluster) -> String {
    if cluster.user_created {
        return format!("{} {}", cluster.mc_version, cluster.mc_loader);
    }
    parse_mc_version(&cluster.mc_version)
        .and_then(|parsed| {
            pick_version_metadata(
                metadata,
                parsed.major,
                parsed.key(),
                Some(cluster.mc_loader),
            )
        })
        .map(|m| m.name)
        .unwrap_or_else(|| cluster.mc_version.clone())
}

#[derive(PartialEq)]
struct TypeTabs {
    cluster_id: i64,
    package_type: String,
    pick_cluster: bool,
}

impl Component for TypeTabs {
    fn render(&self) -> impl IntoElement {
        let cluster_id = self.cluster_id;
        let pick_cluster = self.pick_cluster;
        let cluster = use_cluster(cluster_id);
        let datapacks = cluster
            .as_ref()
            .is_none_or(|c| supports_datapacks(&c.mc_version));
        let mod_loader = cluster.as_ref().is_none_or(|c| !c.lacks_mod_loader());

        let tabs = BROWSE_TYPES
            .into_iter()
            .filter(|(slug, _)| datapacks || *slug != DATAPACK_SLUG)
            .filter(|(slug, _)| mod_loader || !LOADER_SLUGS.contains(slug))
            .map(|(slug, title)| {
                let active = slug == self.package_type;
                TypeTab {
                    title,
                    active,
                    on_press: (move |()| {
                        if !active {
                            let _ = RouterContext::get().push(Route::Browser {
                                cluster_id,
                                package_type: slug.to_string(),
                                pick_cluster,
                            });
                        }
                    })
                    .into(),
                    key: DiffKey::None,
                }
                .key(slug)
                .into_element()
            });

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(4.)
            .padding(Gaps::new_all(4.))
            .corner_radius(CornerRadius::new_all(999.))
            .background(colors::component_bg())
            .border(border_all_color(1., colors::component_border()))
            .children(tabs)
    }
}

#[derive(PartialEq)]
struct TypeTab {
    title: &'static str,
    active: bool,
    on_press: EventHandler<()>,
    key: DiffKey,
}

impl KeyExt for TypeTab {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for TypeTab {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let on_press = self.on_press.clone();
        let active = self.active;
        let hovered = *hovering.read();

        rect()
            .height(Size::px(30.))
            .center()
            .padding(Gaps::new_symmetric(0., 14.))
            .corner_radius(CornerRadius::new_all(999.))
            .maybe(active, |el| el.background(colors::brand()))
            .maybe(!active && hovered, |el| {
                el.background(colors::ghost_overlay())
            })
            .a11y_role(AccessibilityRole::Tab)
            .a11y_focusable(true)
            .cursor(CursorIcon::Pointer)
            .on_pointer_enter(move |_| hovering.set(true))
            .on_pointer_leave(move |_| hovering.set(false))
            .on_press(move |_| on_press.call(()))
            .child(
                label()
                    .text(self.title)
                    .font_size(13.)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .max_lines(1)
                    .color(if active {
                        Color::WHITE
                    } else if hovered {
                        colors::fg_primary()
                    } else {
                        colors::fg_secondary()
                    }),
            )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The choice lives in the route so back and forward keep the cluster being browsed
#[derive(PartialEq)]
struct ClusterPicker {
    cluster_id: i64,
    package_type: String,
}

impl Component for ClusterPicker {
    fn render(&self) -> impl IntoElement {
        let clusters =
            sort_clusters_for_home(settled_or_loading(&use_clusters()).unwrap_or_default());
        let metadata = versions_metadata(&use_versions()).unwrap_or_default();

        let labels: Vec<String> = clusters
            .iter()
            .map(|c| format!("{} · {}", c.name, version_name(&metadata, c)))
            .collect();
        let ids: Vec<i64> = clusters.iter().map(|c| c.id).collect();

        let selected = ids
            .iter()
            .position(|id| *id == self.cluster_id)
            .and_then(|idx| labels.get(idx).cloned())
            .unwrap_or_else(|| "Select a version".to_string());

        let package_type = self.package_type.clone();
        let from_key = state_key(self.cluster_id, &package_type);
        let mut store = use_browser_state_store();

        Dropdown::new(selected, labels)
            .width(Size::fill())
            .height(Size::px(30.))
            .on_select(move |idx: usize| {
                if let (Some(cluster_id), Some(cluster)) =
                    (ids.get(idx).copied(), clusters.get(idx))
                {
                    let to_type = browsable_type(&package_type, cluster);
                    let carried = (to_type == package_type)
                        .then(|| store.peek().get(&from_key).cloned())
                        .flatten();
                    if let Some(saved) = carried {
                        store.write().insert(
                            state_key(cluster_id, &to_type),
                            BrowserUiState { page: 0, ..saved },
                        );
                    }
                    let _ = RouterContext::get().replace(Route::Browser {
                        cluster_id,
                        package_type: to_type,
                        pick_cluster: true,
                    });
                }
            })
    }
}
