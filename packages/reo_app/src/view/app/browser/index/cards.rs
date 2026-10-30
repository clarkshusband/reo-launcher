use super::*;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use freya::router::RouterContext;
use reo_content::packages::types::{ProjectSummary, SideSupport};
use reo_content::packages::{ContentType, ProviderId};
use reo_core::clusters::ModpackSource;

use crate::components::{Button, Icon, IconType};
use crate::hooks::{
    ALL_VERSIONS, ClusterAction, VERSIONS_PAGE_SIZE, content_type_for_slug, use_browser_compat,
    use_cluster, use_dispatch, use_installs_snapshot, use_package_versions_when, version_list,
};
use crate::routes::Route;
use crate::theme::colors;
use crate::ui::border_all_color;

type InstalledMap = HashMap<(ProviderId, String), Installed>;

/// Height of the install control, matched by the installed pill that replaces it
const INSTALL_BUTTON_H: f32 = 28.;

#[derive(Clone, PartialEq)]
struct CardInstalled {
    source: InstallSource,
    enable: Option<ClusterAction>,
}

fn installed_for(
    installed: &InstalledMap,
    item: &ProjectSummary,
    cluster_id: i64,
) -> Option<CardInstalled> {
    installed
        .get(&(item.provider, item.id.clone()))
        .map(|installed| CardInstalled {
            source: installed.source,
            enable: installed
                .disabled_bundled()
                .and_then(|version| version.enable_action(cluster_id)),
        })
}

pub(super) fn grid_row(
    row: Vec<ProjectSummary>,
    cluster_id: i64,
    package_type: &str,
    installed: &InstalledMap,
    cols: usize,
) -> impl IntoElement {
    let package_type = package_type.to_string();
    let fill = cols.saturating_sub(row.len());
    let installed: Vec<Option<CardInstalled>> = row
        .iter()
        .map(|item| installed_for(installed, item, cluster_id))
        .collect();

    rect()
        .horizontal()
        .width(Size::fill())
        .height(Size::px(CARD_H))
        .spacing(GRID_SPACING)
        .content(Content::Flex)
        .children(
            row.into_iter()
                .zip(installed)
                .map(move |(item, installed)| {
                    PackageCard::new(item, cluster_id, package_type.clone(), installed)
                        .into_element()
                }),
        )
        .maybe(fill > 0, move |mut el| {
            for _ in 0..fill {
                el = el.child(rect().width(Size::flex(1.0)));
            }
            el
        })
        .into_element()
}

fn open_package(cluster_id: i64, package_type: &str, provider: ProviderId, id: &str) {
    let _ = RouterContext::get().push(Route::BrowserPackage {
        cluster_id,
        package_type: package_type.to_string(),
        package_id: encode_package_id(provider, id),
    });
}

#[derive(PartialEq)]
struct PackageCard {
    item: ProjectSummary,
    cluster_id: i64,
    package_type: String,
    installed: Option<CardInstalled>,
}

impl PackageCard {
    fn new(
        item: ProjectSummary,
        cluster_id: i64,
        package_type: String,
        installed: Option<CardInstalled>,
    ) -> Self {
        Self {
            item,
            cluster_id,
            package_type,
            installed,
        }
    }
}

