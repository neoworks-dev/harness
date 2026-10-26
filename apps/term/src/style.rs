//! Styles derived from the palette; with `NO_COLOR` only modifiers are used.

use neo_renderer_ratatui::Palette;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Styles {
    pub accent: Style,
    pub muted: Style,
    pub danger: Style,
    pub success: Style,
    pub border: Style,
    /// The palette's pane and raised backgrounds; `Reset` when the theme paints none.
    pub panel: Color,
    pub raised: Color,
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
                panel: Color::Reset,
                raised: Color::Reset,
            };
        }
        Styles {
            accent: Style::new().fg(palette.accent),
            muted: Style::new().fg(palette.muted),
            danger: Style::new().fg(palette.danger),
            success: Style::new().fg(palette.success),
            border: Style::new().fg(palette.border),
            panel: palette.panel,
            raised: palette.raised,
        }
    }

    /// Whether panes are told apart by their fills (else by borders).
    pub fn fills(&self) -> bool {
        self.panel != Color::Reset
    }
}

/// Paints `area`'s background `color`, unless that is `Reset` (no panes in this theme). Draw the
/// content after: text without a background of its own keeps this one.
pub fn fill(buf: &mut Buffer, area: Rect, color: Color) {
    if color == Color::Reset {
        return;
    }
    for y in area.top()..area.bottom().min(buf.area.bottom()) {
        for x in area.left()..area.right().min(buf.area.right()) {
            buf[(x, y)].set_bg(color);
        }
    }
}

/// A pane's left edge: a gap before its fill, or a thin border in themes without fills.
pub fn pane_edge(buf: &mut Buffer, x: u16, area: Rect, palette: &Palette) {
    if palette.panel != Color::Reset {
        return;
    }
    for y in area.top()..area.bottom() {
        buf[(x, y)]
            .set_symbol("│")
            .set_style(Style::new().fg(palette.border));
    }
}

/// The palette without colors: the terminal's own foreground and background everywhere.
pub fn plain_palette() -> Palette {
    Palette {
        fg: Color::Reset,
        bg: Color::Reset,
        surface: Color::Reset,
        panel: Color::Reset,
        raised: Color::Reset,
        border: Color::Reset,
        accent: Color::Reset,
        muted: Color::Reset,
        success: Color::Reset,
        danger: Color::Reset,
    }
}
