use std::time::Instant;

use freya::prelude::*;

use crate::theme;

pub fn border_all(width: f32) -> Border {
    Border::new()
        .fill(theme::colors::component_border())
        .width(BorderWidth {
            top: width,
            right: width,
            bottom: width,
            left: width,
        })
}

pub fn border_all_color(width: f32, color: Color) -> Border {
    Border::new().fill(color).width(BorderWidth {
        top: width,
        right: width,
        bottom: width,
        left: width,
    })
}

/// A tinted strip of explanation under a row; `accent` washes the background
pub fn note(message: String, accent: Color) -> Element {
    rect()
        .horizontal()
        .width(Size::fill())
        .content(Content::Flex)
        .spacing(10.)
        .padding(Gaps::new_symmetric(12., 14.))
        .corner_radius(CornerRadius::new_all(10.))
        .background(accent.with_a(30))
        .child(
            label()
                .text(message)
                .font_size(12.)
                .max_lines(5)
                .width(Size::flex(1.0))
                .color(theme::colors::fg_primary()),
        )
        .into_element()
}

/// A captioned, read-only path in a boxed field
pub fn path_block(caption: &'static str, path: &std::path::Path) -> Element {
    rect()
        .vertical()
        .width(Size::fill())
        .spacing(4.)
        .child(
            label()
                .text(caption)
                .font_size(11.)
                .color(theme::colors::fg_secondary()),
        )
        .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new_all(10.))
                .corner_radius(CornerRadius::new_all(8.))
                .background(theme::colors::component_bg())
                .border(border_all_color(1., theme::colors::component_border()))
                .child(
                    label()
                        .text(path.display().to_string())
                        .font_size(12.)
                        .max_lines(3)
                        .width(Size::fill())
                        .color(theme::colors::fg_primary()),
                ),
        )
        .into_element()
}

pub fn fmt_date(ts: chrono::DateTime<chrono::Utc>) -> String {
    ts.format("%Y-%m-%d %H:%M").to_string()
}

/// Shortest band is a minute nothing re-renders on a timer so a seconds counter
/// would freeze at whatever it read when the row was drawn
pub fn relative_time(created_at: Instant) -> String {
    let secs = created_at.elapsed().as_secs();
    match secs {
        0..=59 => "Just now".to_string(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

pub fn last_played_label(ts: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let Some(ts) = ts else {
        return "Never".to_string();
    };
    match (chrono::Utc::now() - ts).num_days() {
        ..=0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        d @ 2..=6 => format!("{d} days ago"),
        d @ 7..=13 => format!("{} week ago", d / 7),
        d @ 14..=29 => format!("{} weeks ago", d / 7),
        d @ 30..=59 => format!("{} month ago", d / 30),
        d => format!("{} months ago", d / 30),
    }
}

/// Gap the clamped menu keeps from the window edges
pub const EDGE_MARGIN: f32 = 8.;

/// `root_size` is physical while the press position is logical, so it has to be
/// scaled down before the two are compared
pub fn clamp_to_window(x: f32, y: f32, width: f32, height: f32) -> (f32, f32) {
    let platform = Platform::get();
    let scale = *platform.scale_factor.peek() as f32;
    if scale <= 0. {
        return (x, y);
    }

    let window = *platform.root_size.peek();
    let clamp = |pos: f32, len: f32, limit: f32| {
        pos.clamp(EDGE_MARGIN, (limit - len - EDGE_MARGIN).max(EDGE_MARGIN))
    };

    (
        clamp(x, width, window.width / scale),
        clamp(y, height, window.height / scale),
    )
}

/// Returns the `Rect` not an `Element` so callers can inset or round it
pub fn divider() -> Rect {
    rect()
        .width(Size::fill())
        .height(Size::px(1.))
        .background(theme::colors::component_border())
}

pub fn centered_note(text: &str) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(240.))
        .center()
        .child(
            label()
                .text(text.to_string())
                .font_size(14.)
                .color(theme::colors::fg_secondary()),
        )
        .into_element()
}

pub fn grid_columns_for_width(width: f32, max_col: f32, gap: f32) -> usize {
    if width <= 0. {
        return 1;
    }

    (((width + gap) / (max_col + gap)).ceil() as usize).max(1)
}

pub fn columns_for(width: f32, min_cell: f32, max: usize, gap: f32) -> usize {
    let max = max.max(1);
    if width <= 0. {
        return max;
    }

    (((width + gap) / (min_cell + gap)).floor() as usize).clamp(1, max)
}

pub fn fixed_grid(cards: Vec<Element>, columns: usize, card_height: f32, gap: f32) -> Element {
    let columns = columns.max(1);
    let mut root = rect().vertical().width(Size::fill()).spacing(gap);

    for (index, chunk) in cards.chunks(columns).enumerate() {
        let mut row = rect()
            .key(index)
            .horizontal()
            .width(Size::fill())
            .height(Size::px(card_height))
            .content(Content::Flex)
            .spacing(gap);

        for card in chunk {
            row = row.child(
                rect()
                    .width(Size::flex(1.0))
                    .height(Size::fill())
                    .child(card.clone()),
            );
        }
        for _ in chunk.len()..columns {
            row = row.child(rect().width(Size::flex(1.0)).height(Size::fill()));
        }

        root = root.child(row.into_element());
    }

    root.into_element()
}

/// Short final rows are padded with empty flex cells so tiles keep the column width
pub fn flow_grid(items: Vec<Element>, cols: usize, mut width: State<f32>, gap: f32) -> Element {
    let cols = cols.max(1);

    let mut root = rect().vertical().width(Size::fill()).spacing(gap);
    let mut iter = items.into_iter();
    let mut remaining = true;
    while remaining {
        let mut row = rect()
            .horizontal()
            .width(Size::fill())
            .spacing(gap)
            .content(Content::Flex);
        let mut filled = 0;
        for _ in 0..cols {
            if let Some(item) = iter.next() {
                row = row.child(item);
                filled += 1;
            } else {
                row = row.child(rect().width(Size::flex(1.0)));
            }
        }
        if filled == 0 {
            break;
        }
        remaining = filled == cols;
        root = root.child(row.into_element());
    }

    root.on_sized(move |event: Event<SizedEventData>| {
        let w = event.data().area.width();
        if (w - *width.peek()).abs() > 0.5 {
            *width.write() = w;
        }
    })
    .into_element()
}

pub fn entrance_motion_layer(
    slide_x: f32,
    slide_y: f32,
    opacity: f32,
    child: impl IntoElement,
) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::fill())
        .overflow(Overflow::Clip)
        .child(
            rect()
                .width(Size::fill())
                .height(Size::fill())
                .position(Position::new_absolute().top(slide_y).left(slide_x))
                .opacity(opacity)
                .child(child),
        )
        .into_element()
}

