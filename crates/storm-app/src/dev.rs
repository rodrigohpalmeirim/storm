//! Development aid: `STORM_DEV_DIALOG=<name>` opens a dialog at startup so its
//! look can be checked (and screenshotted) without driving the UI.
//!
//! Names: `add`, `files`, `remove`, `settings`, `connect`, `picker`.

use std::rc::Rc;

use gpui::{AnyWindowHandle, App};

use crate::caps::{BackendHandle, pick_backend_folder};
use crate::ui;
use crate::views::connect::ConnectDialog;
use crate::views::dialogs::{AddTorrentDialog, RemoveDialog, SettingsDialog};

const DEV_DIALOG_ENV: &str = "STORM_DEV_DIALOG";

pub fn open_requested_dialog(window: AnyWindowHandle, cx: &mut App) {
    let Some(name) = std::env::var(DEV_DIALOG_ENV).ok().filter(|n| !n.is_empty()) else { return };
    let _ = window.update(cx, |_, window, cx| match name.as_str() {
        "add" => AddTorrentDialog::open_links(None, None, window, cx),
        "files" => AddTorrentDialog::open_files(
            vec![
                ("Big Buck Bunny.torrent".into(), vec![0; 2_340]),
                ("Sintel (2010) 4K.torrent".into(), vec![0; 18_200]),
                ("ubuntu-24.04-desktop-amd64.iso.torrent".into(), vec![0; 61_500]),
            ],
            None,
            window,
            cx,
        ),
        "remove" => RemoveDialog::open(
            vec![1, 2, 3],
            vec!["Big Buck Bunny".into(), "Sintel".into(), "Tears of Steel".into()],
            false,
            window,
            cx,
        ),
        "remove1" => RemoveDialog::open(vec![1], vec!["Big Buck Bunny (2008) 1080p".into()], true, window, cx),
        "connect" => ConnectDialog::open(Rc::new(|_, _, _| {}), window, cx),
        "picker" => pick_backend_folder("Save torrent to", None, window, cx, |_, _, _| {}),
        "settings" => {
            let backend = BackendHandle::get(cx).backend;
            ui::spawn_op(window, cx, "Loading settings", async move { backend.settings().await }, |s, window, cx| {
                SettingsDialog::open(s, window, cx)
            });
        }
        other => tracing::warn!("{DEV_DIALOG_ENV}: unknown dialog {other:?}"),
    });
}
