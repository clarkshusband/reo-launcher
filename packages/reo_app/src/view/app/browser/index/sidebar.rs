use bytes::Bytes;
use freya::animation::{AnimNum, Ease, Function, use_animation_transition};
use freya::prelude::*;
use reo_common::domain::GameLoader;
use reo_content::packages::types::{CategoryTag, GameSide};
use reo_content::packages::{ContentType, ProviderId};
use reo_core::GameVersionKind;

use crate::components::{AssetImage, Icon, IconType, ScrollArea, TextInput, checkbox_labeled};
use crate::hooks::{game_versions, use_game_versions};
use crate::theme::colors;
use crate::ui::border_all_color;
use crate::utils::capitalize;

pub(super) const SIDEBAR_W: f32 = 240.;
const OPTION_H: f32 = 30.;
const OPTION_ICON: f32 = 15.;
const MARK_SIZE: f32 = 18.;
const VERSION_CHIP_H: f32 = 30.;
const VERSION_CHIP_SPACING: f32 = 6.;
/// Rows of version chips shown before the grid scrolls
const VERSION_ROWS: usize = 6;
/// Releases are short enough for three a row; snapshot ids like `1.21.11-pre3` need two
const VERSION_COLUMNS: usize = 3;
const VERSION_COLUMNS_ALL: usize = 2;
/// Clears the grid's scrollbar so it never sits over a chip
const VERSION_SCROLL_GUTTER: f32 = 10.;
const SECTION_REVEAL_MS: u64 = 220;
/// Quilt's brand purple, the same as its loader chip
pub(super) const QUILT_COLOR: Color = Color::from_rgb(199, 150, 249);

/// Loaders most people pick, the rest wait behind "Show more"
const MAIN_LOADERS: [GameLoader; 3] = [GameLoader::Fabric, GameLoader::Forge, GameLoader::NeoForge];
const MORE_LOADERS: [GameLoader; 1] = [GameLoader::Quilt];

const SIDES: [(GameSide, &str, IconType); 2] = [
    (GameSide::Client, "Client", IconType::Monitor01),
    (GameSide::Server, "Server", IconType::Server01),
];

/// What the browse page filters by, read by the search and written by the sidebar
#[derive(Clone, Copy, PartialEq)]
pub(super) struct BrowseFilters {
    pub(super) categories: State<Vec<String>>,
    pub(super) game_versions: State<Vec<String>>,
    pub(super) all_versions: State<bool>,
    pub(super) loaders: State<Vec<GameLoader>>,
    pub(super) sides: State<Vec<GameSide>>,
    pub(super) open_source: State<bool>,
}

#[derive(PartialEq)]
pub(super) struct FilterSidebar {
    /// Where installs go, shown above the filters when the page targets an instance
    pub(super) target: Option<Element>,
    pub(super) content_type: ContentType,
    pub(super) provider: ProviderId,
    pub(super) categories: Vec<CategoryTag>,
    pub(super) filters: BrowseFilters,
    /// Pinned by the instance being browsed for, so they show as chosen but cannot change
    pub(super) locked_version: Option<String>,
    pub(super) locked_loader: Option<GameLoader>,
}

impl Component for FilterSidebar {
    fn render(&self) -> impl IntoElement {
        let filters = self.filters;
        let loader_filter = matches!(self.content_type, ContentType::Mod | ContentType::Modpack);
        let modrinth = self.provider == ProviderId::Modrinth;

        let mut sections: Vec<Element> = Vec::new();
        sections.extend(self.target.clone());
        sections.push(
            VersionFilter {
                selected: filters.game_versions,
                all_versions: filters.all_versions,
                locked: self.locked_version.clone(),
            }
            .into_element(),
        );
        if loader_filter {
            sections.push(
                LoaderFilter {
                    selected: filters.loaders,
                    locked: self.locked_loader,
                }
                .into_element(),
            );
        }
        for (header, tags) in group_by_header(&self.categories) {
            sections.push(
                CategoryFilter {
                    title: header_title(&header),
                    tags,
                    selected: filters.categories,
                    key: DiffKey::None,
                }
                .key(header)
                .into_element(),
            );
        }
        // Only Modrinth reports which side a project runs on or whether it is open source
        if modrinth && loader_filter {
            sections.push(
                EnvironmentFilter {
                    selected: filters.sides,
                }
                .into_element(),
            );
        }
        if modrinth {
            sections.push(
                LicenseFilter {
                    open_source: filters.open_source,
                }
                .into_element(),
            );
        }

        ScrollArea::new()
            .width(Size::fill())
            .height(Size::fill())
            .spacing(10.)
            .children(sections)
    }
}

