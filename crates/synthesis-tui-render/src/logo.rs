//! The brand symbol as braille art (table in `logo_art.rs`, generated offline by
//! `tools/logo-art/gen.py`) with a per-cell sheen: dark cells `muted`, then `body`, `accent`,
//! `accent_light` and `pearl` for the lightest. Mono and 16-colour themes draw one colour.

use crate::logo_art::{ART, COLS, ROWS, SHADE, SHADES};
use crate::theme::{Rgb, ThemeId, lerp, t};

pub const ART_COLS: usize = COLS;
pub const ART_ROWS: usize = ROWS;

fn sheen(shade: u8) -> Rgb {
    let th = t();
    if matches!(th.id, ThemeId::Mono | ThemeId::Ansi16) {
        return th.accent;
    }
    let stops = [th.muted, th.body, th.accent, th.accent_light, th.pearl];
    let pos = f64::from(shade) / f64::from(SHADES - 1) * (stops.len() - 1) as f64;
    let i = (pos.floor() as usize).min(stops.len() - 2);
    lerp(stops[i], stops[i + 1], pos - i as f64)
}

/// `ROWS` rows of `COLS` cells: the character (a plain space where nothing is drawn) and its colour.
pub fn art_cells() -> Vec<Vec<(char, Rgb)>> {
    ART.iter()
        .zip(SHADE.iter())
        .map(|(line, shades)| {
            line.chars()
                .zip(shades)
                .map(|(c, s)| (if c == '\u{2800}' { ' ' } else { c }, sheen(*s)))
                .collect()
        })
        .collect()
}
