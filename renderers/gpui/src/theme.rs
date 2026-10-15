//! The daemon's theme tokens, resolved once into numbers the layout model uses.

use std::fmt;

use suite_protocol::{Theme, Tone};

/// The theme served by the daemon until it sends its own (`protocol/themes/default.json`).
pub fn default_theme() -> Theme {
    serde_json::from_str(include_str!("../../../protocol/themes/default.json"))
        .expect("protocol/themes/default.json is a valid theme")
}

/// A color by meaning. Layout nodes carry the token next to its value, so a snapshot shows what
/// a color is for and a theme change is a re-layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Fg,
    Bg,
    Surface,
    Border,
    Accent,
    Muted,
    Success,
    Danger,
}

impl Token {
    pub fn name(self) -> &'static str {
        match self {
            Token::Fg => "fg",
            Token::Bg => "bg",
            Token::Surface => "surface",
            Token::Border => "border",
            Token::Accent => "accent",
            Token::Muted => "muted",
            Token::Success => "success",
            Token::Danger => "danger",
        }
    }
}

impl From<Tone> for Token {
    fn from(tone: Tone) -> Token {
        match tone {
            Tone::Accent => Token::Accent,
            Tone::Muted => Token::Muted,
            Tone::Success => Token::Success,
            Tone::Danger => Token::Danger,
        }
    }
}

/// A theme color: the token and its `0xrrggbb` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paint {
    pub token: Token,
    pub rgb: u32,
}

impl fmt::Display for Paint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token.name())
    }
}

/// A text role's size, weight and whether it is monospace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: u16,
    pub mono: bool,
}

/// A [`Theme`] with parsed colors and plain numbers. Cheap to copy around.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatTheme {
    pub name: String,
    pub fg: Paint,
    pub bg: Paint,
    pub surface: Paint,
    pub border: Paint,
    pub accent: Paint,
    pub muted: Paint,
    pub success: Paint,
    pub danger: Paint,
    /// Spacing `s`, `m`, `l` in pixels.
    pub s: f32,
    pub m: f32,
    pub l: f32,
    pub title: TextStyle,
    pub body: TextStyle,
    pub code: TextStyle,
}

impl ChatTheme {
    pub fn new(theme: &Theme) -> ChatTheme {
        let c = &theme.colors;
        let style = |s: &suite_protocol::TextStyle| TextStyle {
            size: s.size.clamp(1, 200) as f32,
            weight: s.weight.clamp(1, 1000) as u16,
            mono: s.mono,
        };
        ChatTheme {
            name: theme.name.clone(),
            fg: paint(Token::Fg, &c.fg, 0xe6e6e6),
            bg: paint(Token::Bg, &c.bg, 0x16181d),
            surface: paint(Token::Surface, &c.surface, 0x1f2229),
            border: paint(Token::Border, &c.border, 0x343844),
            accent: paint(Token::Accent, &c.accent, 0x7aa2f7),
            muted: paint(Token::Muted, &c.muted, 0x8a8f9c),
            success: paint(Token::Success, &c.success, 0x9ece6a),
            danger: paint(Token::Danger, &c.danger, 0xf7768e),
            s: theme.spacing.s.clamp(0, 200) as f32,
            m: theme.spacing.m.clamp(0, 200) as f32,
            l: theme.spacing.l.clamp(0, 200) as f32,
            title: style(&theme.text.title),
            body: style(&theme.text.body),
            code: style(&theme.text.code),
        }
    }

    pub fn tone(&self, tone: Tone) -> Paint {
        match tone {
            Tone::Accent => self.accent,
            Tone::Muted => self.muted,
            Tone::Success => self.success,
            Tone::Danger => self.danger,
        }
    }

    pub fn space(&self, space: suite_protocol::Space) -> f32 {
        match space {
            suite_protocol::Space::S => self.s,
            suite_protocol::Space::M => self.m,
            suite_protocol::Space::L => self.l,
        }
    }
}

impl Default for ChatTheme {
    fn default() -> ChatTheme {
        ChatTheme::new(&default_theme())
    }
}

fn paint(token: Token, hex: &str, fallback: u32) -> Paint {
    Paint {
        token,
        rgb: parse_hex(hex).unwrap_or(fallback),
    }
}

/// `#rrggbb` or `#rgb`.
fn parse_hex(hex: &str) -> Option<u32> {
    let digits = hex.strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16).ok(),
        3 => {
            let v = u32::from_str_radix(digits, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some(((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#7aa2f7"), Some(0x7aa2f7));
        assert_eq!(parse_hex("#fa0"), Some(0xffaa00));
        assert_eq!(parse_hex("7aa2f7"), None);
        assert_eq!(parse_hex("#7aa2"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
    }

    #[test]
    fn default_theme_parses() {
        let theme = ChatTheme::default();
        assert_eq!(theme.name, "default-dark");
        assert_eq!(theme.accent.rgb, 0x7aa2f7);
        assert_eq!((theme.s, theme.m, theme.l), (4.0, 8.0, 16.0));
        assert!(theme.code.mono);
    }
}
