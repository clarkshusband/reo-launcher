use freya::prelude::*;
use freya::query::{MutationStateData, UseMutation};

use crate::components::{Button, Icon, IconType, OverlayPopup, TextInput};
use crate::hooks::{
    AddOfflineAccountKeys, AddOfflineAccountMutation, SetDefaultAccountKeys,
    use_add_offline_account, use_set_default_account,
};
use crate::theme::colors;
use crate::ui::border_all_color;

/// The username dialog that adds an offline account. Works without any
/// Microsoft account signed in: good for players who are offline
#[derive(Clone, Copy)]
pub struct OfflineLogin {
    add: UseMutation<AddOfflineAccountMutation>,
    open: State<bool>,
    username: State<String>,
    submitting: State<bool>,
}

impl OfflineLogin {
    pub fn open(&self) {
        let mut open = self.open;
        open.set(true);
    }

    fn close(&self) {
        let mut open = self.open;
        open.set(false);
    }

    fn submit(&self) {
        let name = self.username.peek().trim().to_string();
        if name.is_empty() {
            return;
        }
        self.add.mutate(AddOfflineAccountKeys { username: name });
        let mut submitting = self.submitting;
        submitting.set(true);
    }

    pub fn popup(&self) -> Option<impl IntoElement> {
        let handle = *self;
        self.open.read().then(move || offline_dialog(handle))
    }
}

/// `make_default` switches the launcher to the new account once it is saved
pub fn use_offline_login(make_default: bool) -> OfflineLogin {
    let add = use_add_offline_account();
    let set_default = use_set_default_account();
    let mut open = use_state(|| false);
    let mut username = use_state(String::new);
    let mut submitting = use_state(|| false);

    use_side_effect(move || {
        if !*submitting.read() {
            return;
        }
        let added = match &*add.read().state() {
            MutationStateData::Settled {
                res: Ok(account), ..
            } => Some(account.id),
            MutationStateData::Settled { res: Err(_), .. } => None,
            _ => return,
        };
        submitting.set(false);
        let Some(id) = added else { return };

        if make_default {
            set_default.mutate(SetDefaultAccountKeys { id: Some(id) });
        }
        open.set(false);
        username.set(String::new());
    });

    OfflineLogin {
        add,
        open,
        username,
        submitting,
    }
}

fn mutation_err_text(mutation: &UseMutation<AddOfflineAccountMutation>) -> Option<String> {
    match &*mutation.read().state() {
        MutationStateData::Settled { res: Err(err), .. } => Some(err.to_string()),
        MutationStateData::Loading {
            res: Some(Err(err)),
        } => Some(err.to_string()),
        _ => None,
    }
}

fn offline_dialog(handle: OfflineLogin) -> impl IntoElement {
    let name = handle.username.read().trim().to_string();
    let uuid_preview = (!name.is_empty()).then(|| reo_auth::offline_uuid(&name).to_string());
    let error = mutation_err_text(&handle.add);

    OverlayPopup::new()
        .on_close(move |()| handle.close())
        .child(
            rect()
                .width(Size::window_percent(100.))
                .height(Size::window_percent(100.))
                .center()
                .child(
                    rect()
                        .vertical()
                        .width(Size::px(380.))
                        .max_width(Size::window_percent(90.))
                        .spacing(16.)
                        .padding(Gaps::new_all(20.))
                        .corner_radius(CornerRadius::new_all(16.))
                        .background(colors::page_elevated())
                        .border(border_all_color(1., colors::component_border()))
                        .child(
                            label()
                                .text("Add offline account")
                                .font_size(18.)
                                .font_weight(FontWeight::SEMI_BOLD)
                                .color(colors::fg_primary()),
                        )
                        .child(
                            label()
                                .text(
                                    "Plays under this name in singleplayer, LAN and \
                                     offline-mode servers. No internet needed. Online servers \
                                     still want a Microsoft account.",
                                )
                                .font_size(12.)
                                .width(Size::fill())
                                .color(colors::fg_secondary()),
                        )
                        .child(
                            rect()
                                .vertical()
                                .width(Size::fill())
                                .spacing(6.)
                                .child(field_label("Username"))
                                .child(
                                    TextInput::new(handle.username)
                                        .placeholder("Offline username")
                                        .on_submit(move |_| handle.submit()),
                                ),
                        )
                        .child(
                            rect()
                                .vertical()
                                .width(Size::fill())
                                .spacing(6.)
                                .child(field_label("UUID"))
                                .child(
                                    label()
                                        .text(uuid_preview.unwrap_or_else(|| "-".to_string()))
                                        .font_size(12.)
                                        .color(colors::fg_secondary()),
                                ),
                        )
                        .maybe_child(error.map(error_line))
                        .child(
                            rect()
                                .horizontal()
                                .width(Size::fill())
                                .main_align(Alignment::End)
                                .spacing(8.)
                                .child(
                                    Button::new()
                                        .ghost()
                                        .on_press(move |_| handle.close())
                                        .text("Cancel"),
                                )
                                .child(
                                    Button::new()
                                        .primary()
                                        .on_press(move |_| handle.submit())
                                        .child(Icon::new(IconType::Plus).size(16.))
                                        .text("Add account"),
                                ),
                        ),
                ),
        )
        .into_element()
}

fn field_label(text: &str) -> impl IntoElement {
    label()
        .text(text.to_string())
        .font_size(11.)
        .font_weight(FontWeight::MEDIUM)
        .color(colors::fg_secondary())
        .into_element()
}

fn error_line(message: String) -> Element {
    rect()
        .horizontal()
        .width(Size::fill())
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(
            Icon::new(IconType::AlertTriangle)
                .size(13.)
                .color(colors::danger()),
        )
        .child(
            label()
                .text(message)
                .font_size(12.)
                .width(Size::flex(1.0))
                .color(colors::danger()),
        )
        .into_element()
}
