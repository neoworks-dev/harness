//! File type icons: Nerd Font glyphs by file name and extension (the `devicons` crate), each in
//! its own color when the palette has true colors, else muted.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::Palette;

/// The glyph for a file at `path` and its style. A path ending in `/` is a folder.
pub fn file_icon(path: &str, p: &Palette) -> Span<'static> {
    let (glyph, rgb) = if path.ends_with('/') {
        // `devicons`' own folder icon, which it doesn't export.
        ('\u{f115}', "#7e8ea8")
    } else {
        // By name only: `devicons` looks at the disk for names it doesn't know (is it a folder?).
        let name = path.rsplit('/').next().unwrap_or(path);
        let icon = devicons::icon_for_file(devicons::File::new(name), &None);
        (icon.icon, icon.color)
    };
    // `devicons`' colors are made for a dark background; on a light one they are darkened
    // toward the text color, else a yellow or a pale orange all but disappears.
    let color = match (p.fg, hex(rgb)) {
        (Color::Rgb(..), Some(c)) if p.is_light() => {
            crate::palette::blend(c, p.fg, 45).unwrap_or(c)
        }
        (Color::Rgb(..), Some(c)) => c,
        _ => p.muted,
    };
    Span::styled(glyph.to_string(), Style::new().fg(color))
}

fn hex(s: &str) -> Option<Color> {
    let v = u32::from_str_radix(s.strip_prefix('#')?, 16).ok()?;
    Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_by_name_and_extension() {
        let p = Palette::default();
        let rs = file_icon("src/main.rs", &p);
        let toml = file_icon("Cargo.toml", &p);
        let none = file_icon("LICENSE-unknown-thing", &p);
        assert_ne!(rs.content, toml.content);
        assert_eq!(none.content, "*");
        assert_eq!(file_icon("src/", &p).content, "\u{f115}");
    }

    #[test]
    fn colors_only_with_true_colors() {
        let mut p = Palette {
            fg: Color::Rgb(200, 200, 200),
            ..Palette::default()
        };
        assert!(matches!(
            file_icon("a.rs", &p).style.fg,
            Some(Color::Rgb(..))
        ));
        // Darker on a light background.
        let on_dark = file_icon("a.js", &p).style.fg;
        let light = Palette {
            fg: Color::Rgb(0x1f, 0x23, 0x28),
            bg: Color::Rgb(244, 235, 217),
            ..p
        };
        let lum = |c: Option<Color>| match c {
            Some(Color::Rgb(r, g, b)) => u32::from(r) + u32::from(g) + u32::from(b),
            _ => 0,
        };
        assert!(lum(file_icon("a.js", &light).style.fg) < lum(on_dark));
        p.fg = Color::Reset;
        p.muted = Color::Gray;
        assert_eq!(file_icon("a.rs", &p).style.fg, Some(Color::Gray));
    }
}
