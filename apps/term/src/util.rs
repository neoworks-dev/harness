//! Small helpers: text widths, paths, relative times, OSC 52.

use std::path::{Path, PathBuf};

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Display width of a string in terminal cells.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// `s` cut to at most `max` cells, ending in `…` when something was dropped.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// A line cut to `max` cells (the last kept span ends in `…` if anything was dropped).
pub fn fit_line(line: Line<'static>, max: usize) -> Line<'static> {
    if line.width() <= max {
        return line;
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let w = width(&span.content);
        if used + w <= max.saturating_sub(1) {
            used += w;
            spans.push(span);
            continue;
        }
        let room = max.saturating_sub(1).saturating_sub(used);
        let mut part = String::new();
        let mut taken = 0;
        for ch in span.content.chars() {
            let cw = ch.width().unwrap_or(0);
            if taken + cw > room {
                break;
            }
            part.push(ch);
            taken += cw;
        }
        part.push('…');
        spans.push(Span::styled(part, span.style));
        return Line::from(spans);
    }
    Line::from(spans)
}

/// `a` on the left and `b` on the right of `width` cells (`b` dropped when they don't fit).
pub fn split_line(
    left: Line<'static>,
    right: Option<Line<'static>>,
    width: usize,
) -> Line<'static> {
    let Some(right) = right else {
        return fit_line(left, width);
    };
    let (lw, rw) = (left.width(), right.width());
    if lw + 2 + rw > width {
        return fit_line(left, width);
    }
    let mut spans = left.spans;
    spans.push(Span::styled(" ".repeat(width - lw - rw), Style::new()));
    spans.extend(right.spans);
    Line::from(spans)
}

/// The path with the home directory written as `~`.
pub fn shorten_home(path: &str, home: Option<&str>) -> String {
    let Some(home) = home
        .map(|h| h.trim_end_matches('/'))
        .filter(|h| !h.is_empty())
    else {
        return path.to_string();
    };
    if path == home {
        return "~".into();
    }
    match path.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// "just now", "5m ago", "3h ago", "2d ago" for a unix time in ms.
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..=44 => "just now".into(),
        45..=3599 => format!("{}m ago", (secs / 60).max(1)),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ABC[(n >> 18) as usize & 63] as char);
        out.push(ABC[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ABC[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ABC[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The OSC 52 sequence that puts `text` on the terminal's clipboard.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Neo's data directory (as the daemon resolves it).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("NEO_DATA_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return Some(Path::new(&dir).join("neo"));
    }
    let home = std::env::var_os("HOME").filter(|d| !d.is_empty())?;
    Some(Path::new(&home).join(".local/share/neo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn home_is_shortened_only_on_a_path_boundary() {
        assert_eq!(shorten_home("/home/me/site", Some("/home/me")), "~/site");
        assert_eq!(shorten_home("/home/me", Some("/home/me/")), "~");
        assert_eq!(
            shorten_home("/home/meh/site", Some("/home/me")),
            "/home/meh/site"
        );
        assert_eq!(shorten_home("/x", None), "/x");
    }

    #[test]
    fn truncation_counts_cells() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("日本語です", 5), "日本…");
        let line = fit_line(Line::from(vec![Span::raw("abc"), Span::raw("defgh")]), 5);
        assert_eq!(line.to_string(), "abcd…");
    }

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(0, 10_000), "just now");
        assert_eq!(relative_time(0, 5 * 60_000), "5m ago");
        assert_eq!(relative_time(0, 3 * 3_600_000), "3h ago");
        assert_eq!(relative_time(0, 2 * 86_400_000), "2d ago");
    }
}
