//! In-app folder browser for choosing a folder on a *remote* backend's machine.
//!
//! Don't use this directly: call [`crate::caps::pick_backend_folder`], which picks
//! between this and the native OS dialog.

use std::cell::Cell;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_kit::assets::IconName as LucideIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use storm_proto::{Backend, DirListing};

use crate::caps::OnPick;
use crate::ui::{self, Footer};

/// Height of one folder row.
const ROW_H: f32 = 30.;

pub struct RemoteFolderPicker {
    backend: Arc<dyn Backend>,
    focus_handle: FocusHandle,
    listing: Option<DirListing>,
    loading: bool,
    error: Option<String>,
    selected: Option<usize>,
    /// `Some` while the "New folder" row is shown.
    new_folder: Option<Entity<InputState>>,
    new_folder_error: Option<String>,
    on_pick: OnPick,
    scroll: UniformListScrollHandle,
    _load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl RemoteFolderPicker {
    /// Opens the picker as a dialog. `on_pick` gets the chosen server-side path.
    pub fn open(
        title: &str,
        backend: Arc<dyn Backend>,
        initial: Option<String>,
        on_pick: OnPick,
        window: &mut Window,
        cx: &mut App,
    ) {
        let picker = cx.new(|cx| Self {
            backend,
            focus_handle: cx.focus_handle(),
            listing: None,
            loading: false,
            error: None,
            selected: None,
            new_folder: None,
            new_folder_error: None,
            on_pick,
            scroll: UniformListScrollHandle::new(),
            _load: Task::ready(()),
            _subscriptions: Vec::new(),
        });
        picker.update(cx, |p, cx| p.navigate(initial, cx));

        let title: SharedString = title.to_owned().into();
        let first = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                let focus = picker.read(cx).focus_handle.clone();
                focus.focus(window, cx);
            }
            let header = ui::dialog_header(
                LucideIcon::FolderOpen,
                title.clone(),
                Some("Choose a folder on the server".into()),
                None,
                cx,
            );
            ui::modal(dialog, header, ui::width::LG, cx).child(picker.clone())
        });
    }

    fn navigate(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        self._load = cx.spawn(async move |this, cx| {
            let result = backend.list_dirs(path).await;
            let _ = this.update(cx, |p, cx| {
                p.loading = false;
                match result {
                    Ok(listing) => {
                        p.listing = Some(listing);
                        p.selected = None;
                        p.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                    Err(e) => p.error = Some(e.to_string()),
                }
                cx.notify();
            });
        });
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let current = self.listing.as_ref().map(|l| l.path.clone());
        self.navigate(current, cx);
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.listing.as_ref().and_then(|l| l.parent.clone()) {
            self.navigate(Some(parent), cx);
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let path = self.selected.and_then(|ix| self.listing.as_ref()?.dirs.get(ix)).map(|d| d.path.clone());
        if let Some(path) = path {
            self.navigate(Some(path), cx);
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(count) = self.listing.as_ref().map(|l| l.dirs.len()).filter(|&n| n > 0) else {
            return;
        };
        let next = match self.selected {
            None if delta > 0 => 0,
            None => count - 1,
            Some(ix) => (ix as isize + delta).clamp(0, count as isize - 1) as usize,
        };
        self.selected = Some(next);
        self.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// The path "Select" would return: the highlighted subfolder, else the current folder.
    fn chosen_path(&self) -> Option<String> {
        let listing = self.listing.as_ref()?;
        Some(
            self.selected
                .and_then(|ix| listing.dirs.get(ix))
                .map(|d| d.path.clone())
                .unwrap_or_else(|| listing.path.clone()),
        )
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.chosen_path() else { return };
        let on_pick = self.on_pick.clone();
        window.close_dialog(cx);
        on_pick(path, window, cx);
    }

    fn start_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name"));
        input.read(cx).focus_handle(cx).focus(window, cx);
        self._subscriptions = vec![cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.create_folder(window, cx);
            }
        })];
        self.new_folder = Some(input);
        self.new_folder_error = None;
        cx.notify();
    }

    fn cancel_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_folder = None;
        self.new_folder_error = None;
        self._subscriptions.clear();
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(input), Some(listing)) = (&self.new_folder, &self.listing) else { return };
        let name = match validate_folder_name(&input.read(cx).value(), &listing.separator) {
            Ok(n) => n,
            Err(e) => {
                self.new_folder_error = Some(e);
                cx.notify();
                return;
            }
        };
        let path = join_path(&listing.path, &name, &listing.separator);
        let backend = self.backend.clone();
        self.focus_handle.focus(window, cx);
        self._load = cx.spawn(async move |this, cx| {
            let result = backend.create_dir(path.clone()).await;
            let _ = this.update(cx, |p, cx| match result {
                Ok(()) => {
                    p.new_folder = None;
                    p._subscriptions.clear();
                    // Open the new folder: that's almost always what comes next.
                    p.navigate(Some(path), cx);
                }
                Err(e) => {
                    p.new_folder_error = Some(e.to_string());
                    cx.notify();
                }
            });
        });
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder.is_some() {
            return; // Keys belong to the name input.
        }
        match ev.keystroke.key.as_str() {
            "down" => self.move_selection(1, cx),
            "up" => self.move_selection(-1, cx),
            "enter" if self.selected.is_some() => self.open_selected(cx),
            "enter" => self.confirm(window, cx),
            "backspace" => self.go_up(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_breadcrumbs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mut row = h_flex()
            .flex_1()
            .min_w_0()
            .h(ui::control_h())
            .px_1()
            .gap_0p5()
            .items_center()
            .overflow_hidden()
            .border_1()
            .border_color(theme.input)
            .rounded(theme.radius)
            .bg(theme.input_background());
        if let Some(listing) = &self.listing {
            let crumbs = breadcrumbs(&listing.path, &listing.separator);
            let last = crumbs.len().saturating_sub(1);
            for (ix, (label, path)) in crumbs.into_iter().enumerate() {
                if ix > 0 {
                    row = row.child(
                        Icon::new(IconName::ChevronRight).xsmall().flex_shrink_0().text_color(theme.muted_foreground),
                    );
                }
                row = row.child(
                    Button::new(("crumb", ix))
                        .ghost()
                        .xsmall()
                        .label(label)
                        .when(ix != last, |b| b.text_color(theme.muted_foreground))
                        .when(ix == last, |b| b.font_weight(FontWeight::MEDIUM))
                        .on_click(cx.listener(move |this, _, _, cx| this.navigate(Some(path.clone()), cx))),
                );
            }
        }
        row
    }

    fn render_roots(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.listing.as_ref().map(|l| l.path.clone()).unwrap_or_default();
        let roots = self.listing.as_ref().map(|l| l.roots.clone()).unwrap_or_default();
        v_flex()
            .w(px(132.))
            .flex_shrink_0()
            .gap_0p5()
            .child(div().px_2().pb_1().child(ui::section_label("LOCATIONS", cx)))
            .children(roots.into_iter().enumerate().map(|(ix, root)| {
                let active = is_under_root(&current, &root);
                let theme = cx.theme();
                h_flex()
                    .id(("root", ix))
                    .w_full()
                    .h(px(ROW_H))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme.list_active).font_weight(FontWeight::MEDIUM))
                    .when(!active, |d| d.hover(|s| s.bg(theme.list_hover)))
                    .child(Icon::new(IconName::HardDrive).small().flex_shrink_0().text_color(if active {
                        theme.primary
                    } else {
                        theme.muted_foreground
                    }))
                    .child(div().min_w_0().truncate().child(root.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(Some(root.clone()), cx)))
            }))
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let placeholder = |msg: SharedString, color: Hsla| {
            v_flex().size_full().items_center().justify_center().text_sm().text_color(color).child(msg)
        };
        if let Some(err) = &self.error {
            return placeholder(format!("Couldn't list folder: {err}").into(), theme.danger)
                .gap_2()
                .child(
                    Button::new("retry").small().label("Retry").on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                )
                .into_any_element();
        }
        let Some(listing) = &self.listing else {
            return placeholder("Loading…".into(), theme.muted_foreground).into_any_element();
        };
        if listing.dirs.is_empty() {
            let msg = if self.loading { "Loading…" } else { "No subfolders" };
            return placeholder(msg.into(), theme.muted_foreground).into_any_element();
        }
        uniform_list(
            "dirs",
            listing.dirs.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                let Some(listing) = &this.listing else { return Vec::new() };
                let theme = cx.theme();
                range
                    .filter_map(|ix| {
                        let entry = listing.dirs.get(ix)?;
                        let selected = this.selected == Some(ix);
                        let path = entry.path.clone();
                        Some(
                            h_flex()
                                .id(("dir", ix))
                                .group("dir-row")
                                .w_full()
                                .h(px(ROW_H))
                                .px_2()
                                .gap_2()
                                .items_center()
                                .rounded(theme.radius)
                                .text_sm()
                                .when(selected, |d| d.bg(theme.list_active))
                                .when(!selected, |d| d.hover(|s| s.bg(theme.list_hover)))
                                .child(Icon::new(IconName::Folder).small().flex_shrink_0().text_color(if selected {
                                    theme.primary
                                } else {
                                    theme.muted_foreground
                                }))
                                .child(div().flex_1().min_w_0().truncate().child(entry.name.clone()))
                                .child(div().flex_shrink_0().invisible().group_hover("dir-row", |s| s.visible()).child(
                                    Icon::new(IconName::ChevronRight).xsmall().text_color(theme.muted_foreground),
                                ))
                                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                                    this.focus_handle.focus(window, cx);
                                    if ev.click_count() >= 2 {
                                        this.navigate(Some(path.clone()), cx);
                                    } else {
                                        this.selected = Some(ix);
                                        cx.notify();
                                    }
                                })),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full()
        .into_any_element()
    }
}

