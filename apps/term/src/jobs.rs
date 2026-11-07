//! The background commands under the prompt bar: one row per command the agent runs in the
//! background (and a few seconds after it ends), with its last line of output and how long it
//! has run. Down from an empty editor (past the subagents) selects one, which the terminal panel
//! then shows on its own as it writes; Enter gives the panel the keys, to scroll it; `x` stops
//! it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::agents::elapsed;
use crate::style::Styles;
use crate::util::{fit_line, truncate, width};
use crate::widgets::JobRow;

/// Rows shown before the rest is folded into "+N more".
const MAX_ROWS: usize = 4;

/// What a key in the focused list asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum JobsAction {
    /// The selection moved to this one (the terminal panel shows it).
    Show(String),
    /// Up from the top row.
    Up,
    /// Back to the editor (Esc).
    Leave,
    /// Give the terminal panel the keys, showing this one (Enter).
    Open(String),
    /// Stop this one (`x`, while it runs).
    Stop(String),
    Redraw,
}

#[derive(Debug, Default)]
pub struct Jobs {
    /// The row the keys are on while the list has focus.
    selected: Option<String>,
}

impl Jobs {
    /// Rows the list takes for these commands: a heading, the rows (folded after a few), the
    /// keys while focused.
    pub fn height(rows: &[JobRow], focused: bool) -> usize {
        if rows.is_empty() {
            return 0;
        }
        1 + rows.len().min(MAX_ROWS) + usize::from(rows.len() > MAX_ROWS) + usize::from(focused)
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The list gets focus, coming down from the editor: the keys start on its top row shown.
    pub fn focus(&mut self, rows: &[JobRow]) -> Option<String> {
        self.selected = rows
            .get(rows.len().saturating_sub(MAX_ROWS))
            .map(|r| r.id.clone());
        self.selected.clone()
    }

    pub fn key(&mut self, key: KeyEvent, rows: &[JobRow]) -> JobsAction {
        if key.kind == KeyEventKind::Release || key.modifiers.contains(KeyModifiers::CONTROL) {
            return JobsAction::Redraw;
        }
        let Some(at) = self
            .selected
            .as_ref()
            .and_then(|s| rows.iter().position(|r| r.id == *s))
        else {
            // The selected one has gone: start again from the top.
            return match self.focus(rows) {
                Some(id) => JobsAction::Show(id),
                None => JobsAction::Leave,
            };
        };
        // The folded rows aren't shown: Up from the top row shown leaves the list.
        let top = rows.len().saturating_sub(MAX_ROWS);
        let to = match key.code {
            KeyCode::Up | KeyCode::Char('k') if at <= top => return JobsAction::Up,
            KeyCode::Up | KeyCode::Char('k') => at - 1,
            KeyCode::Down | KeyCode::Char('j') => (at + 1).min(rows.len() - 1),
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                return JobsAction::Leave;
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                return JobsAction::Open(rows[at].id.clone());
            }
            KeyCode::Char('x') if rows[at].ended_ms.is_none() => {
                return JobsAction::Stop(rows[at].id.clone());
            }
            _ => return JobsAction::Redraw,
        };
        if to == at {
            return JobsAction::Redraw;
        }
        self.selected = Some(rows[to].id.clone());
        JobsAction::Show(rows[to].id.clone())
    }

