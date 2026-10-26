//! The compact form of a tool result: one line under the item's header.
//!
//! The result is flattened into leaves (a card's title, a text, a code block, a diff, ...); the
//! first leaf's first line is shown, and what is left out is counted at its end: `… +N lines`.

use neo_protocol as pb;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::render::{Cx, Lines};
use crate::text::{clean, one_line, sline, truncate, truncate_spans, width};

/// Lines shown; the `… +N lines` count goes at the end of the last one.
const BUDGET: usize = 1;

fn is_image(leaf: &Leaf<'_>) -> bool {
    matches!(leaf, Leaf::Comp(pb::Component::ImageComponent(_)))
}

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
            // A picture shown for real is not text: it keeps clear of the line budget.
            if let (Leaf::Comp(pb::Component::ImageComponent(v)), Some(_)) =
                (leaf, self.o.images.as_ref())
            {
                out.extend(self.image(v, w));
                continue;
            }
            let room = BUDGET - shown;
            let later = all[i + 1..]
                .iter()
                .filter(|l| !(self.o.images.is_some() && is_image(l)))
                .count();
            let avail = room.saturating_sub(later).max(room.min(1));
            let (lines, total) = self.leaf_lines(leaf, w, avail);
            if avail == 0 {
                dropped += total;
                continue;
            }
            shown += lines.len();
            dropped += total - lines.len();
            out.extend(lines);
        }
        if dropped > 0 {
            self.more(&mut out, dropped, w);
        }
        out
    }

    /// `… +N lines` after the last line, which is cut to make room; a line of its own when
    /// nothing was shown.
    fn more(&self, out: &mut Lines, n: usize, w: usize) {
        let tag = format!("… +{n} lines");
        let muted = self.p.muted_style();
        match out.last_mut() {
            Some(last) if w > width(&tag) + 2 => {
                let spans = std::mem::take(&mut last.spans);
                let mut spans = truncate_spans(spans, w - width(&tag) - 2);
                spans.push(Span::styled(format!("  {tag}"), muted));
                last.spans = spans;
            }
            _ => out.push(sline(truncate(&tag, w), muted)),
        }
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
            Leaf::Comp(pb::Component::OpenUiComponent(v)) => {
                let (lines, total) = self.openui_summary(&v.ui, w);
                let shown = lines.len().min(avail);
                return (lines.into_iter().take(shown).collect(), total);
            }
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

    /// `path  +3 −1`, with the file's icon before it when icons are on.
    fn diff_line(&self, v: &pb::DiffComponent, w: usize) -> Line<'static> {
        let icon = v
            .path
            .as_deref()
            .filter(|p| self.o.file_icons && !p.is_empty())
            .map(|p| crate::file_icon(p, &self.p));
        let w = if icon.is_some() {
            w.saturating_sub(2)
        } else {
            w
        };
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
        let mut spans = match icon {
            Some(icon) => vec![icon, Span::raw(" ")],
            None => Vec::new(),
        };
        spans.extend([
            Span::raw(path),
            Span::raw("  "),
            Span::styled(adds, Style::new().fg(self.p.success)),
            Span::raw(" "),
            Span::styled(removes, Style::new().fg(self.p.danger)),
        ]);
        Line::from(spans)
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
            spans.push(Span::raw(" "));
            spans.push(Span::styled(one_line(&item.value), self.value_style(item)));
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