impl Render for RemoteFolderPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_parent = self.listing.as_ref().is_some_and(|l| l.parent.is_some());
        let chosen = self.chosen_path();
        let theme = cx.theme().clone();

        let new_folder_row =
            self.new_folder.clone().map(|input| {
                v_flex()
                    .gap(px(ui::space::TIGHT))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().child(
                                Input::new(&input).prefix(
                                    Icon::new(LucideIcon::FolderPlus).small().text_color(theme.muted_foreground),
                                ),
                            ))
                            .child(
                                Button::new("cancel-create")
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, window, cx| this.cancel_new_folder(window, cx))),
                            )
                            .child(
                                Button::new("create")
                                    .primary()
                                    .label("Create")
                                    .on_click(cx.listener(|this, _, window, cx| this.create_folder(window, cx))),
                            ),
                    )
                    .children(ui::error_banner(self.new_folder_error.as_deref(), cx))
            });

        let new_folder_button = Button::new("new-folder")
            .ghost()
            .icon(LucideIcon::FolderPlus)
            .label("New folder")
            .disabled(self.listing.is_none() || self.new_folder.is_some())
            .on_click(cx.listener(|this, _, window, cx| this.start_new_folder(window, cx)));

        v_flex()
            .key_context("RemoteFolderPicker")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .pt_2()
            .gap(px(12.))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("up")
                            .ghost()
                            .icon(LucideIcon::CornerLeftUp)
                            .tooltip("Up one level (Backspace)")
                            .disabled(!has_parent)
                            .on_click(cx.listener(|this, _, _, cx| this.go_up(cx))),
                    )
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh")
                            .loading(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(div().w_1())
                    .child(self.render_breadcrumbs(cx)),
            )
            .child(
                h_flex().h(px(10. * ROW_H + 10.)).gap_3().items_start().child(self.render_roots(cx)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .p_1()
                        .border_1()
                        .border_color(theme.border)
                        .rounded(theme.radius)
                        .child(self.render_list(cx)),
                ),
            )
            .children(new_folder_row)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Folder).xsmall().flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(chosen.clone().unwrap_or_else(|| "No folder selected".into())),
                    ),
            )
            .child(
                Footer::new("Select")
                    .disabled(chosen.is_none())
                    .leading(new_folder_button)
                    .render(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
            )
    }
}

