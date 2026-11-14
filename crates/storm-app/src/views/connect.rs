//! "Connect to server…" dialog: address + token, verified before switching.

use std::rc::Rc;
use std::sync::Arc;

use gpui::*;
use gpui_kit::assets::IconName as LucideIcon;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, v_flex};
use storm_client::RemoteBackend;
use storm_proto::Backend;

use crate::remotes::{self, RemotePrefs};
use crate::ui::{self, Footer, form_row};

pub type OnConnected = Rc<dyn Fn(Arc<dyn Backend>, &mut Window, &mut App)>;

pub struct ConnectDialog {
    url: Entity<InputState>,
    token: Entity<InputState>,
    remember: bool,
    busy: bool,
    error: Option<String>,
    on_connected: OnConnected,
    _subs: Vec<Subscription>,
}

impl ConnectDialog {
    pub fn open(on_connected: OnConnected, window: &mut Window, cx: &mut App) {
        let prefs = remotes::load().unwrap_or_default();
        let dialog = cx.new(|cx| {
            let url = cx.new(|cx| {
                let mut s = InputState::new(window, cx)
                    .placeholder(format!("nas.local  (port {} by default)", storm_proto::DEFAULT_PORT));
                s.set_value(prefs.url.clone(), window, cx);
                s
            });
            let token = cx.new(|cx| {
                let mut s = InputState::new(window, cx).masked(true).placeholder("Printed by stormd on startup");
                s.set_value(prefs.token.clone(), window, cx);
                s
            });
            // Enter in either field connects.
            let subs = [&url, &token]
                .into_iter()
                .map(|input| {
                    cx.subscribe_in(input, window, |this: &mut Self, _, ev: &InputEvent, window, cx| {
                        if matches!(ev, InputEvent::PressEnter { .. }) {
                            this.connect(window, cx);
                        }
                    })
                })
                .collect();
            Self { url, token, remember: true, busy: false, error: None, on_connected, _subs: subs }
        });
        let focus = dialog.read(cx).url.clone();
        window.open_dialog(cx, move |d, _, cx| {
            let header = ui::dialog_header(
                LucideIcon::Server,
                "Connect to a server",
                Some("Control a Storm daemon (stormd) running elsewhere".into()),
                None,
                cx,
            );
            ui::modal(d, header, ui::width::MD, cx).child(dialog.clone())
        });
        focus.update(cx, |s, cx| s.focus(window, cx));
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let url = self.url.read(cx).value().trim().to_string();
        let token = self.token.read(cx).value().trim().to_string();
        if url.is_empty() || token.is_empty() {
            self.error = Some("Enter both the server address and its token.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let remember = self.remember;
        cx.spawn_in(window, async move |this, cx| {
            let result = RemoteBackend::connect(&url, &token).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(remote) => {
                        if remember {
                            remotes::save(&RemotePrefs { url: remote.url().to_string(), token });
                        } else {
                            remotes::forget();
                        }
                        window.close_dialog(cx);
                        (this.on_connected)(Arc::new(remote), window, cx);
                    }
                    Err(e) => {
                        this.error = Some(e.to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for ConnectDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let remember = Checkbox::new("remember").label("Remember this server").checked(self.remember).on_click(
            cx.listener(|this, checked: &bool, _, cx| {
                this.remember = *checked;
                cx.notify();
            }),
        );
        v_flex()
            .pt_2()
            .gap(px(ui::space::ROW))
            .child(form_row(
                "Server address",
                Input::new(&self.url).prefix(Icon::new(LucideIcon::Globe).small().text_color(muted)),
                Some("e.g. nas.local, 192.168.1.20:9870 or https://torrents.example.com".into()),
                cx,
            ))
            .child(form_row(
                "Token",
                Input::new(&self.token).prefix(Icon::new(LucideIcon::KeyRound).small().text_color(muted)).mask_toggle(),
                None,
                cx,
            ))
            .children(ui::error_banner(self.error.as_deref(), cx))
            .child(
                Footer::new("Connect")
                    .busy(self.busy)
                    .leading(remember)
                    .render(cx.listener(|this, _, window, cx| this.connect(window, cx))),
            )
    }
}
