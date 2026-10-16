//! The plan panel: the todo lists of the agent and its guests.

use std::cell::Cell;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use suite_protocol::{Plan, PlanEntryStatus};
use suite_renderer_ratatui::Palette;

use super::text::wrap;

#[derive(Debug, Clone, PartialEq)]
struct Item {
    text: String,
    status: PlanEntryStatus,
}

/// What `render` draws: one block is an agent header or one wrapped entry.
struct Block {
    lines: Vec<Line<'static>>,
    entry: bool,
    active: bool,
}

#[derive(Debug, Default)]
pub(crate) struct PlanState {
    /// Insertion order; the session's own agent (`None`) is drawn first anyway.
    lists: Vec<(Option<String>, Vec<Item>)>,
    /// First block shown after the user scrolled; None follows the in-progress entry.
    manual: Option<usize>,
    /// What the last draw started at / could start at at most (for the keys).
    auto_start: Cell<usize>,
    max_start: Cell<usize>,
}

impl PlanState {
    /// The agent's list is replaced by this event's; an empty one clears it.
    pub(crate) fn apply(&mut self, plan: &Plan) {
        let items: Vec<Item> = plan
            .entries
            .iter()
            .map(|e| Item {
                text: e.content.clone(),
                status: e.status,
            })
            .collect();
        let at = self.lists.iter().position(|(a, _)| *a == plan.agent);
        match (at, items.is_empty()) {
            (Some(i), true) => {
                self.lists.remove(i);
            }
            (Some(i), false) => self.lists[i].1 = items,
            (None, false) => self.lists.push((plan.agent.clone(), items)),
            (None, true) => {}
        }
        self.manual = None;
    }

    pub(crate) fn has_any(&self) -> bool {
        !self.lists.is_empty()
    }

    /// Lines the whole plan takes at `width`.
    pub(crate) fn natural_height(&self, width: usize, palette: &Palette) -> usize {
        self.blocks(width, palette)
            .iter()
            .map(|b| b.lines.len())
            .sum()
    }

    fn blocks(&self, width: usize, palette: &Palette) -> Vec<Block> {
        let multi = self.lists.len() > 1;
        let main = self.lists.iter().filter(|(a, _)| a.is_none());
        let guests = self.lists.iter().filter(|(a, _)| a.is_some());
        let mut blocks = Vec::new();
        for (agent, items) in main.chain(guests) {
            if multi {
                blocks.push(Block {
                    lines: vec![Line::from(Span::styled(
                        agent.clone().unwrap_or_else(|| "agent".to_string()),
                        Style::new().fg(palette.muted).add_modifier(Modifier::BOLD),
                    ))],
                    entry: false,
                    active: false,
                });
            }
            for item in items {
                blocks.push(entry_block(item, width, palette));
            }
        }
        blocks
    }

    /// Exactly `height` lines or fewer: the plan, or a window of it with `… N more` /
    /// `… N earlier` lines when it doesn't fit. The in-progress entry stays in view unless the
    /// user scrolled.
    pub(crate) fn render(
        &self,
        width: usize,
        height: usize,
        palette: &Palette,
    ) -> Vec<Line<'static>> {
        let blocks = self.blocks(width, palette);
        let total: usize = blocks.iter().map(|b| b.lines.len()).sum();
        if height == 0 {
            return Vec::new();
        }
        if total <= height {
            self.auto_start.set(0);
            self.max_start.set(0);
            return blocks.into_iter().flat_map(|b| b.lines).collect();
        }
        let max_start = (0..blocks.len())
            .find(|&s| fit(&blocks, s, height).end == blocks.len())
            .unwrap_or(blocks.len().saturating_sub(1));
        let auto = match blocks.iter().position(|b| b.active) {
            Some(active) => {
                let mut s = 0;
                while s < active && active >= fit(&blocks, s, height).end {
                    s += 1;
                }
                s
            }
            None => 0,
        };
        self.max_start.set(max_start);
        self.auto_start.set(auto);
        let start = self.manual.map_or(auto, |m| m.min(max_start));
        let f = fit(&blocks, start, height);
        let muted = Style::new().fg(palette.muted);
        let count = |r: std::ops::Range<usize>| blocks[r].iter().filter(|b| b.entry).count();
        let mut out = Vec::new();
        if f.top {
            out.push(Line::from(Span::styled(
                format!("… {} earlier", count(0..start)),
                muted,
            )));
        }
        let room = height - usize::from(f.top) - usize::from(f.bottom);
        let mut left = room;
        for b in &blocks[start..f.end.max(start + 1).min(blocks.len())] {
            let take = b.lines.len().min(left);
            out.extend(b.lines[..take].iter().cloned());
            left -= take;
        }
        if f.bottom {
            let shown_end = f.end.max(start + 1);
            out.push(Line::from(Span::styled(
                format!(
                    "… {} more",
                    count(shown_end.min(blocks.len())..blocks.len())
                ),
                muted,
            )));
        }
        out
    }

    /// Up/Down (and k/j, page keys) scroll. Returns true for Esc/q: leave the panel.
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        let cur = self.manual.unwrap_or(self.auto_start.get());
        let max = self.max_start.get();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Up | KeyCode::Char('k') => self.manual = Some(cur.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.manual = Some((cur + 1).min(max)),
            KeyCode::PageUp => self.manual = Some(cur.saturating_sub(5)),
            KeyCode::PageDown => self.manual = Some((cur + 5).min(max)),
            KeyCode::Home | KeyCode::Char('g') => self.manual = Some(0),
            KeyCode::End | KeyCode::Char('G') => self.manual = Some(usize::MAX),
            _ => {}
        }
        false
    }
}