    /// `background · 2`, then a row per command, then the keys while focused.
    pub fn lines(
        &self,
        rows: &[JobRow],
        w: usize,
        now_ms: i64,
        focused: bool,
        styles: &Styles,
    ) -> Vec<Line<'static>> {
        if rows.is_empty() || w < 12 {
            return Vec::new();
        }
        let running = rows.iter().filter(|r| r.ended_ms.is_none()).count();
        let mut out = vec![Line::from(vec![
            Span::styled("⏵", styles.accent),
            Span::raw(" background"),
            Span::styled(format!(" · {running} running"), styles.muted),
        ])];
        // The newest last, like the chat; the oldest fold away.
        let skip = rows.len().saturating_sub(MAX_ROWS);
        if skip > 0 {
            out.push(Line::from(Span::styled(
                format!("  … +{skip} more"),
                styles.muted,
            )));
        }
        for job in &rows[skip..] {
            let mut line = row(job, w, now_ms, styles);
            if focused && self.selected.as_deref() == Some(job.id.as_str()) {
                let pad = w.saturating_sub(line.width());
                line.spans.push(Span::raw(" ".repeat(pad)));
                for span in &mut line.spans {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                }
            }
            out.push(line);
        }
        if focused {
            out.push(Line::from(Span::styled(
                "↑/↓ select · enter scroll its output · x stop · esc back",
                styles.muted,
            )));
        }
        out.into_iter().map(|l| fit_line(l, w)).collect()
    }
}

/// `○ cargo build --release   Compiling neo v0.1   1m 12s`
fn row(job: &JobRow, w: usize, now_ms: i64, styles: &Styles) -> Line<'static> {
    let (glyph, style) = match job.exit {
        None => ("○", styles.accent),
        Some(0) => ("✓", styles.success),
        Some(_) => ("✗", styles.danger),
    };
    let mut stats = elapsed(job.ended_ms.unwrap_or(now_ms) - job.started_ms);
    if let Some(code) = job.exit.filter(|&c| c != 0) {
        stats = format!("exit {code} · {stats}");
    }
    let command = job.command.split_whitespace().collect::<Vec<_>>().join(" ");
    let command = match &job.agent {
        Some(agent) => format!("{agent}: {command}"),
        None => command,
    };
    // The command gets up to half, the stats their room, the last line what is left.
    let room = w.saturating_sub(2 + width(&stats) + 2);
    let command = truncate(&command, room.min((w / 2).max(12)));
    let left = room.saturating_sub(width(&command) + 2);
    let last = job
        .last
        .as_deref()
        .map(|l| truncate(l.trim(), left))
        .unwrap_or_default();
    let used = 2 + width(&command) + 2 + width(&last);
    let gap = " ".repeat(w.saturating_sub(used + width(&stats)).max(1));
    Line::from(vec![
        Span::styled(glyph, style),
        Span::raw(format!(" {command}")),
        Span::raw("  "),
        Span::styled(last, styles.muted),
        Span::raw(gap),
        Span::styled(stats, styles.muted),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, exit: Option<i64>) -> JobRow {
        JobRow {
            id: id.into(),
            agent: None,
            command: format!("run {id}"),
            started_ms: 0,
            ended_ms: exit.map(|_| 3_000),
            exit,
            last: Some(format!("line of {id}")),
        }
    }

    #[test]
    fn the_keys_move_show_and_open() {
        let rows = [job("a", None), job("b", None)];
        let mut jobs = Jobs::default();
        assert_eq!(jobs.focus(&rows).as_deref(), Some("a"), "the top row");
        let key = |c| KeyEvent::from(c);
        assert_eq!(
            jobs.key(key(KeyCode::Down), &rows),
            JobsAction::Show("b".into())
        );
        assert_eq!(
            jobs.key(key(KeyCode::Up), &rows),
            JobsAction::Show("a".into())
        );
        assert_eq!(jobs.key(key(KeyCode::Up), &rows), JobsAction::Up);
        assert_eq!(
            jobs.key(key(KeyCode::Enter), &rows),
            JobsAction::Open("a".into())
        );
        assert_eq!(jobs.key(key(KeyCode::Esc), &rows), JobsAction::Leave);
        // Its command ended and went: the keys start over.
        assert_eq!(
            jobs.key(key(KeyCode::Down), &rows[1..]),
            JobsAction::Show("b".into())
        );
        // Folded past a few rows: the top one shown, not one folded away.
        let many: Vec<_> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|id| job(id, None))
            .collect();
        assert_eq!(jobs.focus(&many).as_deref(), Some("c"));
    }
}
