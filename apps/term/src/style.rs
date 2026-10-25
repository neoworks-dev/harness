//! Styles derived from the palette; with `NO_COLOR` only modifiers are used.

use neo_renderer_ratatui::Palette;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Styles {
    pub accent: Style,
    pub muted: Style,
    pub danger: Style,
    pub success: Style,
    pub border: Style,
}

impl Styles {
    pub fn new(palette: &Palette, no_color: bool) -> Styles {
        if no_color {
            return Styles {
                accent: Style::new().add_modifier(Modifier::BOLD),
                muted: Style::new().add_modifier(Modifier::DIM),
                danger: Style::new().add_modifier(Modifier::BOLD),
                success: Style::new(),
                border: Style::new().add_modifier(Modifier::DIM),
            };
        }
        Styles {
            accent: Style::new().fg(palette.accent),
            muted: Style::new().fg(palette.muted),
            danger: Style::new().fg(palette.danger),
            success: Style::new().fg(palette.success),
            border: Style::new().fg(palette.border),
        }
    }
}

/// The palette without colors: the terminal's own foreground and background everywhere.
pub fn plain_palette() -> Palette {
    Palette {
        fg: Color::Reset,
        bg: Color::Reset,
        surface: Color::Reset,
        border: Color::Reset,
        accent: Color::Reset,
        muted: Color::Reset,
        success: Color::Reset,
        danger: Color::Reset,
    }
}