/// Keeps the provider's order of groups so "Categories" leads and features or resolutions follow
pub(super) fn group_by_header(tags: &[CategoryTag]) -> Vec<(String, Vec<CategoryTag>)> {
    let mut groups: Vec<(String, Vec<CategoryTag>)> = Vec::new();
    for tag in tags {
        match groups.iter_mut().find(|(header, _)| *header == tag.header) {
            Some((_, group)) => group.push(tag.clone()),
            None => groups.push((tag.header.clone(), vec![tag.clone()])),
        }
    }
    groups
}

pub(super) fn header_title(header: &str) -> String {
    match header {
        "" | "categories" => "Category".to_string(),
        other => capitalize(other),
    }
}

/// Modrinth names are slugs such as `game-mechanics`; CurseForge names are already titles
pub(super) fn category_label(name: &str) -> String {
    match name {
        "worldgen" => "World Generation".to_string(),
        "pbr" => "PBR".to_string(),
        _ => name
            .split(['-', '_'])
            .filter(|word| !word.is_empty())
            .map(capitalize)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub(super) fn loader_label(loader: GameLoader) -> &'static str {
    match loader {
        GameLoader::Fabric => "Fabric",
        GameLoader::Forge => "Forge",
        GameLoader::NeoForge => "NeoForge",
        GameLoader::Quilt => "Quilt",
        GameLoader::Ornithe => "Ornithe",
        GameLoader::Vanilla => "Vanilla",
    }
}

pub(super) fn loader_mark(loader: GameLoader, size: f32) -> Option<Element> {
    let path = match loader {
        GameLoader::Fabric => "icons/fabric.png",
        GameLoader::Forge => "icons/forge.png",
        GameLoader::NeoForge => "icons/neo-forge.png",
        GameLoader::Quilt => {
            return Some(
                Icon::new(IconType::Quilt)
                    .size(size)
                    .color(QUILT_COLOR)
                    .into_element(),
            );
        }
        _ => return None,
    };
    Some(AssetImage::new(path, size).into_element())
}

fn toggle_in<T: Clone + PartialEq + 'static>(mut state: State<Vec<T>>, value: T) {
    let mut current: Vec<T> = state.peek().to_vec();
    match current.iter().position(|v| *v == value) {
        Some(i) => {
            current.remove(i);
        }
        None => current.push(value),
    }
    state.set(current);
}

fn icon_leading(icon: IconType, selected: bool) -> Element {
    Icon::new(icon)
        .size(OPTION_ICON)
        .color(if selected {
            colors::fg_primary()
        } else {
            colors::fg_secondary()
        })
        .into_element()
}

/// Inline Modrinth SVGs draw in `currentColor`, so the tint follows the row's state
fn svg_leading(svg: &str, selected: bool) -> Element {
    SvgViewer::new(Bytes::from(svg.to_owned()))
        .show_loader(false)
        .width(Size::px(OPTION_ICON))
        .height(Size::px(OPTION_ICON))
        .color(if selected {
            colors::fg_primary()
        } else {
            colors::fg_secondary()
        })
        .into_element()
}

#[derive(PartialEq)]
struct FilterSection {
    title: String,
    start_open: bool,
    hint: Option<String>,
    children: Vec<Element>,
    /// Labels of what the user picked here, counted in the header and listed while folded
    picked: Vec<String>,
    on_clear: Option<EventHandler<()>>,
}

impl FilterSection {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            start_open: true,
            hint: None,
            children: Vec::new(),
            picked: Vec::new(),
            on_clear: None,
        }
    }

    fn closed(mut self) -> Self {
        self.start_open = false;
        self
    }

    fn picked(mut self, picked: Vec<String>, on_clear: impl FnMut(()) + 'static) -> Self {
        self.picked = picked;
        self.on_clear = Some(on_clear.into());
        self
    }

    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_element());
        self
    }

    fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
        self.children.extend(children);
        self
    }
}