impl Component for PackageCard {
    fn render(&self) -> impl IntoElement {
        let id = self.item.id.clone();
        let provider = self.item.provider;
        let package_type = self.package_type.clone();
        let icon_url = self.item.icon_url.clone();
        let banner = match self.item.banner_url.clone() {
            Some(url) => PackageBanner::new(Some(url), BANNER_H).sharp(),
            None => PackageBanner::new(icon_url.clone(), BANNER_H),
        };
        let cluster_id = self.cluster_id;

        let a11y_id = use_a11y();
        let focus = use_focus(a11y_id);
        let focused = focus().is_focused();

        rect()
            .vertical()
            .width(Size::flex(1.0))
            .height(Size::px(CARD_H))
            .corner_radius(CornerRadius::new_all(8.))
            .background(colors::component_bg())
            .border(border_all_color(
                1.,
                if focused {
                    colors::brand()
                } else {
                    colors::component_border()
                },
            ))
            .overflow(Overflow::Clip)
            .a11y_id(a11y_id)
            .a11y_focusable(true)
            .a11y_role(AccessibilityRole::Button)
            .cursor(CursorIcon::Pointer)
            .on_press(move |_| open_package(cluster_id, &package_type, provider, &id))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(BANNER_H))
                    .overflow(Overflow::Clip)
                    // No installed badge, the control at the foot of the card already states it
                    .child(banner.backdrop_only()),
            )
            .child(
                rect()
                    .position(
                        Position::new_absolute()
                            .top(BANNER_H - CARD_ICON + CARD_ICON_OVERHANG)
                            .left(14.),
                    )
                    .layer(Layer::Relative(7))
                    .child(Thumbnail::new(icon_url, CARD_ICON).radius(6.)),
            )
            .child(
                rect()
                    .vertical()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .padding(Gaps::new(CARD_ICON_OVERHANG + 6., 14., 12., 14.))
                    .main_align(Alignment::SpaceBetween)
                    .child(
                        rect()
                            .vertical()
                            .width(Size::fill())
                            .spacing(6.)
                            .child(
                                label()
                                    .text(self.item.name.clone())
                                    .font_size(14.)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .max_lines(1)
                                    .color(colors::fg_primary()),
                            )
                            .child(
                                rect()
                                    .horizontal()
                                    .cross_align(Alignment::Center)
                                    .spacing(5.)
                                    .child(
                                        label()
                                            .text(format!(
                                                "by {} · {} downloads",
                                                self.item.author,
                                                abbreviate_number(self.item.downloads)
                                            ))
                                            .font_size(10.)
                                            .max_lines(1)
                                            .color(colors::fg_primary().with_a(140)),
                                    )
                                    .child(Icon::new(self.item.provider).size(11.)),
                            )
                            .child(
                                label()
                                    .text(self.item.summary.clone())
                                    .font_size(11.)
                                    .line_height(1.45)
                                    .max_lines(2)
                                    .width(Size::fill())
                                    .color(colors::fg_primary().with_a(184)),
                            ),
                    )
                    .child(InstallButton::new(
                        &self.item,
                        self.cluster_id,
                        &self.package_type,
                        self.installed.clone(),
                    )),
            )
    }
}

pub(super) fn list_row(
    item: ProjectSummary,
    cluster_id: i64,
    package_type: &str,
    installed: &InstalledMap,
) -> impl IntoElement {
    ListRow {
        installed: installed_for(installed, &item, cluster_id),
        item,
        cluster_id,
        package_type: package_type.to_string(),
    }
}

#[derive(PartialEq)]
struct ListRow {
    item: ProjectSummary,
    cluster_id: i64,
    package_type: String,
    installed: Option<CardInstalled>,
}

impl Component for ListRow {
    fn render(&self) -> impl IntoElement {
        let item = self.item.clone();
        let cluster_id = self.cluster_id;
        let id = item.id.clone();
        let provider = item.provider;
        let package_type = self.package_type.clone();

        let a11y_id = use_a11y();
        let focus = use_focus(a11y_id);
        let focused = focus().is_focused();

        rect()
            .horizontal()
            .width(Size::fill())
            .height(Size::px(LIST_ROW_H))
            .cross_align(Alignment::Center)
            .spacing(14.)
            .padding(Gaps::new_all(14.))
            .corner_radius(CornerRadius::new_all(12.))
            .background(colors::component_bg())
            .border(border_all_color(
                1.,
                if focused {
                    colors::brand()
                } else {
                    colors::component_border()
                },
            ))
            .overflow(Overflow::Clip)
            .content(Content::Flex)
            .a11y_id(a11y_id)
            .a11y_focusable(true)
            .a11y_role(AccessibilityRole::Button)
            .cursor(CursorIcon::Pointer)
            .on_press(move |_| open_package(cluster_id, &package_type, provider, &id))
            .child(Thumbnail::new(item.icon_url.clone(), LIST_ICON).radius(12.))
            .child(
                rect()
                    .vertical()
                    .width(Size::flex(1.0))
                    .spacing(5.)
                    .child(
                        rect()
                            .horizontal()
                            .width(Size::fill())
                            .cross_align(Alignment::Center)
                            .spacing(6.)
                            .child(
                                label()
                                    .text(item.name.clone())
                                    .font_size(16.)
                                    .font_weight(FontWeight::BOLD)
                                    .max_lines(1)
                                    .color(colors::fg_primary()),
                            )
                            .child(
                                label()
                                    .text(format!("by {}", item.author))
                                    .font_size(12.)
                                    .max_lines(1)
                                    .color(colors::fg_secondary()),
                            )
                            .child(Icon::new(item.provider).size(12.)),
                    )
                    .child(
                        label()
                            .text(item.summary.clone())
                            .font_size(12.)
                            .max_lines(2)
                            .width(Size::fill())
                            .color(colors::fg_primary().with_a(184)),
                    )
                    .child(tag_chips(&item)),
            )
            .child(row_stats(&item))
            .child(InstallButton::new(
                &self.item,
                self.cluster_id,
                &self.package_type,
                self.installed.clone(),
            ))
    }
}

