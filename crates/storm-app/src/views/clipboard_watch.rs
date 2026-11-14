//! Offers to add magnet links / `.torrent` URLs that show up on the clipboard.
//!
//! The clipboard is checked when the window is activated and every
//! [`POLL_INTERVAL`] while it stays active. A link is offered once per session
//! (and never if its info hash is already in the torrent list) through a
//! sonner-style toast in the bottom-right corner: Add / Options… / ×.
//!
//! The toast is a gpui-component [`Notification`]: its toast stack already
//! stacks, animates and expands on hover the way sonner does. Its built-in
//! auto-hide is a fixed 5s, too short for something to act on, so we run our own
//! timer that pauses while the toast is hovered or the window is in the background.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::*;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, Theme, WindowExt as _, h_flex, v_flex};
use storm_proto::AddTorrentRequest;

use crate::caps::BackendHandle;
use crate::prefs;
use crate::ui::spawn_op;
use crate::views::clip_link::{self, ClipLink, LinkKind};
use crate::views::dialogs::AddTorrentDialog;

const POLL_INTERVAL: Duration = Duration::from_millis(1500);
/// How long an offer stays up (not counting time hovered or in the background).
const OFFER_TIMEOUT: Duration = Duration::from_secs(12);
const OFFER_TICK: Duration = Duration::from_millis(250);
/// Keeps bottom-anchored toasts clear of the status bar.
const TOAST_BOTTOM_MARGIN: Pixels = px(44.);

/// Notification id type for clipboard offers (keyed by the link).
struct ClipboardOffer;

pub struct ClipboardWatcher {
    /// Links and info hashes already offered (or seen while a dialog was open).
    seen: HashSet<String>,
    /// The clipboard text last examined, so unchanged clipboards cost one compare.
    last_text: Option<String>,
    /// The link whose toast is currently up.
    current: Option<String>,
    offer_timer: Option<Task<()>>,
    _activation: Subscription,
    _poll: Task<()>,
}