impl Component for FilterSection {
    fn render(&self) -> impl IntoElement {
        let mut open = use_state(|| self.start_open);
        let mut hovering = use_state(|| false);
        let is_open = *open.read();

        let reveal = use_animation_transition(if is_open { 1f32 } else { 0. }, |from, to| {
            AnimNum::new(from, to)
                .time(SECTION_REVEAL_MS)
                .ease(Ease::Out)
                .function(Function::Cubic)
        });
        let t = reveal.get().value();

        let count = self.picked.len();
        let clear = self.on_clear.clone().filter(|_| count > 0);

        rect()
            .vertical()
            .width(Size::fill())
            .padding(Gaps::new_all(12.))
            .spacing(10.)
            .corner_radius(CornerRadius::new_all(12.))
            .background(colors::component_bg())
            .border(border_all_color(
                1.,
                if count > 0 {
                    colors::brand().with_a(90)
                } else {
                    colors::component_border()
                },
            ))
            .child(
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .cross_align(Alignment::Center)
                    .content(Content::Flex)
                    .spacing(8.)
                    .a11y_role(AccessibilityRole::Button)
                    .a11y_focusable(true)
                    .cursor(CursorIcon::Pointer)
                    .on_pointer_enter(move |_| hovering.set(true))
                    .on_pointer_leave(move |_| hovering.set(false))
                    .on_press(move |_| open.toggle())
                    .child(
                        label()
                            .text(self.title.clone())
                            .font_size(14.)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .max_lines(1)
                            .color(colors::fg_primary()),
                    )
                    .maybe_child((count > 0).then(|| count_badge(count)))
                    .child(rect().width(Size::flex(1.0)))
                    .maybe_child(clear.map(|on_clear| ClearButton { on_clear }.into_element()))
                    .child(
                        Icon::new(IconType::ChevronDown)
                            .size(16.)
                            .rotate(t * 180.)
                            .color(if *hovering.read() {
                                colors::fg_primary()
                            } else {
                                colors::fg_secondary()
                            }),
                    ),
            )
            .maybe(is_open, |el| {
                el.child(
                    rect()
                        .vertical()
                        .width(Size::fill())
                        .spacing(10.)
                        .opacity(t)
                        .children(self.children.clone())
                        .maybe_child(self.hint.clone().map(|hint| {
                            label()
                                .text(hint)
                                .font_size(11.)
                                .width(Size::fill())
                                .color(colors::fg_secondary())
                                .into_element()
                        })),
                )
            })
            // Folded away the picks would be invisible, so they ride along under the title
            .maybe(!is_open && count > 0, |el| {
                el.child(
                    rect()
                        .horizontal()
                        .width(Size::fill())
                        .content(Content::wrap_spacing(6.))
                        .spacing(6.)
                        .children(self.picked.iter().map(|picked| summary_chip(picked))),
                )
            })
    }
}

fn count_badge(count: usize) -> Element {
    rect()
        .height(Size::px(18.))
        .min_width(Size::px(18.))
        .center()
        .padding(Gaps::new_symmetric(0., 6.))
        .corner_radius(CornerRadius::new_all(999.))
        .background(colors::brand())
        .child(
            label()
                .text(count.to_string())
                .font_size(11.)
                .font_weight(FontWeight::BOLD)
                .color(Color::WHITE),
        )
        .into_element()
}

fn summary_chip(text: &str) -> Element {
    rect()
        .height(Size::px(22.))
        .center()
        .padding(Gaps::new_symmetric(0., 8.))
        .corner_radius(CornerRadius::new_all(999.))
        .background(colors::brand().with_a(40))
        .border(border_all_color(1., colors::brand().with_a(110)))
        .child(
            label()
                .text(text.to_string())
                .font_size(11.)
                .font_weight(FontWeight::MEDIUM)
                .max_lines(1)
                .color(colors::fg_primary()),
        )
        .into_element()
}