/// Chips a row has room for before the rest fold into "+N"
const MAX_CHIPS: usize = 5;

/// Where a project runs, worded the way Modrinth's own listings word it
pub(super) fn environment_label(client: SideSupport, server: SideSupport) -> Option<&'static str> {
    use SideSupport::{Optional, Required, Unsupported};
    match (client, server) {
        (Optional, Optional) => Some("Client or server"),
        (Required, Required) => Some("Client and server"),
        (Required | Optional, Unsupported) | (Required, Optional) => Some("Client"),
        (Unsupported, Required | Optional) | (Optional, Required) => Some("Server"),
        _ => None,
    }
}

/// Modrinth's loader colours, so a chip reads as the loader it names
fn loader_color(loader: GameLoader) -> Color {
    match loader {
        GameLoader::Fabric => Color::from_rgb(219, 182, 155),
        GameLoader::Forge => Color::from_rgb(149, 158, 239),
        GameLoader::NeoForge => Color::from_rgb(249, 158, 107),
        GameLoader::Quilt => sidebar::QUILT_COLOR,
        _ => colors::fg_secondary(),
    }
}

fn tag_chips(item: &ProjectSummary) -> impl IntoElement {
    let mut chips: Vec<Element> = Vec::new();
    if let Some(environment) = environment_label(item.client_side, item.server_side) {
        chips.push(chip(
            Some(
                Icon::new(IconType::Globe01)
                    .size(11.)
                    .color(colors::fg_secondary())
                    .into_element(),
            ),
            environment.to_string(),
            colors::fg_secondary(),
        ));
    }
    for category in &item.categories {
        chips.push(chip(
            None,
            sidebar::category_label(category),
            colors::fg_secondary(),
        ));
    }
    for loader in item.loaders.iter().filter(|l| l.is_modded()) {
        chips.push(chip(
            sidebar::loader_mark(*loader, 11.),
            sidebar::loader_label(*loader).to_string(),
            loader_color(*loader),
        ));
    }

    let hidden = chips.len().saturating_sub(MAX_CHIPS);
    chips.truncate(MAX_CHIPS);
    if hidden > 0 {
        chips.push(chip(None, format!("+{hidden}"), colors::fg_secondary()));
    }

    rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .overflow(Overflow::Clip)
        .children(chips)
}

fn chip(leading: Option<Element>, text: String, color: Color) -> Element {
    rect()
        .horizontal()
        .height(Size::px(22.))
        .cross_align(Alignment::Center)
        .spacing(4.)
        .padding(Gaps::new_symmetric(0., 8.))
        .corner_radius(CornerRadius::new_all(999.))
        .background(colors::ghost_overlay())
        .border(border_all_color(1., colors::component_border()))
        .maybe_child(leading)
        .child(
            label()
                .text(text)
                .font_size(11.)
                .font_weight(FontWeight::MEDIUM)
                .max_lines(1)
                .color(color),
        )
        .into_element()
}

fn row_stats(item: &ProjectSummary) -> impl IntoElement {
    rect()
        .vertical()
        .cross_align(Alignment::End)
        .spacing(8.)
        .child(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(12.)
                .child(stat(
                    IconType::Download01,
                    abbreviate_number(item.downloads),
                ))
                .maybe_child(
                    item.follows
                        .map(|follows| stat(IconType::Heart, abbreviate_number(follows))),
                ),
        )
        .child(stat(
            IconType::ClockRewind,
            updated_ago(item.updated, Utc::now()),
        ))
}