impl ClipboardWatcher {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.check(window, cx);
            }
        });
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if window.is_window_active() {
                        this.check(window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Self {
            seen: HashSet::new(),
            last_text: None,
            current: None,
            offer_timer: None,
            _activation: activation,
            _poll: poll,
        }
    }

    fn remember(&mut self, link: &ClipLink) -> bool {
        let new_url = self.seen.insert(link.url.clone());
        let new_hash = link.info_hash.as_ref().is_none_or(|h| self.seen.insert(h.clone()));
        new_url && new_hash
    }

    fn check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !prefs::watch_clipboard(cx) {
            self.dismiss_current(window, cx);
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let changed = self.last_text.as_deref() != Some(text.as_str());

        // A dialog is up, most likely the add dialog the user just pasted this
        // link into: don't interrupt, and don't offer the link afterwards.
        if window.has_active_dialog(cx) {
            if let Some(link) = clip_link::find(&text) {
                self.remember(&link);
                if self.current.as_deref() == Some(link.url.as_str()) {
                    self.dismiss_current(window, cx);
                }
            }
            self.last_text = Some(text);
            return;
        }
        if !changed {
            return;
        }
        let link = clip_link::find(&text);
        self.last_text = Some(text);
        let Some(link) = link else { return };
        if !self.remember(&link) {
            return;
        }

        let Some(hash) = link.info_hash.clone() else {
            self.show(link, window, cx);
            return;
        };
        // Skip torrents we already have. Asked of the backend (only when a new
        // link appears) so this works whatever backend is active.
        let backend = BackendHandle::get(cx).backend;
        cx.spawn_in(window, async move |this, cx| {
            let have = backend
                .list_torrents()
                .await
                .is_ok_and(|ts| ts.iter().any(|t| t.info_hash.eq_ignore_ascii_case(&hash)));
            if !have {
                let _ = this.update_in(cx, |this, window, cx| this.show(link, window, cx));
            }
        })
        .detach();
    }

    /// Marks a link in `text` as already handled (e.g. the user pasted it into
    /// the add dialog) and closes its toast if it is showing.
    pub fn mark_seen(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.last_text = Some(text.to_owned());
        let Some(link) = clip_link::find(text) else { return };
        self.remember(&link);
        if self.current.as_deref() == Some(link.url.as_str()) {
            self.dismiss_current(window, cx);
        }
    }

    fn dismiss_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.current.take() {
            window.remove_notification1::<ClipboardOffer>(url, cx);
        }
        self.offer_timer = None;
    }

    fn show(&mut self, link: ClipLink, window: &mut Window, cx: &mut Context<Self>) {
        // The user may have opened a dialog (or turned this off) while we asked the backend.
        if window.has_active_dialog(cx) || !prefs::watch_clipboard(cx) {
            return;
        }
        self.dismiss_current(window, cx);
        self.current = Some(link.url.clone());

        let hovered = Rc::new(Cell::new(false));
        let weak = cx.entity().downgrade();
        let url = link.url.clone();
        let note = Notification::new()
            .id1::<ClipboardOffer>(link.url.clone())
            .autohide(false)
            .content({
                let hovered = hovered.clone();
                move |_, _, cx| render_offer(&link, hovered.clone(), cx)
            })
            .on_close(move |_, cx| {
                let _ = weak.update(cx, |this, _| {
                    if this.current.as_deref() == Some(url.as_str()) {
                        this.current = None;
                        this.offer_timer = None;
                    }
                });
            });
        push_bottom_right(note, window, cx);

        let url = self.current.clone();
        self.offer_timer = Some(cx.spawn_in(window, async move |this, cx| {
            let mut left = OFFER_TIMEOUT;
            loop {
                cx.background_executor().timer(OFFER_TICK).await;
                let running = this.update_in(cx, |this, window, cx| {
                    if this.current != url {
                        return false;
                    }
                    if !hovered.get() && window.is_window_active() {
                        left = left.saturating_sub(OFFER_TICK);
                    }
                    if left.is_zero() {
                        // Only drop our handle; the task finishes on its own.
                        if let Some(url) = this.current.take() {
                            window.remove_notification1::<ClipboardOffer>(url, cx);
                        }
                        return false;
                    }
                    true
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }
}

fn render_offer(link: &ClipLink, hovered: Rc<Cell<bool>>, cx: &mut Context<Notification>) -> AnyElement {
    let theme = cx.theme();
    let (icon, caption) = match link.kind {
        LinkKind::Magnet => (Lucide::Magnet, "Magnet link on your clipboard"),
        LinkKind::TorrentUrl => (Lucide::Link, "Torrent URL on your clipboard"),
    };
    let add = {
        let link = link.clone();
        cx.listener(move |note: &mut Notification, _: &ClickEvent, window, cx| {
            note.dismiss(window, cx);
            add_now(&link, window, cx);
        })
    };
    let options = {
        let url = link.url.clone();
        cx.listener(move |note: &mut Notification, _: &ClickEvent, window, cx| {
            note.dismiss(window, cx);
            open_options(url.clone(), window, cx);
        })
    };
    h_flex()
        .id("clipboard-offer")
        .on_hover(move |h, _, _| hovered.set(*h))
        .items_start()
        .gap_3()
        .child(
            div()
                .mt(px(2.))
                .size(px(28.))
                .flex_shrink_0()
                .rounded(theme.radius)
                .bg(theme.primary.opacity(0.12))
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon).small().text_color(theme.primary)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .pr_5() // room for the close button
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(link.name.clone()),
                )
                .child(div().text_xs().text_color(theme.muted_foreground).child(caption))
                .child(
                    h_flex()
                        .mt_2()
                        .gap_2()
                        .justify_end()
                        .child(Button::new("clip-options").small().ghost().label("Options…").on_click(options))
                        .child(
                            Button::new("clip-add").small().primary().icon(IconName::Plus).label("Add").on_click(add),
                        ),
                ),
        )
        .into_any_element()
}

/// Adds with default settings, then confirms with a short toast.
fn add_now(link: &ClipLink, window: &mut Window, cx: &mut App) {
    let backend = BackendHandle::get(cx).backend;
    let (url, name) = (link.url.clone(), link.name.clone());
    spawn_op(
        window,
        cx,
        "Adding torrent",
        async move { backend.add_torrent(AddTorrentRequest::url(url)).await },
        move |res, window, cx| {
            let note = if res.already_existed {
                Notification::info(format!("\u{201c}{name}\u{201d} is already in your list"))
            } else {
                Notification::success(format!("Added \u{201c}{name}\u{201d}"))
            };
            push_bottom_right(note, window, cx);
        },
    );
}

/// Opens the regular add dialog prefilled with the link, for picking a folder etc.
fn open_options(url: String, window: &mut Window, cx: &mut App) {
    let backend = BackendHandle::get(cx).backend;
    window
        .spawn(cx, async move |cx| {
            let dir = backend.settings().await.ok().map(|s| s.download_dir);
            let _ = cx.update(|window, cx| AddTorrentDialog::open_links(Some(url), dir, window, cx));
        })
        .detach();
}

/// Shows `note` in the bottom-right stack, above the status bar. Only the
/// bottom margin changes; the app's other toasts are top-anchored.
pub(crate) fn push_bottom_right(note: Notification, window: &mut Window, cx: &mut App) {
    Theme::global_mut(cx).notification.margins.bottom = TOAST_BOTTOM_MARGIN;
    window.push_notification(note.placement(Anchor::BottomRight), cx);
}