impl Focusable for RemoteFolderPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// ---------------------------------------------------------------------------
// Pure path helpers. These operate on *server-side* paths, so they must use the
// backend's separator, never `std::path` (which is this machine's convention).
// ---------------------------------------------------------------------------

/// Splits an absolute path into clickable `(label, full_path)` segments.
///
/// `C:\Users\me` → `[("C:\", "C:\"), ("Users", "C:\Users"), ("me", "C:\Users\me")]`
/// `/home/me`    → `[("/", "/"), ("home", "/home"), ("me", "/home/me")]`
pub fn breadcrumbs(path: &str, sep: &str) -> Vec<(String, String)> {
    if path.is_empty() || sep.is_empty() {
        return Vec::new();
    }
    let (root, rest) = split_root(path, sep);
    let mut out = vec![(root.clone(), root.clone())];
    let mut acc = root;
    for part in rest.split(sep).filter(|p| !p.is_empty()) {
        acc = join_path(&acc, part, sep);
        out.push((part.to_owned(), acc.clone()));
    }
    out
}

/// Separates the root (`/`, `C:\`, `\\server\share\`) from the rest of the path.
fn split_root<'a>(path: &'a str, sep: &str) -> (String, &'a str) {
    let double = format!("{sep}{sep}");
    if sep == "\\" && path.starts_with(&double) {
        // UNC: \\server\share\ is the root.
        let body = &path[2..];
        let mut parts = body.splitn(3, sep);
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        return (format!("{double}{server}{sep}{share}{sep}"), rest);
    }
    if let Some(rest) = path.strip_prefix(sep) {
        return (sep.to_owned(), rest);
    }
    match path.find(sep) {
        Some(ix) => (path[..ix + sep.len()].to_owned(), &path[ix + sep.len()..]),
        // Bare drive like "C:".
        None => (format!("{path}{sep}"), ""),
    }
}

