use super::{
    accent, code_color, danger, foreground, on_accent, secondary, status_bg, success, user_color,
    warning,
};
use ratatui::style::Color;

#[cfg(test)]
struct PaletteSnapshot {
    accent: Color,
    user_color: Color,
    secondary: Color,
    success: Color,
    warning: Color,
    danger: Color,
    status_bg: Color,
    code_color: Color,
    foreground: Color,
    on_accent: Color,
}

/// Gathers every palette accessor into one snapshot so the pin and contrast
/// tests cover the whole surface from one place — adding a new accessor means
/// adding it here, and the routing test then catches a colour that bypasses
/// the central switch.
#[cfg(test)]
fn theme_accessors() -> PaletteSnapshot {
    PaletteSnapshot {
        accent: accent(),
        user_color: user_color(),
        secondary: secondary(),
        success: success(),
        warning: warning(),
        danger: danger(),
        status_bg: status_bg(),
        code_color: code_color(),
        foreground: foreground(),
        on_accent: on_accent(),
    }
}

use super::{COLOR_ENABLED, Theme, decide, resolve_theme, set_theme};
use saya_config::{ColorChoice, ThemeChoice};
use std::sync::{Mutex, atomic::Ordering};

/// The palette lives in process-globals (`COLOR_ENABLED`, `ACTIVE_THEME`),
/// so tests that read or write them must run one at a time; without this
/// guard a parallel test that disables colour makes every accessor return
/// `Reset` underneath the pin tests.
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[path = "tests/color_decision.rs"]
mod color_decision;
#[path = "tests/contrast.rs"]
mod contrast;
#[path = "tests/palette.rs"]
mod palette;
