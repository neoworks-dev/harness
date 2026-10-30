//! A chat item back as markdown: what `y` copies from the chat and what the reader shows. The
//! daemon parses the agent's markdown into blocks; this writes them out again.

use neo_protocol::{Block, ColumnAlign, Component, Span, ViewItem};

/// The item as markdown; empty when it holds nothing that reads as text.
pub fn item_markdown(item: &ViewItem) -> String {
    let mut parts = Vec::new();
    component(&item.component, &mut parts);
    parts.join("\n\n")
}

fn component(c: &Component, out: &mut Vec<String>) {
    match c {
        Component::TextComponent(t) => out.extend(t.blocks.iter().map(block)),
        Component::CardComponent(v) => v.children.iter().for_each(|c| component(c, out)),
        Component::RowComponent(v) => v.children.iter().for_each(|c| component(c, out)),
        Component::StackComponent(v) => v.children.iter().for_each(|c| component(c, out)),
        Component::CodeComponent(v) => out.push(fence(&v.code, v.language.as_deref())),
        Component::ImageComponent(v) => {
            out.push(format!("![{}]({})", v.alt.as_deref().unwrap_or(""), v.src));
        }
        _ => {}
    }
}

fn block(b: &Block) -> String {
    match b {
        Block::ParagraphBlock(p) => spans(&p.spans),
        Block::HeadingBlock(h) => {
            let level = h.level.clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), spans(&h.spans))
        }
        Block::ListBlock(l) => {
            let start = l.start.unwrap_or(1);
            let items: Vec<String> = l
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let mut bullet = if l.ordered {
                        format!("{}. ", start + i as i64)
                    } else {
                        "- ".to_string()
                    };
                    match item.checked {
                        Some(true) => bullet.push_str("[x] "),
                        Some(false) => bullet.push_str("[ ] "),
                        None => {}
                    }
                    let body = item.blocks.iter().map(block).collect::<Vec<_>>().join("\n");
                    indent(&body, &bullet, &" ".repeat(bullet.chars().count()))
                })
                .collect();
            items.join("\n")
        }
        Block::CodeBlock(c) => fence(&c.code, c.language.as_deref()),
        Block::QuoteBlock(q) => {
            let body = q.blocks.iter().map(block).collect::<Vec<_>>().join("\n\n");
            body.lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".into()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Block::RuleBlock(_) => "---".into(),
        Block::TableBlock(t) => {
            let row = |cells: Vec<String>| format!("| {} |", cells.join(" | "));
            let cell = |s: &[Span]| spans(s).replace('|', "\\|");
            let mut lines = vec![row(t.header.iter().map(|c| cell(&c.spans)).collect())];
            let align = t.align.clone().unwrap_or_default();
            lines.push(row((0..t.header.len())
                .map(|i| match align.get(i) {
                    Some(ColumnAlign::Center) => ":---:".into(),
                    Some(ColumnAlign::Right) => "---:".into(),
                    _ => "---".into(),
                })
                .collect()));
            for r in &t.rows {
                lines.push(row(r.iter().map(|c| cell(&c.spans)).collect()));
            }
            lines.join("\n")
        }
        // The program itself isn't kept, only what it evaluated to.
        Block::NeoUiBlock(_) => "```neoui\n```".into(),
        Block::Unknown(_) => String::new(),
    }
}

/// `text` with `first` before its first line and `rest` before the others (empty lines stay
/// empty).
fn indent(text: &str, first: &str, rest: &str) -> String {
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let lead = if i == 0 { first } else { rest };
        if line.is_empty() && i > 0 {
            out.push(String::new());
        } else {
            out.push(format!("{lead}{line}"));
        }
    }
    out.join("\n")
}

/// A fenced code block, the fence longer than any run of backticks in the code.
fn fence(code: &str, language: Option<&str>) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in code.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let ticks = "`".repeat(longest.max(2) + 1);
    let code = code.strip_suffix('\n').unwrap_or(code);
    format!("{ticks}{}\n{code}\n{ticks}", language.unwrap_or(""))
}

fn spans(spans: &[Span]) -> String {
    let mut out = String::new();
    for s in spans {
        let mut text = if s.code == Some(true) {
            let ticks = if s.text.contains('`') { "``" } else { "`" };
            let pad = if s.text.starts_with('`') || s.text.ends_with('`') {
                " "
            } else {
                ""
            };
            format!("{ticks}{pad}{}{pad}{ticks}", s.text)
        } else {
            s.text.clone()
        };
        // Emphasis marks go inside the whitespace around the text, where markdown wants them.
        let wrap = |text: String, mark: &str| -> String {
            let body = text.trim();
            if body.is_empty() {
                return text;
            }
            let lead = &text[..text.len() - text.trim_start().len()];
            let tail = &text[text.trim_end().len()..];
            format!("{lead}{mark}{body}{mark}{tail}")
        };
        if s.strike == Some(true) {
            text = wrap(text, "~~");
        }
        if s.italic == Some(true) {
            text = wrap(text, "*");
        }
        if s.bold == Some(true) {
            text = wrap(text, "**");
        }
        if let Some(href) = s.href.as_deref() {
            text = if text == href {
                format!("<{href}>")
            } else {
                format!("[{text}]({href})")
            };
        }
        out.push_str(&text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn md(blocks: serde_json::Value) -> String {
        let item: ViewItem = serde_json::from_value(json!({
            "ref": "a", "kind": "agent",
            "component": {"type": "text", "blocks": blocks}
        }))
        .unwrap();
        item_markdown(&item)
    }

    #[test]
    fn writes_the_blocks_back_as_markdown() {
        let got = md(json!([
            {"type": "heading", "level": 2, "spans": [{"text": "Plan"}]},
            {"type": "paragraph", "spans": [
                {"text": "Run "}, {"text": "cargo test", "code": true}, {"text": " and "},
                {"text": "check it ", "bold": true}, {"text": "here", "href": "https://x.y"}
            ]},
            {"type": "list", "ordered": true, "items": [
                {"blocks": [{"type": "paragraph", "spans": [{"text": "one"}]}]},
                {"blocks": [
                    {"type": "paragraph", "spans": [{"text": "two"}]},
                    {"type": "list", "ordered": false, "items": [
                        {"checked": true, "blocks": [{"type": "paragraph", "spans": [{"text": "done"}]}]}
                    ]}
                ]}
            ]},
            {"type": "code_block", "language": "rs", "code": "fn main() {}\n"},
            {"type": "quote", "blocks": [{"type": "paragraph", "spans": [{"text": "said"}]}]},
            {"type": "rule"},
            {"type": "table_block",
             "header": [{"spans": [{"text": "a"}]}, {"spans": [{"text": "b"}]}],
             "rows": [[{"spans": [{"text": "1"}]}, {"spans": [{"text": "x|y"}]}]],
             "align": ["left", "right"]}
        ]));
        assert_eq!(
            got,
            "## Plan\n\n\
             Run `cargo test` and **check it** [here](https://x.y)\n\n\
             1. one\n2. two\n   - [x] done\n\n\
             ```rs\nfn main() {}\n```\n\n\
             > said\n\n\
             ---\n\n\
             | a | b |\n| --- | ---: |\n| 1 | x\\|y |"
        );
    }

    #[test]
    fn a_fence_outgrows_the_backticks_in_its_code() {
        let got = md(json!([{"type": "code_block", "code": "```\nx\n```"}]));
        assert_eq!(got, "````\n```\nx\n```\n````");
    }
}
