//! Small shared UI helpers: notifications, form rows and the dialog chrome every
//! modal in the app shares.

use std::future::Future;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, FontWeight, Hsla, IntoElement, ParentElement as _, Pixels, SharedString, Styled as _,
    Window, div, px, relative,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{Dialog, DialogEntrance};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};

/// Spacing scale shared by dialogs and forms, so every modal breathes the same.
pub mod space {
    /// Between related items (label → control → hint).
    pub const TIGHT: f32 = 6.;
    /// Between form rows / sections inside a dialog.
    pub const ROW: f32 = 16.;
    /// Dialog padding on every side.
    pub const DIALOG: f32 = 20.;
    /// Height of a standard control (input, button, folder field).
    pub const CONTROL_H: f32 = 32.;
}

/// Standard dialog widths.
pub mod width {
    pub const SM: f32 = 440.;
    pub const MD: f32 = 540.;
    pub const LG: f32 = 680.;
}

/// Runs a backend call on gpui's executor. Errors become a notification
/// (`"<what>: <error>"`); success runs `on_ok` with window access.
pub fn spawn_op<T: 'static>(
    window: &Window,
    cx: &mut App,
    what: impl Into<String>,
    fut: impl Future<Output = storm_proto::Result<T>> + 'static,
    on_ok: impl FnOnce(T, &mut Window, &mut App) + 'static,
) {
    let what = what.into();
    window
        .spawn(cx, async move |cx| match fut.await {
            Ok(v) => {
                let _ = cx.update(|window, cx| on_ok(v, window, cx));
            }
            Err(e) => {
                tracing::warn!("{what}: {e}");
                let _ = cx.update(|window, cx| notify_error(window, cx, format!("{what}: {e}")));
            }
        })
        .detach();
}

pub fn notify_error(window: &mut Window, cx: &mut App, msg: impl Into<SharedString>) {
    window.push_notification(Notification::error(msg), cx);
}

pub fn notify_info(window: &mut Window, cx: &mut App, msg: impl Into<SharedString>) {
    window.push_notification(Notification::info(msg), cx);
}

/// A `label: value` row for property lists.
#[allow(dead_code)] // Shared helper; not every build of the views uses it.
pub fn field(label: impl Into<SharedString>, value: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_start()
        .text_sm()
        .child(div().w(px(120.)).flex_shrink_0().text_color(cx.theme().muted_foreground).child(label.into()))
        .child(div().flex_1().min_w_0().child(value))
}

/// Form label with an optional hint underneath the control.
pub fn form_row(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    hint: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .gap(px(space::TIGHT))
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).line_height(relative(1.25)).child(label.into()))
        .child(control)
        .children(hint.map(|h| div().text_xs().text_color(muted).child(h)))
}

/// An icon and a label on one optically centred line. Use this anywhere an icon
/// sits next to text so they share a line box (no baseline drift).
pub fn icon_label(icon: impl Into<Icon>, label: impl Into<SharedString>, color: Option<Hsla>) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_center()
        .line_height(relative(1.))
        .child(Icon::new(icon).small().flex_shrink_0().when_some(color, |i, c| i.text_color(c)))
        .child(div().min_w_0().truncate().child(label.into()))
}

/// Small muted heading that groups related settings.
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div().text_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child(text.into())
}

// ---------------------------------------------------------------------------
// Dialog chrome
// ---------------------------------------------------------------------------

/// Applies Storm's modal look to a gpui-component [`Dialog`]: fixed width, even
/// padding, a raised surface over a "frosted" scrim (the theme's `overlay`),
/// and a fade-and-settle entrance.
///
/// gpui has no backdrop-filter, so the app behind a modal can't be truly
/// blurred; the scrim instead washes it out towards the surface colour, which
/// reads as frosted glass without the noise of sharp text showing through.
pub fn modal(d: Dialog, header: impl IntoElement, w: f32, cx: &App) -> Dialog {
    let theme = cx.theme();
    // Opaque on purpose: without a real blur, a see-through surface lets sharp
    // text from behind ghost through the form.
    // The built-in close button is absolutely positioned from the padding and
    // doesn't line up with our header row; `dialog_header` renders its own.
    d.title(header)
        .close_button(false)
        .w(px(w))
        .p(px(space::DIALOG))
        .bg(theme.popover)
        .border_color(theme.border.opacity(if theme.is_dark() { 1. } else { 0.9 }))
        .rounded(theme.radius_lg)
        .entrance(DialogEntrance::FadeSlide)
}

/// Dialog heading: an icon in a soft tile, the title and an optional one-line
/// description, all vertically centred on the tile, plus the close button on
/// the same centre line. `accent` tints the tile (defaults to the primary
/// colour; pass `danger` for destructive dialogs).
pub fn dialog_header(
    icon: impl Into<Icon>,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    accent: Option<Hsla>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let accent = accent.unwrap_or(theme.primary);
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .child(
            h_flex()
                .flex_shrink_0()
                .size(px(36.))
                .items_center()
                .justify_center()
                .rounded(theme.radius)
                .bg(accent.opacity(if theme.is_dark() { 0.16 } else { 0.1 }))
                .text_color(accent)
                .child(Icon::new(icon).size(px(18.))),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_base().font_semibold().line_height(relative(1.25)).truncate().child(title.into()))
                .children(description.map(|d| {
                    div()
                        .text_sm()
                        .font_weight(FontWeight::NORMAL)
                        .line_height(relative(1.3))
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(d)
                })),
        )
        .child(
            Button::new("dialog-close")
                .ghost()
                .small()
                .icon(IconName::Close)
                .flex_shrink_0()
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .into_any_element()
}

/// Right-aligned footer: optional leading content (status text, secondary
/// actions), then Cancel and the primary action.
pub struct Footer {
    ok_label: SharedString,
    danger: bool,
    busy: bool,
    disabled: bool,
    leading: Option<AnyElement>,
}

impl Footer {
    pub fn new(ok_label: impl Into<SharedString>) -> Self {
        Self { ok_label: ok_label.into(), danger: false, busy: false, disabled: false, leading: None }
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn render(self, on_ok: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
        let Self { ok_label, danger, busy, disabled, leading } = self;
        h_flex()
            .w_full()
            .pt(px(space::TIGHT))
            .gap_2()
            .items_center()
            .child(h_flex().flex_1().min_w_0().gap_2().items_center().children(leading))
            .child(
                Button::new("cancel")
                    .ghost()
                    .label("Cancel")
                    .min_w(px(80.))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                Button::new("ok")
                    .label(ok_label)
                    .min_w(px(80.))
                    .map(|b| if danger { b.danger() } else { b.primary() })
                    .loading(busy)
                    .disabled(disabled || busy)
                    .on_click(on_ok),
            )
    }
}

/// Inline error under a form. `None` when there's no error, so it can go in
/// `.children(…)` without adding an empty gap.
pub fn error_banner(err: Option<&str>, cx: &App) -> Option<AnyElement> {
    let theme = cx.theme();
    err.map(|e| {
        h_flex()
            .gap_2()
            .items_start()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .bg(theme.danger.opacity(0.08))
            .border_1()
            .border_color(theme.danger.opacity(0.25))
            .text_sm()
            .text_color(theme.danger)
            .child(
                // Nudge the icon onto the first text line's centre.
                div().pt(px(2.)).child(Icon::new(IconName::CircleX).small()),
            )
            .child(div().flex_1().min_w_0().child(e.to_owned()))
            .into_any_element()
    })
}

/// Height every single-line control in a dialog should use.
pub fn control_h() -> Pixels {
    px(space::CONTROL_H)
}