/// Peak blur sigma per pixel a page slides. Skia reads this as sigma, so the visible smear is
/// about three times wider
const MOTION_BLUR_PER_PX: f32 = 0.045;
/// Under this the blur cannot be seen but the backdrop pass still costs a frame's worth of work
const MIN_VISIBLE_BLUR: f32 = 0.25;
/// Paints above everything a page draws (nesting adds a layer per level and scroll bars sit 999
/// higher) while staying under popups, which use overlay layers
const MOTION_BLUR_LAYER: i16 = 1500;

/// Blur for a page that fades in (opacity `p`) while a cubic ease-out slides it `slide_px`.
/// Such a slide moves at a speed proportional to (1 - p)^(2/3), and the blur follows speed times
/// opacity: the veil also softens the still background under the page, so the frames where the
/// page has not shown up yet stay sharp and the blur lands on the page while it is in motion
pub fn motion_blur(slide_px: f32, p: f32) -> f32 {
    let p = p.clamp(0., 1.);
    // p * (1 - p)^(2/3) peaks at p = 3/5, scaled so that peak is the full blur
    let peak = 0.6 * 0.4_f32.powf(2. / 3.);
    slide_px.abs() * MOTION_BLUR_PER_PX * p * (1. - p).powf(2. / 3.) / peak
}

/// Draws `content` under a veil that blurs it by `blur` without taking pointer events, so the
/// page stays clickable mid-transition. Both children are keyed: unkeyed siblings pair up by
/// position, so the veil coming and going would otherwise remount the page under it
pub fn motion_blurred(blur: f32, content: impl IntoElement) -> impl IntoElement {
    // The veil goes first: an absolute child is sized from the space its earlier siblings
    // leave, and the page takes all of it. Its layer still paints it on top
    rect()
        .width(Size::fill())
        .height(Size::fill())
        .maybe_child((blur > MIN_VISIBLE_BLUR).then(|| {
            rect()
                .key("motion-blur-veil")
                .width(Size::fill())
                .height(Size::fill())
                .position(Position::new_absolute().top(0.).left(0.))
                .layer(Layer::Relative(MOTION_BLUR_LAYER))
                .interactive(false)
                .backdrop_blur(blur)
        }))
        .child(
            rect()
                .key("motion-blur-content")
                .width(Size::fill())
                .height(Size::fill())
                .child(content),
        )
        .into_element()
}

pub trait ImageFallbackExt: Sized {
    fn fallback(self, placeholder: impl IntoElement) -> Self;
}

impl ImageFallbackExt for ImageViewer {
    fn fallback(self, placeholder: impl IntoElement) -> Self {
        let placeholder = placeholder.into_element();
        self.error_renderer(move |_: String| placeholder.clone())
    }
}

pub fn window_logical_size() -> Size2D {
    let platform = Platform::get();
    let scale = *platform.scale_factor.peek() as f32;
    let size = *platform.root_size.peek();
    if scale <= 0. {
        return size;
    }

    Size2D::new(size.width / scale, size.height / scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_blur_lands_on_the_page_while_it_moves() {
        assert_eq!(
            motion_blur(48., 0.),
            0.,
            "nothing of the page is on screen yet, only the background"
        );
        assert!(motion_blur(48., 0.6) > motion_blur(48., 0.15));
        assert!(motion_blur(48., 0.6) > motion_blur(48., 0.95));
        assert_eq!(motion_blur(48., 1.), 0.);
        assert_eq!(
            motion_blur(48., 1.2),
            0.,
            "an overshooting value must not blur"
        );
    }

    #[test]
    fn motion_blur_peaks_at_the_configured_strength() {
        let peak = motion_blur(48., 0.6);
        assert!((peak - 48. * MOTION_BLUR_PER_PX).abs() < 1e-4, "{peak}");
    }

    #[test]
    fn motion_blur_scales_with_distance_whichever_way_the_page_moves() {
        assert_eq!(motion_blur(-44., 0.25), motion_blur(44., 0.25));
        assert!(motion_blur(48., 0.5) > motion_blur(22., 0.5));
        assert_eq!(
            motion_blur(0., 0.5),
            0.,
            "a page that stays put stays sharp"
        );
    }
}