#[derive(PartialEq)]
struct ClearButton {
    on_clear: EventHandler<()>,
}

impl Component for ClearButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let on_clear = self.on_clear.clone();

        label()
            .text("Clear")
            .font_size(12.)
            .font_weight(FontWeight::MEDIUM)
            .color(if *hovering.read() {
                colors::fg_primary()
            } else {
                colors::fg_secondary()
            })
            .a11y_role(AccessibilityRole::Button)
            .cursor(CursorIcon::Pointer)
            .on_pointer_enter(move |_| hovering.set(true))
            .on_pointer_leave(move |_| hovering.set(false))
            .on_press(move |e: Event<PressEventData>| {
                // The header around it folds the section on press
                e.stop_propagation();
                on_clear.call(());
            })
    }
}

/// A filled ring with a tick once picked, an empty ring on hover, nothing at rest
fn selection_mark(selected: bool, hovered: bool, locked: bool) -> Option<Element> {
    let ring = rect()
        .width(Size::px(MARK_SIZE))
        .height(Size::px(MARK_SIZE))
        .center()
        .corner_radius(CornerRadius::new_all(999.));
    if selected {
        Some(
            ring.background(if locked {
                colors::component_border_hover()
            } else {
                colors::brand()
            })
            .child(Icon::new(IconType::Check).size(12.).color(Color::WHITE))
            .into_element(),
        )
    } else if hovered {
        Some(
            ring.border(border_all_color(1.5, colors::component_border_hover()))
                .into_element(),
        )
    } else {
        None
    }
}

#[derive(PartialEq)]
struct FilterOption {
    label: String,
    leading: Option<Element>,
    selected: bool,
    locked: bool,
    on_toggle: Option<EventHandler<()>>,
    key: DiffKey,
}

impl FilterOption {
    fn new(label: impl Into<String>, selected: bool, on_toggle: impl FnMut(()) + 'static) -> Self {
        let on_toggle: EventHandler<()> = on_toggle.into();
        Self {
            label: label.into(),
            leading: None,
            selected,
            locked: false,
            on_toggle: Some(on_toggle),
            key: DiffKey::None,
        }
    }

    fn locked(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            leading: None,
            selected: true,
            locked: true,
            on_toggle: None,
            key: DiffKey::None,
        }
    }

    fn leading(mut self, leading: Option<Element>) -> Self {
        self.leading = leading;
        self
    }
}

impl KeyExt for FilterOption {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for FilterOption {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let a11y_id = use_a11y();
        let focus = use_focus(a11y_id);

        let selected = self.selected;
        let interactive = !self.locked && self.on_toggle.is_some();
        let hovered = interactive && *hovering.read();
        let focused = focus().is_focused();
        let color = if selected || hovered || focused {
            colors::fg_primary()
        } else {
            colors::fg_secondary()
        };
        let on_toggle = self.on_toggle.clone();