fn stat(icon: IconType, text: String) -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(5.)
        .child(Icon::new(icon).size(13.).color(colors::fg_secondary()))
        .child(
            label()
                .text(text)
                .font_size(12.)
                .font_weight(FontWeight::MEDIUM)
                .max_lines(1)
                .color(colors::fg_secondary()),
        )
        .into_element()
}

pub(super) fn updated_ago(updated: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let minutes = (now - updated).num_minutes().max(0);
    let hours = minutes / 60;
    let days = hours / 24;
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };

    match days {
        0 if minutes < 1 => "Just now".to_string(),
        0 if hours < 1 => plural(minutes, "minute"),
        0 => plural(hours, "hour"),
        1 => "Yesterday".to_string(),
        2..=6 => format!("{days} days ago"),
        7..=13 => "Last week".to_string(),
        14..=29 => format!("{} weeks ago", days / 7),
        30..=59 => "Last month".to_string(),
        60..=364 => format!("{} months ago", days / 30),
        365..=729 => "Last year".to_string(),
        _ => format!("{} years ago", days / 365),
    }
}

#[derive(PartialEq)]
struct InstallButton {
    provider: ProviderId,
    project_id: String,
    cluster_id: i64,
    content_type: ContentType,
    /// Cluster already has this one, so the control becomes a static pill
    installed: Option<CardInstalled>,
}

impl InstallButton {
    fn new(
        item: &ProjectSummary,
        cluster_id: i64,
        package_type: &str,
        installed: Option<CardInstalled>,
    ) -> Self {
        Self {
            provider: item.provider,
            project_id: item.id.clone(),
            cluster_id,
            content_type: content_type_for_slug(package_type),
            installed,
        }
    }
}

impl Component for InstallButton {
    fn render(&self) -> impl IntoElement {
        let cluster_id = self.cluster_id;
        let provider = self.provider;
        let project_id = self.project_id.clone();

        let dispatch = use_dispatch();
        let compat = *use_browser_compat().read();
        let cluster = use_cluster(cluster_id);

        // The same narrowing the package page does so both agree on what "latest" installs
        let is_modpack = self.content_type == ContentType::Modpack;
        let (game_version, loader) = match (compat && !is_modpack, &cluster) {
            (true, Some(c)) => (
                Some(c.mc_version.clone()),
                (self.content_type == ContentType::Mod).then_some(c.mc_loader),
            ),
            _ => (None, None),
        };

        let versions = version_list(&use_package_versions_when(
            self.installed.is_none(),
            provider,
            project_id.clone(),
            game_version,
            loader,
            0,
            if is_modpack {
                ALL_VERSIONS
            } else {
                VERSIONS_PAGE_SIZE
            },
        ));
        let latest = preferred_version(&versions, self.content_type).map(|v| v.version_id.clone());
        let is_datapack = self.content_type == ContentType::DataPack;
        let mut world_prompt = use_state(|| None::<String>);
        let mut modpack_prompt = use_state(|| false);
        let mut choices = use_state(Vec::new);

        // Nothing to start twice while an install is running or before versions arrive
        let (installing, waiting) =
            use_installs_snapshot().package_busy(is_modpack, cluster_id, provider, &project_id);

        if let Some(installed) = &self.installed {
            if let Some(action) = installed.enable.clone() {
                return EnableButton {
                    action,
                    variant: EnableVariant::Card {
                        height: INSTALL_BUTTON_H,
                    },
                }
                .into_element();
            }
            let installed = installed.source;
            let color = installed.color();
            return rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .height(Size::px(INSTALL_BUTTON_H))
                .spacing(6.)
                .padding(Gaps::new_symmetric(0., 10.))
                .corner_radius(CornerRadius::new_all(6.))
                .background(colors::component_bg())
                .border(border_all_color(1., colors::component_border()))
                .child(Icon::new(IconType::CheckCircle).size(12.).color(color))
                .child(
                    label()
                        .text(installed.label())
                        .font_size(11.)
                        .font_weight(FontWeight::NORMAL)
                        .max_lines(1)
                        .color(colors::fg_secondary()),
                )
                .into_element();
        }

        rect()
            // The card behind is one big press target stop here so the package page doesn't open on top of the install
            .on_press(|e: Event<PressEventData>| e.stop_propagation())
            .child(
                Button::new()
                    .primary()
                    .small()
                    .height(Size::px(INSTALL_BUTTON_H))
                    .padding(Gaps::new_symmetric(0., 11.))
                    .enabled(latest.is_some() && !installing && !waiting)
                    .on_press({
                        let project_id = project_id.clone();
                        move |_| {
                            let Some(version_id) = latest.clone() else {
                                return;
                            };
                            if is_datapack {
                                world_prompt.set(Some(version_id));
                            } else if is_modpack {
                                let picks = minecraft_choices(&versions);
                                if picks.len() > 1 {
                                    choices.set(picks);
                                    modpack_prompt.set(true);
                                } else {
                                    dispatch.install_modpack(ModpackSource::Provider {
                                        provider,
                                        project_id: project_id.clone(),
                                        version_id,
                                    });
                                }
                            } else {
                                dispatch.install_package(
                                    cluster_id,
                                    provider,
                                    project_id.clone(),
                                    version_id,
                                    None,
                                );
                            }
                        }
                    })
                    .child(
                        Icon::new(if installing {
                            IconType::Loading02
                        } else {
                            IconType::Plus
                        })
                        .size(12.)
                        .color(colors::fg_primary()),
                    )
                    .child(
                        label()
                            .text(if installing { "Installing" } else { "Install" })
                            .font_size(11.)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .max_lines(1)
                            .color(colors::fg_primary()),
                    ),
            )
            .maybe_child(world_prompt.read().is_some().then_some(WorldInstallPrompt {
                cluster_id,
                provider,
                project_id: project_id.clone(),
                pending: world_prompt,
            }))
            .maybe_child(modpack_prompt.read().then(|| ModpackVersionPrompt {
                provider,
                project_id,
                choices: choices.read().clone(),
                open: modpack_prompt,
            }))
            .into_element()
    }
}

