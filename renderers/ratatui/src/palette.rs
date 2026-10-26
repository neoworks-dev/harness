//! The daemon's theme colors as ratatui colors.

use neo_protocol::{Theme, Tone};
use ratatui::style::{Color, Style};

/// The theme's colors by meaning, as true-color ratatui colors. Fields are public: an app that
/// wants 16/256 colors or none at all (`NO_COLOR`) maps or replaces them before rendering.
///
/// Ordinary text is drawn without a foreground color, so the terminal's own is used; `fg` and
/// `surface` only meet where text sits on a painted background (code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub fg: Color,
    pub bg: Color,
    /// Code backgrounds.
    pub surface: Color,
    /// Panes beside the chat (sidebars, a side column): a step off the background. `Reset`
    /// (the protocol's themes) paints none; the panes are told apart by borders then.
    pub panel: Color,
    /// Raised boxes, like the input: a step further.
    pub raised: Color,
    pub border: Color,
    pub accent: Color,
    pub muted: Color,
    pub success: Color,
    pub danger: Color,
}

impl Palette {
    /// The colors of a protocol theme; a color that isn't `#rrggbb` / `#rgb` keeps the default.
    pub fn from_theme(theme: &Theme) -> Palette {
        let c = &theme.colors;
        let d = Palette::default();
        Palette {
            fg: hex(&c.fg).unwrap_or(d.fg),
            bg: hex(&c.bg).unwrap_or(d.bg),
            surface: hex(&c.surface).unwrap_or(d.surface),
            panel: d.panel,
            raised: d.raised,
            border: hex(&c.border).unwrap_or(d.border),
            accent: hex(&c.accent).unwrap_or(d.accent),
            muted: hex(&c.muted).unwrap_or(d.muted),
            success: hex(&c.success).unwrap_or(d.success),
            danger: hex(&c.danger).unwrap_or(d.danger),
        }
    }

    /// The color a tone stands for.
    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Accent => self.accent,
            Tone::Muted => self.muted,
            Tone::Success => self.success,
            Tone::Danger => self.danger,
        }
    }

    pub(crate) fn fg(&self, color: Color) -> Style {
        Style::new().fg(color)
    }

    pub(crate) fn muted_style(&self) -> Style {
        Style::new().fg(self.muted)
    }

    /// Whether the background is a light one; false without a true color.
    pub fn is_light(&self) -> bool {
        let Color::Rgb(r, g, b) = self.bg else {
            return false;
        };
        u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 >= 128_000
    }

    /// The background of the user's messages: the text color mixed into the background, so the
    /// band reads in any theme (the surface color is often too close to the background). Themes
    /// without true colors get the surface color.
    pub fn band(&self) -> Color {
        blend(self.bg, self.fg, 14).unwrap_or(self.surface)
    }

    /// A chip's background: its color mixed into the background, the band's without one. Themes
    /// without true colors get the surface color.
    pub(crate) fn tint(&self, color: Option<Color>) -> Color {
        match color {
            Some(c) => blend(self.bg, c, 22).unwrap_or(self.surface),
            None => self.band(),
        }
    }

    /// Text on the surface color: code.
    pub(crate) fn code_style(&self) -> Style {
        Style::new().fg(self.fg).bg(self.surface)
    }
}

/// `over` mixed into `base` by `pct` percent; None unless both are true colors.
pub(crate) fn blend(base: Color, over: Color, pct: u32) -> Option<Color> {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (base, over) else {
        return None;
    };
    let mix = |a: u8, b: u8| ((u32::from(a) * (100 - pct) + u32::from(b) * pct) / 100) as u8;
    Some(Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1)))
}

/// `protocol/themes/default.json` (a test keeps the two in step).
impl Default for Palette {
    fn default() -> Palette {
        Palette {
            fg: Color::Rgb(0xe6, 0xe6, 0xe6),
            bg: Color::Rgb(0x16, 0x18, 0x1d),
            surface: Color::Rgb(0x1f, 0x22, 0x29),
            // The protocol's themes have neither.
            panel: Color::Reset,
            raised: Color::Reset,
            border: Color::Rgb(0x34, 0x38, 0x44),
            accent: Color::Rgb(0x7a, 0xa2, 0xf7),
            muted: Color::Rgb(0x8a, 0x8f, 0x9c),
            success: Color::Rgb(0x9e, 0xce, 0x6a),
            danger: Color::Rgb(0xf7, 0x76, 0x8e),
        }
    }
}

/// `#rrggbb` or `#rgb`.
fn hex(s: &str) -> Option<Color> {
    let digits = s.strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)),
        3 => {
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some(Color::Rgb(
                (r * 0x11) as u8,
                (g * 0x11) as u8,
                (b * 0x11) as u8,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(hex("#7aa2f7"), Some(Color::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(hex("#fa0"), Some(Color::Rgb(0xff, 0xaa, 0x00)));
        assert_eq!(hex("7aa2f7"), None);
        assert_eq!(hex("#7aa2"), None);
        assert_eq!(hex("#zzzzzz"), None);
        assert_eq!(hex("#+12345"), None);
    }

    #[test]
    fn default_matches_the_default_theme() {
        let theme: Theme =
            serde_json::from_str(include_str!("../../../protocol/themes/default.json")).unwrap();
        assert_eq!(Palette::from_theme(&theme), Palette::default());
    }

    #[test]
    fn bad_colors_keep_the_default() {
        let mut theme: Theme =
            serde_json::from_str(include_str!("../../../protocol/themes/default.json")).unwrap();
        theme.colors.accent = "blue".into();
        theme.colors.danger = "#ff0000".into();
        let p = Palette::from_theme(&theme);
        assert_eq!(p.accent, Palette::default().accent);
        assert_eq!(p.danger, Color::Rgb(255, 0, 0));
    }
}
