//! Bottom status bar: connection indicator on the left; counts, a speed
//! sparkline and the global speeds on the right. Session/lifetime totals live
//! in the speeds' tooltip to keep the bar quiet.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_kit::component::chart::AreaChart;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex};
use storm_proto::StatsSample;

use crate::format;
use crate::state::AppState;

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// How much of the history the sparkline shows.
const SPARK_SECS: i64 = 2 * 60;

pub struct StatusBar {
    state: Entity<AppState>,
}

impl StatusBar {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // Which server we're on (if any) lives in the title bar.
        let state = self.state.read(cx);
        let s = &state.session;
        let newest = state.history.back().map(|s| s.ts).unwrap_or_default();
        let samples: Vec<StatsSample> = state.history.iter().filter(|x| x.ts >= newest - SPARK_SECS).copied().collect();
        let peak = samples.iter().map(|s| s.down_bps.max(s.up_bps)).max().unwrap_or(0).max(16 * 1024);
        let down_color = theme.primary;
        let up_color = theme.success;
        let limits = state.settings.as_ref().map(|st| (st.down_limit_bps, st.up_limit_bps)).unwrap_or_default();
        let totals = format!(
            "This session: ↓ {}  ↑ {}\nAll time: ↓ {}  ↑ {}",
            format::bytes(s.session_downloaded),
            format::bytes(s.session_uploaded),
            format::bytes(s.total_downloaded),
            format::bytes(s.total_uploaded)
        );
        let tnum = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));

        let speed = |icon: IconName, color: Hsla, bps: u64, limit: Option<u32>| {
            h_flex()
                .gap_1()
                .child(Icon::new(icon).size(px(12.)).text_color(color))
                .child(
                    div()
                        .min_w(px(68.))
                        .font_features(tnum.clone())
                        .text_color(if bps > 0 { theme.foreground } else { theme.muted_foreground })
                        .child(format::speed_always(bps)),
                )
                .when_some(limit.filter(|l| *l > 0), |d, l| {
                    d.child(
                        div().text_color(theme.muted_foreground).child(format!("/ {}", format::speed_always(l as u64))),
                    )
                })
        };

        h_flex()
            .h(px(28.))
            .flex_shrink_0()
            .px_3()
            .gap_4()
            .text_xs()
            .bg(theme.background)
            .border_t_1()
            .border_color(theme.border.opacity(0.6))
            .text_color(theme.muted_foreground)
            .child(div().flex_1())
            .child(format!("{} · {}", plural(state.torrents.len(), "torrent"), plural(s.peers_live as usize, "peer")))
            .child(div().w(px(1.)).h(px(12.)).bg(theme.border))
            .child(div().w(px(96.)).h(px(18.)).when(
                samples.len() >= 2 && samples.iter().any(|x| x.down_bps + x.up_bps > 0),
                |this| {
                    this.child(
                        AreaChart::new(samples)
                            .id("speed-chart")
                            .x(|_| SharedString::default())
                            .y(|s: &StatsSample| s.down_bps as f64)
                            .stroke(down_color)
                            .fill(down_color.opacity(0.18))
                            .y(|s: &StatsSample| s.up_bps as f64)
                            .stroke(up_color)
                            .fill(up_color.opacity(0.12))
                            .linear()
                            .x_axis(false)
                            .y_axis(false)
                            .grid(false)
                            .appear(false)
                            .y_domain(0.0, peak as f64 * 1.1)
                            .tooltip_title(|s: &StatsSample| format::date(Some(s.ts)).into())
                            .tooltip_value(|_, series, v| {
                                let dir = if series == 0 { "↓" } else { "↑" };
                                format!("{dir} {}", format::speed_always(v as u64)).into()
                            }),
                    )
                },
            ))
            .child(
                h_flex()
                    .id("speeds")
                    .gap_3()
                    .child(speed(IconName::ArrowDown, down_color, s.down_bps, limits.0))
                    .child(speed(IconName::ArrowUp, up_color, s.up_bps, limits.1))
                    .tooltip(move |window, cx| Tooltip::new(totals.clone()).build(window, cx)),
            )
    }
}