pub(super) fn empty_state() -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::fill())
        .center()
        .child(
            label()
                .text("No results. Try a different search, category or provider.")
                .font_size(14.)
                .color(colors::fg_secondary()),
        )
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    #[test]
    fn environment_reads_like_modrinths_listing() {
        use SideSupport::{Optional, Required, Unknown, Unsupported};
        assert_eq!(
            environment_label(Optional, Optional),
            Some("Client or server")
        );
        assert_eq!(
            environment_label(Required, Required),
            Some("Client and server")
        );
        assert_eq!(environment_label(Required, Unsupported), Some("Client"));
        assert_eq!(environment_label(Required, Optional), Some("Client"));
        assert_eq!(environment_label(Unsupported, Required), Some("Server"));
        assert_eq!(environment_label(Optional, Required), Some("Server"));
        assert_eq!(environment_label(Unsupported, Unsupported), None);
        assert_eq!(
            environment_label(Unknown, Unknown),
            None,
            "CurseForge reports no sides, so its rows get no chip"
        );
    }

    #[test]
    fn update_age_reads_in_the_largest_whole_unit() {
        let now = Utc::now();
        let ago = |delta: TimeDelta| updated_ago(now - delta, now);

        assert_eq!(ago(TimeDelta::seconds(20)), "Just now");
        assert_eq!(ago(TimeDelta::minutes(1)), "1 minute ago");
        assert_eq!(ago(TimeDelta::minutes(45)), "45 minutes ago");
        assert_eq!(ago(TimeDelta::hours(5)), "5 hours ago");
        assert_eq!(ago(TimeDelta::days(1)), "Yesterday");
        assert_eq!(ago(TimeDelta::days(3)), "3 days ago");
        assert_eq!(ago(TimeDelta::days(9)), "Last week");
        assert_eq!(ago(TimeDelta::days(16)), "2 weeks ago");
        assert_eq!(ago(TimeDelta::days(40)), "Last month");
        assert_eq!(ago(TimeDelta::days(213)), "7 months ago");
        assert_eq!(ago(TimeDelta::days(400)), "Last year");
        assert_eq!(ago(TimeDelta::days(1100)), "3 years ago");
        assert_eq!(
            updated_ago(now + TimeDelta::hours(2), now),
            "Just now",
            "a clock running ahead must not read as negative"
        );
    }
}