fn entry_block(item: &Item, width: usize, palette: &Palette) -> Block {
    let (glyph, glyph_style, text_style) = match item.status {
        PlanEntryStatus::Completed => (
            "✓",
            Style::new().fg(palette.muted),
            Style::new()
                .fg(palette.muted)
                .add_modifier(Modifier::CROSSED_OUT),
        ),
        PlanEntryStatus::InProgress => {
            let s = Style::new().fg(palette.accent).add_modifier(Modifier::BOLD);
            ("◐", s, s)
        }
        PlanEntryStatus::Pending => ("○", Style::new().fg(palette.muted), Style::new()),
    };
    let wrapped = wrap(&item.text, width.saturating_sub(2));
    let mut lines = Vec::new();
    for (i, w) in wrapped.into_iter().enumerate() {
        let lead = if i == 0 {
            Span::styled(format!("{glyph} "), glyph_style)
        } else {
            Span::raw("  ")
        };
        lines.push(Line::from(vec![lead, Span::styled(w, text_style)]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(glyph, glyph_style)));
    }
    Block {
        lines,
        entry: true,
        active: item.status == PlanEntryStatus::InProgress,
    }
}

struct Fit {
    /// One past the last block drawn (it may be cut when it is the only one).
    end: usize,
    top: bool,
    bottom: bool,
}

/// Which blocks fit from `start` on, with room for the `… N earlier` / `… N more` lines.
fn fit(blocks: &[Block], start: usize, height: usize) -> Fit {
    let top = start > 0;
    let avail = height.saturating_sub(usize::from(top));
    let rest: usize = blocks[start..].iter().map(|b| b.lines.len()).sum();
    if rest <= avail {
        return Fit {
            end: blocks.len(),
            top,
            bottom: false,
        };
    }
    let avail = avail.saturating_sub(1);
    let mut used = 0;
    let mut end = start;
    while end < blocks.len() && used + blocks[end].lines.len() <= avail {
        used += blocks[end].lines.len();
        end += 1;
    }
    Fit {
        end,
        top,
        bottom: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_protocol::PlanEntry;

    fn entry(text: &str, status: PlanEntryStatus) -> PlanEntry {
        PlanEntry {
            content: text.into(),
            status,
            priority: None,
        }
    }

    fn plan(agent: Option<&str>, entries: Vec<PlanEntry>) -> Plan {
        Plan {
            agent: agent.map(String::from),
            entries,
        }
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    use PlanEntryStatus::{Completed as Done, InProgress as Doing, Pending as Todo};

    #[test]
    fn replace_and_clear_per_agent() {
        let mut s = PlanState::default();
        s.apply(&plan(None, vec![entry("a", Todo)]));
        s.apply(&plan(Some("claude"), vec![entry("c", Todo)]));
        s.apply(&plan(None, vec![entry("b", Done), entry("b2", Todo)]));
        let p = Palette::default();
        assert_eq!(
            text(&s.render(20, 10, &p)),
            ["agent", "✓ b", "○ b2", "claude", "○ c"]
        );
        s.apply(&plan(Some("claude"), vec![]));
        assert_eq!(text(&s.render(20, 10, &p)), ["✓ b", "○ b2"]);
        s.apply(&plan(None, vec![]));
        assert!(!s.has_any());
    }

    #[test]
    fn wraps_with_hanging_indent_and_styles() {
        let mut s = PlanState::default();
        s.apply(&plan(
            None,
            vec![entry("write the parser for porcelain", Doing)],
        ));
        let p = Palette::default();
        let lines = s.render(14, 10, &p);
        assert_eq!(text(&lines), ["◐ write the", "  parser for", "  porcelain"]);
        assert_eq!(lines[0].spans[1].style.fg, Some(p.accent));
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(s.natural_height(14, &p), 3);
    }

    #[test]
    fn keeps_the_active_entry_visible() {
        let mut s = PlanState::default();
        let mut entries: Vec<_> = (0..8).map(|i| entry(&format!("t{i}"), Done)).collect();
        entries[6].status = Doing;
        entries.push(entry("t8", Todo));
        entries.push(entry("t9", Todo));
        s.apply(&plan(None, entries));
        let p = Palette::default();
        let out = text(&s.render(20, 4, &p));
        assert_eq!(out.len(), 4);
        assert!(out.contains(&"◐ t6".to_string()), "{out:?}");
        assert_eq!(out[0], "… 5 earlier");
        assert!(out.last().unwrap().starts_with("… "));
        // From the top the window is the first entries and the rest is counted.
        let mut s = PlanState::default();
        s.apply(&plan(
            None,
            (0..6).map(|i| entry(&format!("t{i}"), Todo)).collect(),
        ));
        assert_eq!(text(&s.render(20, 3, &p)), ["○ t0", "○ t1", "… 4 more"]);
    }

    #[test]
    fn keys_scroll_and_leave() {
        let mut s = PlanState::default();
        s.apply(&plan(
            None,
            (0..6).map(|i| entry(&format!("t{i}"), Todo)).collect(),
        ));
        let p = Palette::default();
        let key = |c| KeyEvent::from(c);
        s.render(20, 3, &p);
        assert!(!s.key(key(KeyCode::Down)));
        assert_eq!(text(&s.render(20, 4, &p))[0], "… 1 earlier");
        s.key(key(KeyCode::End));
        let out = text(&s.render(20, 3, &p));
        assert_eq!(out.last().unwrap(), "○ t5");
        s.key(key(KeyCode::Up));
        assert!(text(&s.render(20, 3, &p)).contains(&"○ t4".to_string()));
        assert!(s.key(key(KeyCode::Esc)));
    }
}
