//! Shared by the test files: fixtures and text dumps of rendered lines.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use neo_protocol::ViewItem;
use neo_renderer_ratatui::{Options, Palette, plain_text, render_item};
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("resolved.json").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

pub fn fixture_items(name: &str) -> Vec<ViewItem> {
    let path = fixtures_dir().join(name).join("resolved.json");
    let data = fs::read_to_string(&path).expect("resolved.json");
    serde_json::from_str(&data).unwrap_or_else(|err| panic!("{name}: {err}"))
}

pub fn item(json: &str) -> ViewItem {
    serde_json::from_str(json).unwrap_or_else(|err| panic!("{err}: {json}"))
}

/// An item whose component is the given JSON.
pub fn item_of(kind: &str, extra: &str, component: &str) -> ViewItem {
    item(&format!(
        r#"{{"ref":"x","kind":"{kind}",{extra}"component":{component}}}"#
    ))
}

/// Items one after the other, a blank line between, as an app would draw them.
pub fn render_all(items: &[ViewItem], width: u16, opts: &Options) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push(Line::default());
        }
        out.extend(render_item(it, width, opts));
    }
    out
}

pub fn plain(lines: &[Line<'_>]) -> String {
    let mut s = plain_text(lines).join("\n");
    s.push('\n');
    s
}

pub fn lines_of(it: &ViewItem, width: u16, opts: &Options) -> Vec<String> {
    plain_text(&render_item(it, width, opts))
}

/// Like [`plain`], with every styled run as `⟦style|text⟧`: colors by theme name, `bg:` for
/// backgrounds, then modifiers.
pub fn styled(lines: &[Line<'_>], pal: &Palette) -> String {
    let name = |c: Color| -> String {
        let table = [
            (pal.fg, "fg"),
            (pal.bg, "bg"),
            (pal.surface, "surface"),
            (pal.border, "border"),
            (pal.accent, "accent"),
            (pal.muted, "muted"),
            (pal.success, "success"),
            (pal.danger, "danger"),
        ];
        match table.iter().find(|(color, _)| *color == c) {
            Some((_, n)) => n.to_string(),
            None => format!("{c}"),
        }
    };
    let mut out = String::new();
    for line in lines {
        let mut text = String::new();
        for span in &line.spans {
            let st = span.style;
            let mut parts: Vec<String> = Vec::new();
            if let Some(c) = st.fg {
                parts.push(name(c));
            }
            if let Some(c) = st.bg {
                parts.push(format!("bg:{}", name(c)));
            }
            for (m, n) in [
                (Modifier::BOLD, "bold"),
                (Modifier::DIM, "dim"),
                (Modifier::ITALIC, "italic"),
                (Modifier::UNDERLINED, "underlined"),
                (Modifier::REVERSED, "reversed"),
                (Modifier::CROSSED_OUT, "crossed-out"),
            ] {
                if st.add_modifier.contains(m) {
                    parts.push(n.to_string());
                }
            }
            if parts.is_empty() {
                text.push_str(&span.content);
            } else {
                text.push_str(&format!("⟦{}|{}⟧", parts.join(" "), span.content));
            }
        }
        out.push_str(text.trim_end());
        out.push('\n');
    }
    out
}
