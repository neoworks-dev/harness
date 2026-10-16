//! The compact form of a tool result: a few lines under the item's header.
//!
//! The result is flattened into leaves (a card's title, a text, a code block, a diff, ...), and
//! the line budget is shared out so that every leaf gets a line before any gets a second: the
//! `Exit code 1` after a screenful of output is not cut off. A leaf that is cut ends in
//! `… +N lines`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use suite_protocol as pb;

use crate::render::{Cx, Lines};
use crate::text::{clean, one_line, sline, truncate, truncate_spans, width};

/// Lines shown before anything is cut (the `… +N lines` markers come on top).
const BUDGET: usize = 3;

enum Leaf<'c> {
    Title(&'c pb::CardComponent),
    Comp(&'c pb::Component),
}

fn leaves<'c>(c: &'c pb::Component, out: &mut Vec<Leaf<'c>>) {
    match c {
        pb::Component::CardComponent(v) => {
            if v.title.as_deref().is_some_and(|t| !one_line(t).is_empty()) {
                out.push(Leaf::Title(v));
            }
            v.children.iter().for_each(|c| leaves(c, out));
        }
        pb::Component::RowComponent(v) => v.children.iter().for_each(|c| leaves(c, out)),
        pb::Component::StackComponent(v) => v.children.iter().for_each(|c| leaves(c, out)),
        other => out.push(Leaf::Comp(other)),
    }
}

impl Cx<'_> {
    /// The result in `w` columns, ready to sit after `  ⎿ `.
    pub(crate) fn summary(&mut self, c: &pb::Component, w: usize) -> Lines {
        let mut all = Vec::new();
        leaves(c, &mut all);
        let mut out: Lines = Vec::new();
        let mut shown = 0;
        let mut dropped = 0;
        for (i, leaf) in all.iter().enumerate() {
            let room = BUDGET - shown;
            let later = all.len() - 1 - i;
            let avail = room.saturating_sub(later).max(room.min(1));
            let (lines, total) = self.leaf_lines(leaf, w, avail);
            if avail == 0 {
                dropped += total;
                continue;
            }
            shown += lines.len();
            let cut = total - lines.len();
            out.extend(lines);
            if cut > 0 {
                out.push(self.more(cut));
            }
        }
        if dropped > 0 {
            out.push(self.more(dropped));
        }
        out
    }

    fn more(&self, n: usize) -> Line<'static> {
        sline(format!("… +{n} lines"), self.p.muted_style())
    }

    /// At most `avail` lines of the leaf, and how many lines it has in all.
    fn leaf_lines(&mut self, leaf: &Leaf<'_>, w: usize, avail: usize) -> (Lines, usize) {
        let mut lines = match leaf {
            Leaf::Title(card) => {
                let title = card.title.as_deref().map(one_line).unwrap_or_default();
                let mut st = Style::new().add_modifier(Modifier::BOLD);
                if let Some(tone) = card.tone {
                    st = st.fg(self.p.tone(tone));
                }
                vec![sline(truncate(&title, w), st)]
            }
            Leaf::Comp(pb::Component::CodeComponent(v)) => return code_lines(&v.code, w, avail),
            Leaf::Comp(pb::Component::TextComponent(v)) => self.text(v, w),
            Leaf::Comp(pb::Component::DiffComponent(v)) => vec![self.diff_line(v, w)],
            Leaf::Comp(pb::Component::KvComponent(v)) => vec![self.kv_line(v, w)],
            Leaf::Comp(pb::Component::ImageComponent(v)) => {
                let name = v
                    .alt
                    .as_deref()
                    .map(one_line)
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| one_line(&v.src));
                let text = truncate(&format!("[image {name}]"), w);
                vec![sline(text, self.p.muted_style())]
            }
            Leaf::Comp(pb::Component::TableComponent(v)) => {
                let rows = match v.rows.len() {
                    1 => "1 row".to_string(),
                    n => format!("{n} rows"),
                };
                let cols: Vec<String> = v.columns.iter().map(|c| one_line(c)).collect();
                let text = if cols.is_empty() {
                    rows
                } else {
                    format!("{rows} · {}", cols.join(", "))
                };
                vec![Line::raw(truncate(&text, w))]
            }
            // Files, links, progress bars, buttons and what we don't know are short already.
            Leaf::Comp(other) => self.comp(other, w),
        };
        let total = lines.len();
        lines.truncate(avail);
        (lines, total)
    }

    /// `path  +3 −1`
    fn diff_line(&self, v: &pb::DiffComponent, w: usize) -> Line<'static> {
        let count = |kind| v.lines.iter().filter(|l| l.kind == kind).count();
        let (added, removed) = (
            count(pb::DiffLineKind::Add),
            count(pb::DiffLineKind::Remove),
        );
        let adds = format!("+{added}");
        let removes = format!("−{removed}");
        let path = v
            .path
            .as_deref()
            .map(one_line)
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "diff".to_string());
        let counts_w = width(&adds) + 1 + width(&removes);
        let path = truncate(&path, w.saturating_sub(counts_w + 2).max(1));
        Line::from(vec![
            Span::raw(path),
            Span::raw("  "),
            Span::styled(adds, Style::new().fg(self.p.success)),
            Span::raw(" "),
            Span::styled(removes, Style::new().fg(self.p.danger)),
        ])
    }

    /// `key: value · key: value`
    fn kv_line(&self, v: &pb::KvComponent, w: usize) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, item) in v.items.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", self.p.muted_style()));
            }
            spans.push(Span::styled(
                format!("{}:", one_line(&item.key)),
                self.p.muted_style(),
            ));
            spans.push(Span::raw(format!(" {}", one_line(&item.value))));
        }
        Line::from(truncate_spans(spans, w))
    }
}

/// The first `avail` lines of the output, each cut to the width; no wrapping, so a screenful
/// of output costs one pass.
fn code_lines(code: &str, w: usize, avail: usize) -> (Lines, usize) {
    let cleaned = clean(code);
    let text = cleaned.trim_end_matches('\n');
    if text.trim().is_empty() {
        return (Vec::new(), 0);
    }
    let total = text.split('\n').count();
    let lines = text
        .split('\n')
        .take(avail)
        .map(|l| Line::raw(truncate(l, w)))
        .collect();
    (lines, total)
}
