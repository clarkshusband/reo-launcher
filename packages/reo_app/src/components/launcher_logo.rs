use freya::elements::image::{ImageHandle, image};
use freya::prelude::*;

use super::local_image::decode;
use crate::AppAssets;
use crate::theme::colors;

/// Intrinsic aspect ratio (width / height) of the `reo-icon.png` asset.
const MARK_ASPECT: f32 = 150. / 118.;

/// The Reo mark by itself, sized by height. Width follows the artwork's aspect.
#[derive(PartialEq)]
pub struct ReoMark {
    height_px: f32,
}

impl ReoMark {
    pub fn new(height_px: f32) -> Self {
        Self { height_px }
    }
}

impl Component for ReoMark {
    fn render(&self) -> impl IntoElement {
        let height = self.height_px;
        let handle: Option<ImageHandle> = use_hook(|| {
            AppAssets::get_bytes("reo-icon.png")
                .as_ref()
                .and_then(decode)
        });

        rect()
            .width(Size::px(height * MARK_ASPECT))
            .height(Size::px(height))
            .maybe_child(handle.map(|handle| {
                image(handle)
                    .width(Size::fill())
                    .height(Size::fill())
                    .into_element()
            }))
    }
}

/// Mark + "REO LAUNCHER" wordmark lockup.
#[derive(PartialEq)]
pub struct ReoLogo {
    mark_px: f32,
    font_px: f32,
}

impl ReoLogo {
    pub fn new(mark_px: f32, font_px: f32) -> Self {
        Self { mark_px, font_px }
    }
}

impl Component for ReoLogo {
    fn render(&self) -> impl IntoElement {
        let mark_px = self.mark_px;
        let font_px = self.font_px;

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(mark_px * 0.5)
            .child(ReoMark::new(mark_px).into_element())
            .child(
                label()
                    .text("REO LAUNCHER")
                    .font_size(font_px)
                    .font_weight(FontWeight::BOLD)
                    .letter_spacing(font_px * 0.05)
                    .color(colors::fg_primary()),
            )
    }
}
