//! The app's colors (`/theme`): `dark` paints the screen near-black, `light` cream, `terminal` leaves
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

    /// The colors to draw with. Dark is the daemon's theme on a near-black background (code a
    /// shade lighter, so it still stands out) with a green accent; light has colors of its own,
    /// as the daemon's are made for a dark background.
    pub fn palette(self, daemon: &Palette) -> Palette {
        match self {
            Theme::Dark => Palette {
                bg: Color::Rgb(28, 29, 32),
                surface: Color::Rgb(38, 40, 45),
                accent: Color::Rgb(92, 191, 140),
                ..*daemon
            },
            Theme::Light => LIGHT,
            Theme::Terminal => *daemon,
        }
    }

    /// Whether the background and text color are painted (not the terminal's).
    pub fn paints(self) -> bool {
        self != Theme::Terminal
    }
}

const LIGHT: Palette = Palette {
    fg: Color::Rgb(0x1f, 0x23, 0x28),
    bg: Color::Rgb(244, 235, 217),
    // A shade darker than the background, for code.
    surface: Color::Rgb(232, 221, 199),
    border: Color::Rgb(206, 193, 168),
    accent: Color::Rgb(0, 0, 0),
    muted: Color::Rgb(0x6b, 0x66, 0x5c),
    success: Color::Rgb(0x1a, 0x7f, 0x37),
    danger: Color::Rgb(0xcf, 0x22, 0x2e),
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

/// Where the theme is kept: `{"theme": "light"}`. No file: nothing is kept (tests).
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

    /// The kept theme; dark when there is none or the file can't be read.
    pub fn theme(&self) -> Theme {
        self.file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| v.get("theme")?.as_str().and_then(Theme::parse))
            .unwrap_or_default()
    }

    pub fn set_theme(&self, theme: Theme) -> std::io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::json!({ "theme": theme.name() }).to_string();
        std::fs::write(file, format!("{text}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    #[test]
    fn names_round_trip_and_cycle() {
        for t in Theme::ALL {
            assert_eq!(Theme::parse(t.name()), Some(t));
        }
        assert_eq!(Theme::parse("solarized"), None);
        assert_eq!(Theme::Dark.next().next().next(), Theme::Dark);
    }

    #[test]
    fn painting_fills_only_the_default_colors() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(1, 0)].set_style(Style::new().fg(Color::Red).bg(Color::Blue));
        let p = Theme::Light.palette(&Palette::default());
        paint(&mut buf, &p);
        assert_eq!((buf[(0, 0)].fg, buf[(0, 0)].bg), (p.fg, p.bg));
        assert_eq!((buf[(1, 0)].fg, buf[(1, 0)].bg), (Color::Red, Color::Blue));
    }

    #[test]
    fn the_theme_is_kept_in_a_file() {
        let dir = std::env::temp_dir().join(format!("neo-term-prefs-{}", std::process::id()));
        let prefs = Prefs::new(Some(dir.join("term.json")));
        assert_eq!(prefs.theme(), Theme::Dark);
        prefs.set_theme(Theme::Light).unwrap();
        assert_eq!(prefs.theme(), Theme::Light);
        std::fs::write(dir.join("term.json"), "not json").unwrap();
        assert_eq!(prefs.theme(), Theme::Dark);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