        rect()
            .horizontal()
            .width(Size::fill())
            .height(Size::px(OPTION_H))
            .cross_align(Alignment::Center)
            .content(Content::Flex)
            .spacing(8.)
            .padding(Gaps::new_symmetric(0., 8.))
            .corner_radius(CornerRadius::new_all(8.))
            .maybe(selected, |el| el.background(colors::brand().with_a(40)))
            .maybe(!selected && hovered, |el| {
                el.background(colors::ghost_overlay())
            })
            .opacity(if self.locked { 0.75 } else { 1. })
            .a11y_id(a11y_id)
            .a11y_focusable(interactive)
            .a11y_role(AccessibilityRole::Button)
            .maybe(interactive, |el| {
                el.cursor(CursorIcon::Pointer)
                    .on_pointer_enter(move |_| hovering.set(true))
                    .on_pointer_leave(move |_| hovering.set(false))
                    .on_press(move |_| {
                        if let Some(on_toggle) = &on_toggle {
                            on_toggle.call(());
                        }
                    })
            })
            .maybe_child(self.leading.clone())
            .child(
                label()
                    .text(self.label.clone())
                    .font_size(13.)
                    .max_lines(1)
                    .width(Size::flex(1.0))
                    .font_weight(if selected {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .color(color),
            )
            .maybe_child(selection_mark(selected, hovered, self.locked))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(PartialEq)]
struct VersionFilter {
    selected: State<Vec<String>>,
    all_versions: State<bool>,
    locked: Option<String>,
}

impl Component for VersionFilter {
    fn render(&self) -> impl IntoElement {
        let versions_query = use_game_versions();
        let search = use_state(String::new);
        let selected = self.selected;

        if let Some(version) = self.locked.clone() {
            return FilterSection::new("Game version")
                .hint("Matches the instance you are installing to.")
                .child(FilterOption::locked(version))
                .into_element();
        }

        let show_all = *self.all_versions.read();
        let needle = search.read().trim().to_lowercase();
        let picked = selected.read().clone();
        let shown: Vec<String> = game_versions(&versions_query)
            .unwrap_or_default()
            .iter()
            .filter(|v| {
                picked.contains(&v.id)
                    || ((show_all || v.kind == GameVersionKind::Release)
                        && (needle.is_empty() || v.id.to_lowercase().contains(&needle)))
            })
            .map(|v| v.id.clone())
            .collect();

        let columns = if show_all {
            VERSION_COLUMNS_ALL
        } else {
            VERSION_COLUMNS
        };
        let grid = if shown.is_empty() {
            label()
                .text("No versions match.")
                .font_size(12.)
                .color(colors::fg_secondary())
                .into_element()
        } else {
            version_grid(shown, columns, selected)
        };

        let mut cleared = selected;
        FilterSection::new("Game version")
            .picked(picked, move |()| cleared.set(Vec::new()))
            .child(
                TextInput::new(search)
                    .width(Size::fill())
                    .compact()
                    .placeholder("Search...")
                    .leading(
                        Icon::new(IconType::SearchMd)
                            .size(13.)
                            .color(colors::fg_secondary())
                            .into_element(),
                    ),
            )
            .child(grid)
            .child(checkbox_labeled(self.all_versions, "Show all versions"))
            .into_element()
    }
}

fn version_grid(versions: Vec<String>, columns: usize, selected: State<Vec<String>>) -> Element {
    let rows: Vec<Vec<String>> = versions.chunks(columns).map(<[String]>::to_vec).collect();
    let row_count = rows.len();
    let visible = row_count.min(VERSION_ROWS) as f32;
    let height = visible * VERSION_CHIP_H + (visible - 1.).max(0.) * VERSION_CHIP_SPACING;
    let scrolls = row_count > VERSION_ROWS;

    rect()
        .width(Size::fill())
        .height(Size::px(height))
        .child(
            ScrollArea::new()
                .width(Size::fill())
                .height(Size::fill())
                .padding(Gaps::new(
                    0.,
                    if scrolls { VERSION_SCROLL_GUTTER } else { 0. },
                    0.,
                    0.,
                ))
                .lazy(row_count, VERSION_CHIP_H, VERSION_CHIP_SPACING, move |i| {
                    let row = rows[i].clone();
                    // Spacers keep the last row's chips as wide as the ones above
                    let spacers = columns.saturating_sub(row.len());
                    rect()
                        .horizontal()
                        .width(Size::fill())
                        .height(Size::px(VERSION_CHIP_H))
                        .spacing(VERSION_CHIP_SPACING)
                        .content(Content::Flex)
                        .children(row.into_iter().map(|version| {
                            let is_selected = selected.read().contains(&version);
                            let toggled = version.clone();
                            VersionChip {
                                version: version.clone(),
                                selected: is_selected,
                                on_toggle: (move |()| toggle_in(selected, toggled.clone())).into(),
                                key: DiffKey::None,
                            }
                            .key(version)
                            .into_element()
                        }))
                        .children(
                            (0..spacers).map(|_| rect().width(Size::flex(1.0)).into_element()),
                        )
                        .into_element()
                }),
        )
        .into_element()
}

#[derive(PartialEq)]
struct VersionChip {
    version: String,
    selected: bool,
    on_toggle: EventHandler<()>,
    key: DiffKey,
}

impl KeyExt for VersionChip {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for VersionChip {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let on_toggle = self.on_toggle.clone();
        let selected = self.selected;
        let hovered = *hovering.read();

        let (background, border, text) = if selected {
            (colors::brand(), colors::brand(), Color::WHITE)
        } else if hovered {
            (
                colors::component_bg_hover(),
                colors::component_border_hover(),
                colors::fg_primary(),
            )
        } else {
            (
                colors::ghost_overlay(),
                colors::component_border(),
                colors::fg_secondary(),
            )
        };

        rect()
            .width(Size::flex(1.0))
            .height(Size::px(VERSION_CHIP_H))
            .center()
            .padding(Gaps::new_symmetric(0., 4.))
            .corner_radius(CornerRadius::new_all(8.))
            .background(background)
            .border(border_all_color(1., border))
            .a11y_role(AccessibilityRole::Button)
            .a11y_focusable(true)
            .cursor(CursorIcon::Pointer)
            .on_pointer_enter(move |_| hovering.set(true))
            .on_pointer_leave(move |_| hovering.set(false))
            .on_press(move |_| on_toggle.call(()))
            .child(
                label()
                    .text(self.version.clone())
                    .font_size(12.)
                    .font_weight(if selected {
                        FontWeight::SEMI_BOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .max_lines(1)
                    .color(text),
            )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(PartialEq)]
struct LoaderFilter {
    selected: State<Vec<GameLoader>>,
    locked: Option<GameLoader>,
}

impl Component for LoaderFilter {
    fn render(&self) -> impl IntoElement {
        let mut more = use_state(|| false);
        let selected = self.selected;

        if let Some(loader) = self.locked {
            return FilterSection::new("Loader")
                .hint("Matches the instance you are installing to.")
                .child(
                    FilterOption::locked(loader_label(loader))
                        .leading(loader_mark(loader, OPTION_ICON)),
                )
                .into_element();
        }

        let picked = selected.read().clone();
        // A pick from the hidden group keeps it open, a chosen filter never hides
        let extra_picked = picked.iter().any(|l| MORE_LOADERS.contains(l));
        let show_more = *more.read() || extra_picked;

        let options: Vec<Element> = MAIN_LOADERS
            .iter()
            .chain(MORE_LOADERS.iter().filter(|_| show_more))
            .map(|loader| {
                let loader = *loader;
                FilterOption::new(loader_label(loader), picked.contains(&loader), move |()| {
                    toggle_in(selected, loader);
                })
                .leading(
                    loader_mark(loader, OPTION_ICON)
                        .or_else(|| Some(rect().width(Size::px(OPTION_ICON)).into_element())),
                )
                .key(loader_label(loader))
                .into_element()
            })
            .collect();

        let picked_labels = picked
            .iter()
            .map(|l| loader_label(*l).to_string())
            .collect();
        let mut cleared = selected;
        let section = FilterSection::new("Loader")
            .picked(picked_labels, move |()| cleared.set(Vec::new()))
            .children(options);
        if extra_picked {
            return section.into_element();
        }
        section
            .child(ShowMore {
                open: show_more,
                on_press: (move |()| more.toggle()).into(),
            })
            .into_element()
    }
}

#[derive(PartialEq)]
struct ShowMore {
    open: bool,
    on_press: EventHandler<()>,
}

impl Component for ShowMore {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let on_press = self.on_press.clone();
        let color = if *hovering.read() {
            colors::fg_primary()
        } else {
            colors::fg_secondary()
        };

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(6.)
            .padding(Gaps::new_symmetric(2., 8.))
            .a11y_role(AccessibilityRole::Button)
            .a11y_focusable(true)
            .cursor(CursorIcon::Pointer)
            .on_pointer_enter(move |_| hovering.set(true))
            .on_pointer_leave(move |_| hovering.set(false))
            .on_press(move |_| on_press.call(()))
            .child(
                Icon::new(IconType::ChevronDown)
                    .size(14.)
                    .rotate(if self.open { 180. } else { 0. })
                    .color(color),
            )
            .child(
                label()
                    .text(if self.open { "Show less" } else { "Show more" })
                    .font_size(12.)
                    .font_weight(FontWeight::MEDIUM)
                    .color(color),
            )
    }
}

#[derive(PartialEq)]
struct CategoryFilter {
    title: String,
    tags: Vec<CategoryTag>,
    selected: State<Vec<String>>,
    key: DiffKey,
}

impl KeyExt for CategoryFilter {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for CategoryFilter {
    fn render(&self) -> impl IntoElement {
        let selected = self.selected;
        let picked = selected.read().clone();

        let options: Vec<Element> = self
            .tags
            .iter()
            .map(|tag| {
                let is_selected = picked.contains(&tag.name);
                let name = tag.name.clone();
                FilterOption::new(category_label(&tag.name), is_selected, move |()| {
                    toggle_in(selected, name.clone());
                })
                .leading(
                    tag.icon_svg
                        .as_deref()
                        .map(|svg| svg_leading(svg, is_selected)),
                )
                .key(tag.name.clone())
                .into_element()
            })
            .collect();

        // Clearing a group drops only its own tags, picks in the other groups stay
        let group: Vec<String> = self.tags.iter().map(|t| t.name.clone()).collect();
        let picked_labels = self
            .tags
            .iter()
            .filter(|t| picked.contains(&t.name))
            .map(|t| category_label(&t.name))
            .collect();
        let mut cleared = selected;
        FilterSection::new(self.title.clone())
            .picked(picked_labels, move |()| {
                let kept: Vec<String> = cleared
                    .peek()
                    .iter()
                    .filter(|name| !group.contains(name))
                    .cloned()
                    .collect();
                cleared.set(kept);
            })
            .children(options)
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(PartialEq)]
struct EnvironmentFilter {
    selected: State<Vec<GameSide>>,
}

impl Component for EnvironmentFilter {
    fn render(&self) -> impl IntoElement {
        let selected = self.selected;
        let picked = selected.read().clone();
        let picked_labels = SIDES
            .iter()
            .filter(|(side, _, _)| picked.contains(side))
            .map(|(_, title, _)| (*title).to_string())
            .collect();
        let mut cleared = selected;

        FilterSection::new("Environment")
            .picked(picked_labels, move |()| cleared.set(Vec::new()))
            .children(SIDES.iter().map(|(side, title, icon)| {
                let side = *side;
                let is_selected = picked.contains(&side);
                FilterOption::new(*title, is_selected, move |()| toggle_in(selected, side))
                    .leading(Some(icon_leading(*icon, is_selected)))
                    .key(*title)
                    .into_element()
            }))
    }
}

#[derive(PartialEq)]
struct LicenseFilter {
    open_source: State<bool>,
}

impl Component for LicenseFilter {
    fn render(&self) -> impl IntoElement {
        let mut open_source = self.open_source;
        let on = *open_source.read();
        let mut cleared = open_source;

        FilterSection::new("License")
            .closed()
            .picked(
                if on {
                    vec!["Open source".to_string()]
                } else {
                    Vec::new()
                },
                move |()| cleared.set(false),
            )
            .child(
                FilterOption::new("Open source only", on, move |()| open_source.toggle())
                    .leading(Some(icon_leading(IconType::CodeSnippet02, on))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, header: &str) -> CategoryTag {
        CategoryTag {
            name: name.to_string(),
            header: header.to_string(),
            icon_svg: None,
        }
    }

    #[test]
    fn category_slugs_read_as_titles() {
        assert_eq!(category_label("game-mechanics"), "Game Mechanics");
        assert_eq!(category_label("worldgen"), "World Generation");
        assert_eq!(category_label("adventure"), "Adventure");
        assert_eq!(category_label("16x"), "16x");
        assert_eq!(
            category_label("Armor, Tools, and Weapons"),
            "Armor, Tools, and Weapons"
        );
    }

    #[test]
    fn groups_keep_the_provider_order_and_name_the_main_one_category() {
        let groups = group_by_header(&[
            tag("adventure", "categories"),
            tag("pbr", "features"),
            tag("magic", "categories"),
            tag("16x", "resolutions"),
        ]);
        let headers: Vec<&str> = groups.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(headers, ["categories", "features", "resolutions"]);
        assert_eq!(groups[0].1.len(), 2);

        assert_eq!(header_title("categories"), "Category");
        assert_eq!(header_title(""), "Category");
        assert_eq!(header_title("performance impact"), "Performance impact");
    }
}
