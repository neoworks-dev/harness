//! The app's colors (`/theme`): `dark` paints the screen near-black, `light` off-white, `terminal` leaves
//! the terminal's own background and text color. The choice is kept in `<data dir>/term.json`.
//!
//! Painting happens once a frame is drawn: every cell still on the terminal's default color gets
//! the theme's. Widgets (and `Clear`, which resets cells to the default) need not know.

use std::path::PathBuf;

use neo_renderer_ratatui::Palette;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Terminal,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::Terminal];

    pub fn parse(name: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Terminal => "terminal",
        }
    }

    /// The one after this, for `/theme` without a name.
    pub fn next(self) -> Theme {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Terminal,
            Theme::Terminal => Theme::Dark,
        }
    }

    /// The colors to draw with: dark and light are the Neoworks design tokens
    /// (`@neoworks-dev/ui`), terminal the daemon's theme.
    pub fn palette(self, daemon: &Palette) -> Palette {
        match self {
            Theme::Dark => DARK,
            Theme::Light => LIGHT,
            // Without panes: they would be painted on the terminal's own background.
            Theme::Terminal => *daemon,
        }
    }

    /// Whether the background and text color are painted (not the terminal's).
    pub fn paints(self) -> bool {
        self != Theme::Terminal
    }
}

/// The tokens' dark roles: canvas, text, text-muted, sunken (code), surface (panes), hover (the
/// input), line, success, danger, warning, info. The accent is pine-11, not the accent role's
/// pine-9: that one is made to be filled, and as text on the canvas it is about 3:1.
const DARK: Palette = Palette {
    fg: Color::Rgb(0xf1, 0xf0, 0xed),
    bg: Color::Rgb(0x0f, 0x0e, 0x0c),
    surface: Color::Rgb(0x1e, 0x1d, 0x1b),
    panel: Color::Rgb(0x15, 0x15, 0x13),
    raised: Color::Rgb(0x25, 0x25, 0x22),
    border: Color::Rgb(0x36, 0x35, 0x33),
    accent: Color::Rgb(0x9f, 0xc9, 0xb1),
    muted: Color::Rgb(0xbb, 0xbb, 0xb8),
    success: Color::Rgb(0x9f, 0xc9, 0xb1),
    danger: Color::Rgb(0xff, 0xa1, 0x89),
    warning: Color::Rgb(0xe9, 0xb2, 0x6d),
    info: Color::Rgb(0x97, 0xc1, 0xf6),
};

/// The tokens' light roles, the same ones but the panes: those are sunken, as the raised
/// surface is the input.
const LIGHT: Palette = Palette {
    fg: Color::Rgb(0x17, 0x17, 0x15),
    bg: Color::Rgb(0xf2, 0xf1, 0xee),
    surface: Color::Rgb(0xe9, 0xe8, 0xe6),
    panel: Color::Rgb(0xe9, 0xe8, 0xe6),
    raised: Color::Rgb(0xfd, 0xfd, 0xfc),
    border: Color::Rgb(0xd0, 0xcf, 0xcc),
    accent: Color::Rgb(0x34, 0x67, 0x4d),
    muted: Color::Rgb(0x64, 0x63, 0x61),
    success: Color::Rgb(0x22, 0x4f, 0x39),
    danger: Color::Rgb(0x97, 0x37, 0x1f),
    warning: Color::Rgb(0x84, 0x4c, 0x00),
    info: Color::Rgb(0x31, 0x5c, 0x91),
};

/// Gives every cell left on the terminal's default colors the palette's.
pub fn paint(buf: &mut Buffer, palette: &Palette) {
    for cell in &mut buf.content {
        if cell.bg == Color::Reset {
            cell.bg = palette.bg;
        }
        if cell.fg == Color::Reset {
            cell.fg = palette.fg;
        }
    }
}

/// Where the theme, the marked sessions and whether the setup walkthrough was done are kept:
/// `{"theme": "light", "marks": […], "setup": true}`. No file: nothing is kept (tests).
#[derive(Debug, Clone, Default)]
pub struct Prefs {
    file: Option<PathBuf>,
}

impl Prefs {
    pub fn new(file: Option<PathBuf>) -> Prefs {
        Prefs { file }
    }

    /// `<data dir>/term.json`.
    pub fn in_data_dir() -> Prefs {
        Prefs::new(crate::util::data_dir().map(|d| d.join("term.json")))
    }

    /// What the file holds: an object, else an empty one.
    fn read(&self) -> serde_json::Map<String, serde_json::Value> {
        self.file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    /// Sets one key, keeping the others.
    fn write(&self, key: &str, value: impl Into<serde_json::Value>) -> std::io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut all = self.read();
        all.insert(key.into(), value.into());
        let text = serde_json::Value::Object(all).to_string();
        std::fs::write(file, format!("{text}\n"))
    }

    /// The kept theme; dark when there is none or the file can't be read.
    pub fn theme(&self) -> Theme {
        self.read()
            .get("theme")
            .and_then(|v| v.as_str())
            .and_then(Theme::parse)
            .unwrap_or_default()
    }

    pub fn set_theme(&self, theme: Theme) -> std::io::Result<()> {
        self.write("theme", theme.name())
    }

    /// The marked sessions (Alt-m), slot 1 first: `{"marks": ["id", …]}`.
    pub fn marks(&self) -> Vec<String> {
        self.read()
            .get("marks")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_marks(&self, marks: &[String]) -> std::io::Result<()> {
        self.write("marks", marks.to_vec())
    }

    /// The setup walkthrough was run through or left on this machine: `{"setup": true}`.
    pub fn setup_done(&self) -> bool {
        self.read().get("setup").and_then(|v| v.as_bool()) == Some(true)
    }

    pub fn set_setup_done(&self) -> std::io::Result<()> {
        self.write("setup", true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_cycle() {
        for t in Theme::ALL {
            assert_eq!(Theme::parse(t.name()), Some(t));
        }
        assert_eq!(Theme::parse("solarized"), None);
        assert_eq!(Theme::Dark.next().next().next(), Theme::Dark);
    }

    #[test]
    fn the_theme_is_kept_in_a_file() {
        let dir = std::env::temp_dir().join(format!("neo-term-prefs-{}", std::process::id()));
        let prefs = Prefs::new(Some(dir.join("term.json")));
        assert_eq!(prefs.theme(), Theme::Dark);
        prefs.set_theme(Theme::Light).unwrap();
        assert_eq!(prefs.theme(), Theme::Light);
        prefs.set_marks(&["s2".into(), "s1".into()]).unwrap();
        assert_eq!(prefs.marks(), ["s2", "s1"]);
        assert_eq!(prefs.theme(), Theme::Light, "the other keys stay");
        std::fs::write(dir.join("term.json"), "not json").unwrap();
        assert_eq!(prefs.theme(), Theme::Dark);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
