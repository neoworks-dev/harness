//! Command output with its colors: SGR sequences (`ESC [ … m`) read into styles, every other
//! escape sequence and control character dropped, tabs expanded.

use neo_renderer_ratatui::Palette;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

/// A color as the output named it; the palette decides the basic ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ansi {
    /// 0–7, and 8–15 for the bright ones.
    Basic(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// What the SGR sequences so far have set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Sgr {
    fg: Option<Ansi>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

/// A line of output: its text in runs of one style.
pub(crate) type Segs = Vec<(String, Sgr)>;

impl Sgr {
    /// `ESC [ params m`. Background colors are left out: on the panel's own background they
    /// mostly make a patchwork.
    fn apply(&mut self, params: &str) {
        let mut codes = params
            .split(';')
            .map(|c| c.parse::<u16>().unwrap_or(0))
            .peekable();
        if params.is_empty() {
            *self = Sgr::default();
            return;
        }
        while let Some(code) = codes.next() {
            match code {
                0 => *self = Sgr::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                22 => (self.bold, self.dim) = (false, false),
                23 => self.italic = false,
                24 => self.underline = false,
                30..=37 => self.fg = Some(Ansi::Basic((code - 30) as u8)),
                90..=97 => self.fg = Some(Ansi::Basic((code - 90 + 8) as u8)),
                39 => self.fg = None,
                38 | 48 => {
                    let color = match codes.next() {
                        Some(5) => codes.next().map(|n| Ansi::Indexed(n as u8)),
                        Some(2) => {
                            let mut c = || codes.next().unwrap_or(0) as u8;
                            Some(Ansi::Rgb(c(), c(), c()))
                        }
                        _ => None,
                    };
                    if code == 38 {
                        self.fg = color;
                    }
                }
                _ => {}
            }
        }
    }

    /// The style on top of `base` (what output without colors gets).
    pub(crate) fn style(&self, base: Style, p: &Palette) -> Style {
        let mut style = base;
        if let Some(fg) = self.fg {
            style = style.fg(match fg {
                Ansi::Basic(n) => basic(n, p),
                Ansi::Indexed(n) if n < 16 => basic(n, p),
                Ansi::Indexed(n) => Color::Indexed(n),
                Ansi::Rgb(r, g, b) => Color::Rgb(r, g, b),
            });
        }
        let mut add = Modifier::empty();
        if self.bold {
            add |= Modifier::BOLD;
        }
        if self.dim {
            add |= Modifier::DIM;
        }
        if self.italic {
            add |= Modifier::ITALIC;
        }
        if self.underline {
            add |= Modifier::UNDERLINED;
        }
        style.add_modifier(add)
    }
}

/// The 16 basic colors in the theme's: red its danger, green success, yellow warning, blue info,
/// grey and black muted, white the text; magenta and cyan as the terminal has them.
fn basic(n: u8, p: &Palette) -> Color {
    match n % 8 {
        0 => p.muted,
        1 => p.danger,
        2 => p.success,
        3 => p.warning,
        4 => p.info,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ if n == 8 => p.muted,
        _ => p.fg,
    }
}

/// One line of output (no newline in it) as styled runs; `sgr` carries what earlier lines set.
pub(crate) fn parse_line(line: &str, sgr: &mut Sgr) -> Segs {
    let mut out: Segs = Vec::new();
    let mut text = String::new();
    let mut col = 0;
    let mut chars = line.chars().peekable();
    let flush = |out: &mut Segs, text: &mut String, sgr: Sgr| {
        if !text.is_empty() {
            out.push((std::mem::take(text), sgr));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    let mut last = None;
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            last = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    if last == Some('m') {
                        flush(&mut out, &mut text, *sgr);
                        sgr.apply(&params);
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => {
                let n = 4 - col % 4;
                text.push_str(&" ".repeat(n));
                col += n;
            }
            c if c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}') =>
                {}
            c => {
                text.push(c);
                col += c.width().unwrap_or(0);
            }
        }
    }
    flush(&mut out, &mut text, *sgr);
    out
}

/// The line's text without its styles.
pub(crate) fn plain(line: &Segs) -> String {
    line.iter().map(|(t, _)| t.as_str()).collect()
}

/// The cells `from..from + width` of the line as spans; true when some of it is past them.
pub(crate) fn window(
    line: &Segs,
    from: usize,
    width: usize,
    base: Style,
    p: &Palette,
) -> (Vec<Span<'static>>, bool) {
    let mut spans = Vec::new();
    let mut col = 0;
    let mut cut = false;
    for (text, sgr) in line {
        let mut part = String::new();
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if col >= from && col + w <= from + width {
                part.push(c);
            } else if col + w > from + width {
                cut = true;
            }
            col += w;
        }
        if !part.is_empty() {
            spans.push(Span::styled(part, sgr.style(base, p)));
        }
        if cut {
            break;
        }
    }
    (spans, cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(line: &str) -> Segs {
        parse_line(line, &mut Sgr::default())
    }

    #[test]
    fn colors_are_kept_and_other_escapes_dropped() {
        let line = segs("\u{1b}[1;31merror\u{1b}[0m: \u{1b}]8;;http://x\u{7}link\u{1b}[2K done");
        assert_eq!(plain(&line), "error: link done");
        let p = Palette::default();
        let red = line[0].1.style(Style::new(), &p);
        assert_eq!(red.fg, Some(p.danger));
        assert!(red.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line[1].1, Sgr::default());
    }

    #[test]
    fn a_color_goes_on_into_the_next_line_until_reset() {
        let mut sgr = Sgr::default();
        parse_line("\u{1b}[32mok", &mut sgr);
        let next = parse_line("still green\u{1b}[39m", &mut sgr);
        assert_eq!(next[0].1.fg, Some(Ansi::Basic(2)));
        assert_eq!(sgr.fg, None);
        let rgb = segs("\u{1b}[38;2;1;2;3mx\u{1b}[38;5;200my");
        assert_eq!(rgb[0].1.fg, Some(Ansi::Rgb(1, 2, 3)));
        assert_eq!(rgb[1].1.fg, Some(Ansi::Indexed(200)));
    }
}
