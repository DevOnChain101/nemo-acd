//! `synthesis.tui` of the USER settings file (spec docs/specs/tui-design.md §7): the theme and the
//! glyph set. Project, local and managed files never set it (`synthesis-settings` reports that).
//! `/config` and the other keys come with phase P2.

use serde_json::Value;
use std::path::Path;
use synthesis_config::Env;
use synthesis_tui_base::sgr::ColorMode;
use synthesis_tui_render::theme::{RenderConfig, ThemeId, select_theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphChoice {
    #[default]
    Auto,
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuiSettings {
    pub theme: ThemeId,
    pub glyphs: GlyphChoice,
}

impl Default for TuiSettings {
    fn default() -> Self {
        Self {
            theme: ThemeId::Graphite,
            glyphs: GlyphChoice::Auto,
        }
    }
}

impl TuiSettings {
    /// Reads `synthesis.tui` of a parsed settings file. A wrong type or an unknown value keeps the
    /// default and yields a diagnostic that names the key, never the value.
    pub fn from_settings(v: &Value) -> (Self, Vec<String>) {
        let mut s = Self::default();
        let mut diags = vec![];
        let Some(tui) = v.get("synthesis").and_then(|n| n.get("tui")) else {
            return (s, diags);
        };
        let Some(tui) = tui.as_object() else {
            diags.push("synthesis.tui is not an object, ignored".into());
            return (s, diags);
        };
        match tui
            .get("theme")
            .map(|t| t.as_str().and_then(ThemeId::parse))
        {
            Some(Some(t)) => s.theme = t,
            Some(None) => {
                diags.push("synthesis.tui.theme is not a known theme, using graphite".into())
            }
            None => {}
        }
        match tui.get("glyphs").map(Value::as_str) {
            Some(Some("auto")) | None => {}
            Some(Some("unicode")) => s.glyphs = GlyphChoice::Unicode,
            Some(Some("ascii")) => s.glyphs = GlyphChoice::Ascii,
            Some(_) => {
                diags.push("synthesis.tui.glyphs must be auto, unicode or ascii, using auto".into())
            }
        }
        (s, diags)
    }

    /// `<synthesis_home>/settings.json`. A missing file is the defaults; an unreadable one is the
    /// defaults plus a diagnostic (the file itself is never touched).
    pub fn load_user(home: &Path) -> (Self, Vec<String>) {
        let path = home.join("settings.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return (Self::default(), vec![]);
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => Self::from_settings(&v),
            Err(_) => (
                Self::default(),
                vec!["settings.json is unreadable; using the default theme".into()],
            ),
        }
    }

    /// The theme and glyph set for this terminal: `NO_COLOR` and 16 colours override the theme;
    /// `SYNTHESIS_ASCII` and a basic `TERM` choose ASCII glyphs unless the user said otherwise.
    pub fn render_config(&self, mode: ColorMode, env: &Env) -> RenderConfig {
        let ascii = match self.glyphs {
            GlyphChoice::Ascii => true,
            GlyphChoice::Unicode => false,
            GlyphChoice::Auto => match env.get_nonempty("SYNTHESIS_ASCII") {
                Some("1" | "true") => true,
                Some("0" | "false") => false,
                _ => matches!(
                    env.get("TERM"),
                    Some("linux" | "vt100" | "vt220" | "cons25" | "dumb")
                ),
            },
        };
        RenderConfig {
            theme: select_theme(mode, self.theme),
            ascii,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    }

    #[test]
    fn theme_and_glyphs_are_read_and_bad_values_keep_defaults() {
        let (s, d) = TuiSettings::from_settings(
            &json!({"synthesis": {"tui": {"theme": "pearl", "glyphs": "ascii"}}}),
        );
        assert_eq!(
            (s.theme, s.glyphs, d.len()),
            (ThemeId::Pearl, GlyphChoice::Ascii, 0)
        );
        let (s, d) = TuiSettings::from_settings(
            &json!({"synthesis": {"tui": {"theme": "neon", "glyphs": 3}}}),
        );
        assert_eq!(s, TuiSettings::default());
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|m| !m.contains("neon")), "{d:?}");
        assert_eq!(TuiSettings::from_settings(&json!({})).1.len(), 0);
    }

    #[test]
    fn no_color_and_16_colours_override_the_theme_and_ascii_follows_the_env() {
        let s = TuiSettings {
            theme: ThemeId::Midnight,
            glyphs: GlyphChoice::Auto,
        };
        let e = env(&[]);
        assert_eq!(
            s.render_config(ColorMode::TrueColor, &e).theme,
            ThemeId::Midnight
        );
        assert_eq!(s.render_config(ColorMode::Mono, &e).theme, ThemeId::Mono);
        assert_eq!(
            s.render_config(ColorMode::Ansi16, &e).theme,
            ThemeId::Ansi16
        );
        assert!(!s.render_config(ColorMode::TrueColor, &e).ascii);
        assert!(
            s.render_config(ColorMode::TrueColor, &env(&[("SYNTHESIS_ASCII", "1")]))
                .ascii
        );
        assert!(
            s.render_config(ColorMode::TrueColor, &env(&[("TERM", "linux")]))
                .ascii
        );
        assert!(
            s.render_config(ColorMode::TrueColor, &env(&[("TERM", "dumb")]))
                .ascii
        );
        assert!(
            !s.render_config(
                ColorMode::TrueColor,
                &env(&[("TERM", "linux"), ("SYNTHESIS_ASCII", "0")])
            )
            .ascii
        );
    }
}
