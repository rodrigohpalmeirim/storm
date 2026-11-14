//! Storm's look: a quiet zinc palette with an iris accent, light and dark.
//!
//! The palette lives in `theme.json` (gpui-component's theme-file format); keys
//! it leaves out fall back to values derived from the ones it sets.

use std::rc::Rc;

use gpui::{App, Window};
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeSet};

const THEME_JSON: &str = include_str!("theme.json");

/// Bundled UI font. Inter's vertical metrics put cap height at the centre of
/// the line box, so text lines up with icons and backgrounds; Segoe UI's sit
/// visibly low in gpui. (OFL licensed, see assets/fonts/Inter-LICENSE.txt.)
const UI_FONT: Option<&str> = Some("Inter");
/// Subset to Latin, Greek, Cyrillic, punctuation and common symbols (other
/// scripts fall back to system fonts), and only the weights we use. Regenerate
/// with `pyftsubset`; see assets/fonts/README.md.
const UI_FONT_FILES: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
];

fn configs() -> (ThemeConfig, ThemeConfig) {
    let set: ThemeSet = serde_json::from_str(THEME_JSON).expect("theme.json is valid");
    let mut light = None;
    let mut dark = None;
    for mut config in set.themes {
        let font = std::env::var("STORM_FONT").ok().filter(|f| !f.is_empty()).or(UI_FONT.map(str::to_owned));
        if let Some(font) = font {
            config.font_family = Some(font.into());
        }
        if config.mode.is_dark() { dark = Some(config) } else { light = Some(config) }
    }
    (light.expect("theme.json has a light theme"), dark.expect("theme.json has a dark theme"))
}

/// `STORM_THEME=light|dark` pins the mode instead of following the system.
const THEME_ENV: &str = "STORM_THEME";

fn forced_mode() -> Option<ThemeMode> {
    match std::env::var(THEME_ENV).ok()?.to_ascii_lowercase().as_str() {
        "light" => Some(ThemeMode::Light),
        "dark" => Some(ThemeMode::Dark),
        _ => None,
    }
}

/// Installs the Storm light/dark themes and applies the one matching the system.
pub fn install(cx: &mut App) {
    let fonts = UI_FONT_FILES.iter().map(|b| std::borrow::Cow::Borrowed(*b)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        tracing::warn!("couldn't load the bundled UI font: {e:#}");
    }
    let (light, dark) = configs();
    let theme = Theme::global_mut(cx);
    theme.light_theme = Rc::new(light);
    theme.dark_theme = Rc::new(dark);
    match forced_mode() {
        Some(mode) => Theme::change(mode, None, cx),
        None => Theme::sync_system_appearance(None, cx),
    }
}

/// Keeps a window's theme in step with the system light/dark setting.
pub fn follow_system_appearance(window: &Window) {
    if forced_mode().is_some() {
        return;
    }
    window.observe_window_appearance(|window, cx| Theme::sync_system_appearance(Some(window), cx)).detach();
}

#[cfg(test)]
mod tests {
    #[test]
    fn theme_file_parses_both_modes() {
        let (light, dark) = super::configs();
        assert!(!light.mode.is_dark());
        assert!(dark.mode.is_dark());
        assert!(light.colors.overlay.is_some() && dark.colors.overlay.is_some());
    }
}