pub fn join_path(base: &str, name: &str, sep: &str) -> String {
    if base.ends_with(sep) { format!("{base}{name}") } else { format!("{base}{sep}{name}") }
}

/// Whether `path` lives on `root` (for highlighting the drive in the sidebar).
fn is_under_root(path: &str, root: &str) -> bool {
    !root.is_empty() && path.to_lowercase().starts_with(&root.to_lowercase())
}

pub fn validate_folder_name(raw: &str, sep: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Enter a folder name".into());
    }
    if name == "." || name == ".." {
        return Err("That name is reserved".into());
    }
    let windows = sep == "\\";
    let bad: &[char] = if windows { &['/', '\\', '<', '>', ':', '"', '|', '?', '*'] } else { &['/'] };
    if let Some(c) = name.chars().find(|c| bad.contains(c) || c.is_control()) {
        return Err(format!("Folder names can't contain “{c}”"));
    }
    if windows && name.ends_with('.') {
        return Err("Folder names can't end with a dot".into());
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{breadcrumbs, is_under_root, join_path, validate_folder_name};

    fn labels(v: &[(String, String)]) -> Vec<(&str, &str)> {
        v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
    }

    #[test]
    fn windows_breadcrumbs() {
        let c = breadcrumbs(r"C:\Users\me", "\\");
        assert_eq!(labels(&c), vec![(r"C:\", r"C:\"), ("Users", r"C:\Users"), ("me", r"C:\Users\me")]);
        assert_eq!(labels(&breadcrumbs(r"D:\", "\\")), vec![(r"D:\", r"D:\")]);
        assert_eq!(labels(&breadcrumbs("D:", "\\")), vec![(r"D:\", r"D:\")]);
        assert_eq!(labels(&breadcrumbs(r"C:\a\", "\\")), vec![(r"C:\", r"C:\"), ("a", r"C:\a")]);
    }

    #[test]
    fn unc_breadcrumbs() {
        let c = breadcrumbs(r"\\nas\media\tv", "\\");
        assert_eq!(labels(&c), vec![(r"\\nas\media\", r"\\nas\media\"), ("tv", r"\\nas\media\tv")]);
    }

    #[test]
    fn unix_breadcrumbs() {
        let c = breadcrumbs("/home/me", "/");
        assert_eq!(labels(&c), vec![("/", "/"), ("home", "/home"), ("me", "/home/me")]);
        assert_eq!(labels(&breadcrumbs("/", "/")), vec![("/", "/")]);
        assert!(breadcrumbs("", "/").is_empty());
    }

    #[test]
    fn joining() {
        assert_eq!(join_path("/", "a", "/"), "/a");
        assert_eq!(join_path("/a", "b", "/"), "/a/b");
        assert_eq!(join_path(r"C:\", "x", "\\"), r"C:\x");
        assert_eq!(join_path(r"C:\x", "y", "\\"), r"C:\x\y");
    }

    #[test]
    fn folder_names() {
        assert_eq!(validate_folder_name("  Movies ", "/"), Ok("Movies".into()));
        assert!(validate_folder_name("", "/").is_err());
        assert!(validate_folder_name("..", "/").is_err());
        assert!(validate_folder_name("a/b", "/").is_err());
        assert_eq!(validate_folder_name("a:b", "/"), Ok("a:b".into()));
        assert!(validate_folder_name("a:b", "\\").is_err());
        assert!(validate_folder_name("trailing.", "\\").is_err());
    }

    #[test]
    fn root_matching() {
        assert!(is_under_root(r"c:\users", r"C:\"));
        assert!(!is_under_root(r"D:\x", r"C:\"));
        assert!(is_under_root("/home", "/"));
    }
}
