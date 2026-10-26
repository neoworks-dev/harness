//! The terminal panel: the commands the agent and its guests ran (`execute` tool calls), as
//! a shell would show them: `$ command`, its output as it comes, and how it ended.

use std::cell::Cell;
use std::collections::HashSet;

use neo_protocol::{ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind};
use neo_renderer_ratatui::{Palette, highlight_shell, strip_cd};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use super::text::clean_line;

/// Commands kept, and lines of output kept per command (the end of it).
const RUNS: usize = 50;
const OUTPUT_LINES: usize = 400;
/// Rows a command takes at most once it is no longer waiting for approval.
const COMMAND_ROWS: usize = 4;

#[derive(Debug, Clone)]
struct Run {
    id: String,
    agent: Option<String>,
    command: String,
    /// Cleaned lines, the last [`OUTPUT_LINES`].
    output: Vec<String>,
    status: ToolCallStatus,
    exit: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct TerminalState {
    runs: Vec<Run>,
    /// toolCallIds of calls that are not commands, so their updates are passed over cheaply.
    others: HashSet<String>,
    /// Lines scrolled up from the end; None follows the output.
    back: Option<usize>,
    /// How far up the last draw could go (for the keys).
    max_back: Cell<usize>,
}

impl TerminalState {
    pub(crate) fn has_any(&self) -> bool {
        !self.runs.is_empty()
    }

    pub(crate) fn on_call(&mut self, call: &ToolCall) {
        if call.kind != ToolKind::Execute {
            self.others.insert(call.tool_call_id.clone());
            return;
        }
        let command = command_of(call.raw_input.as_ref()).unwrap_or_else(|| {
            call.title
                .strip_prefix("bash: ")
                .unwrap_or(&call.title)
                .to_string()
        });
        if let Some(run) = self.runs.iter_mut().find(|r| r.id == call.tool_call_id) {
            run.command = command;
            run.status = call.status;
            return;
        }
        self.runs.push(Run {
            id: call.tool_call_id.clone(),
            agent: call.agent.clone(),
            command,
            output: Vec::new(),
            status: call.status,
            exit: None,
        });
        if self.runs.len() > RUNS {
            self.runs.remove(0);
        }
    }

    /// Output replaces what was there (a running command's updates carry the tail so far).
    pub(crate) fn on_update(&mut self, update: &ToolCallUpdate) {
        if self.others.contains(&update.tool_call_id) {
            return;
        }
        let Some(run) = self.runs.iter_mut().find(|r| r.id == update.tool_call_id) else {
            return;
        };
        if let Some(command) = command_of(update.raw_input.as_ref()) {
            run.command = command;
        }
        if let Some(status) = update.status {
            run.status = status;
        }
        if let Some(content) = &update.content {
            let text: Vec<&str> = content
                .iter()
                .filter_map(|c| match c {
                    ToolCallContent::TextContent(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect();
            run.output = output_lines(&text.join("\n"));
        }
        if let Some(out) = update.raw_output.as_ref().and_then(|o| o.as_object()) {
            run.exit = ["exitCode", "exit_code"]
                .iter()
                .find_map(|k| out.get(*k).and_then(|v| v.as_i64()));
        }
    }

    /// The last `height` lines (or as far up as scrolled), `width` cells wide; a `cd` into `cwd`
    /// left out of the commands.
    pub(crate) fn render(
        &self,
        width: usize,
        height: usize,
        cwd: Option<&str>,
        p: &Palette,
    ) -> Vec<Line<'static>> {
        let want = height + self.back.unwrap_or(0);
        // From the newest run back, only as many as fill the view; a rule between runs.
        let mut blocks: Vec<Vec<Line<'static>>> = Vec::new();
        let mut have = 0;
        let mut complete = true;
        for (i, run) in self.runs.iter().enumerate().rev() {
            let mut lines = run_lines(run, width, cwd, p);
            if i + 1 < self.runs.len() {
                lines.push(Line::from(Span::styled(
                    "─".repeat(width),
                    Style::new().fg(p.border),
                )));
            }
            have += lines.len();
            blocks.push(lines);
            if have >= want {
                complete = i == 0;
                break;
            }
        }
        let all: Vec<Line<'static>> = blocks.into_iter().rev().flatten().collect();
        // Up as far as there is; while some runs were left out, at least what was drawn.
        let max = all.len().saturating_sub(height);
        self.max_back
            .set(if complete { max } else { max.max(want) });
        let back = self.back.unwrap_or(0).min(max);
        let end = all.len() - back;
        all[end.saturating_sub(height)..end].to_vec()
    }

    /// Keys while focused; true leaves the panel.
    pub(crate) fn key(&mut self, key: KeyEvent, page: usize) -> bool {
        let cur = self.back.unwrap_or(0);
        let max = self.max_back.get();
        let to = |n: usize| (n > 0).then_some(n.min(max));
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Up | KeyCode::Char('k') => self.back = to(cur + 1),
            KeyCode::Down | KeyCode::Char('j') => self.back = to(cur.saturating_sub(1)),
            KeyCode::PageUp => self.back = to(cur + page),
            KeyCode::PageDown => self.back = to(cur.saturating_sub(page)),
            KeyCode::Home | KeyCode::Char('g') => self.back = to(usize::MAX),
            KeyCode::End | KeyCode::Char('G') => self.back = None,
            _ => {}
        }
        false
    }

    /// Whether the view follows the newest output.
    pub(crate) fn following(&self) -> bool {
        self.back.is_none()
    }
}

/// `rawInput.command`, a string or (Codex) a list of words.
fn command_of(input: Option<&serde_json::Value>) -> Option<String> {
    let command = input?.get("command")?;
    match command {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(words) => Some(
            words
                .iter()
                .filter_map(|w| w.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

/// Output as lines to draw: escape sequences gone, a carriage return keeping what came after
/// it (progress bars), a fence around it (some adapters send one) dropped.
fn output_lines(text: &str) -> Vec<String> {
    let text = strip_escapes(text);
    let mut lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    if lines.len() >= 2 && lines[0].starts_with("```") && lines[lines.len() - 1] == "```" {
        lines = lines[1..lines.len() - 1].to_vec();
    }
    if lines == [""] {
        return Vec::new();
    }
    let skip = lines.len().saturating_sub(OUTPUT_LINES);
    lines[skip..]
        .iter()
        .map(|l| {
            let l = l.trim_end_matches('\r');
            clean_line(l.rsplit('\r').next().unwrap_or(l))
        })
        .collect()
}

/// Drops CSI (`ESC [ … final`) and OSC (`ESC ] … BEL` or `ESC \`) sequences and other
/// two-character escapes.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
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
        }
    }
    out
}

/// `$ command` (highlighted, continuation lines under it, at most [`COMMAND_ROWS`] once it
/// started), the output, and a muted `exit 1` (danger) or `running…` line.
fn run_lines(run: &Run, width: usize, cwd: Option<&str>, p: &Palette) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut lead = Vec::new();
    if let Some(agent) = &run.agent {
        lead.push(Span::styled(format!("{agent} "), Style::new().fg(p.muted)));
    }
    lead.push(Span::styled(
        "$ ",
        Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
    ));
    let mut spans = lead;
    spans.extend(highlight_shell(
        &clean_command(strip_cd(&run.command, cwd)),
        p,
        Style::new(),
    ));
    let mut command = wrap_spans(spans, width);
    // What waits for approval is read in full; after that the output is what matters.
    if run.status != ToolCallStatus::Pending && command.len() > COMMAND_ROWS {
        let more = command.len() - (COMMAND_ROWS - 1);
        command.truncate(COMMAND_ROWS - 1);
        command.push(Line::from(Span::styled(
            format!("… +{more} lines"),
            Style::new().fg(p.muted),
        )));
    }
    out.extend(command);
    for line in &run.output {
        out.extend(wrap_spans(vec![Span::raw(line.clone())], width));
    }
    let muted = Style::new().fg(p.muted);
    match run.status {
        ToolCallStatus::Pending => {
            out.push(Line::from(Span::styled("waiting for approval…", muted)))
        }
        ToolCallStatus::InProgress => out.push(Line::from(Span::styled("running…", muted))),
        ToolCallStatus::Failed => {
            let text = run
                .exit
                .map_or_else(|| "failed".to_string(), |code| format!("exit {code}"));
            out.push(Line::from(Span::styled(text, Style::new().fg(p.danger))));
        }
        ToolCallStatus::Completed => {}
    }
    out
}

/// The command's lines cleaned of control characters, newlines kept.
fn clean_command(command: &str) -> String {
    command
        .trim_end()
        .split('\n')
        .map(clean_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Spans cut into lines of `width` cells at newlines and wherever they reach the edge.
fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in spans {
        let mut text = String::new();
        for c in span.content.chars() {
            if c == '\n' {
                if !text.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut cur)));
                used = 0;
                continue;
            }
            let w = c.width().unwrap_or(0);
            if used + w > width {
                if !text.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut cur)));
                used = 0;
            }
            text.push(c);
            used += w;
        }
        if !text.is_empty() {
            cur.push(Span::styled(text, span.style));
        }
    }
    lines.push(Line::from(cur));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_renderer_ratatui::plain_text;
    use serde_json::json;

    fn call(id: &str, kind: &str, command: &str) -> ToolCall {
        serde_json::from_value(json!({
            "toolCallId": id, "title": format!("bash: {command}"), "kind": kind,
            "status": "in_progress", "rawInput": {"command": command}
        }))
        .unwrap()
    }

    fn update(id: &str, status: Option<&str>, text: &str, exit: Option<i64>) -> ToolCallUpdate {
        let mut v = json!({"toolCallId": id, "content": [{"type": "text", "text": text}]});
        if let Some(s) = status {
            v["status"] = json!(s);
        }
        if let Some(code) = exit {
            v["rawOutput"] = json!({"exitCode": code});
        }
        serde_json::from_value(v).unwrap()
    }

    fn shown(t: &TerminalState, w: usize, h: usize) -> Vec<String> {
        plain_text(&t.render(w, h, Some("/w/site"), &Palette::default()))
    }

    #[test]
    fn a_cd_home_is_left_out_and_a_long_command_cut_once_it_ran() {
        let mut t = TerminalState::default();
        let mut c = call(
            "1",
            "execute",
            "cd /w/site && gh issue create --body 'a\nb\nc\nd\ne'",
        );
        c.status = ToolCallStatus::Pending;
        t.on_call(&c);
        assert_eq!(
            shown(&t, 60, 10),
            [
                "$ gh issue create --body 'a",
                "b",
                "c",
                "d",
                "e'",
                "waiting for approval…"
            ]
        );
        t.on_update(&update("1", Some("completed"), "https://x/42", None));
        assert_eq!(
            shown(&t, 60, 10),
            [
                "$ gh issue create --body 'a",
                "b",
                "c",
                "… +2 lines",
                "https://x/42"
            ]
        );
    }

    #[test]
    fn commands_show_with_their_output_as_it_comes() {
        let mut t = TerminalState::default();
        t.on_call(&call("a", "read", "x"));
        assert!(!t.has_any(), "only commands");
        t.on_call(&call("1", "execute", "cargo test\n  --all"));
        t.on_update(&update("1", None, "\u{1b}[32mok\u{1b}[0m 1", None));
        assert_eq!(
            shown(&t, 40, 10),
            ["$ cargo test", "  --all", "ok 1", "running…"]
        );
        t.on_update(&update("1", Some("failed"), "ok 1\nboom\n", Some(101)));
        t.on_call(&call("2", "execute", "ls"));
        t.on_update(&update("2", Some("completed"), "50%\r100%", None));
        assert_eq!(
            shown(&t, 40, 10),
            [
                "$ cargo test",
                "  --all",
                "ok 1",
                "boom",
                "exit 101",
                "─".repeat(40).as_str(),
                "$ ls",
                "100%"
            ]
        );
    }

    #[test]
    fn the_end_shows_until_scrolled_back() {
        let mut t = TerminalState::default();
        t.on_call(&call("1", "execute", "seq 20"));
        let numbers: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
        t.on_update(&update("1", Some("completed"), &numbers.join("\n"), None));
        assert_eq!(shown(&t, 20, 3), ["18", "19", "20"]);
        t.key(KeyEvent::from(KeyCode::PageUp), 3);
        assert!(!t.following());
        assert_eq!(shown(&t, 20, 3), ["15", "16", "17"]);
        t.key(KeyEvent::from(KeyCode::Char('g')), 3);
        assert_eq!(shown(&t, 20, 3), ["$ seq 20", "1", "2"]);
        t.key(KeyEvent::from(KeyCode::Char('G')), 3);
        assert!(t.following());
        assert!(t.key(KeyEvent::from(KeyCode::Esc), 3));
    }

    #[test]
    fn long_lines_wrap_and_guests_are_named() {
        let mut t = TerminalState::default();
        let mut c = call("1", "execute", "echo");
        c.agent = Some("claude".into());
        t.on_call(&c);
        t.on_update(&update(
            "1",
            Some("completed"),
            "```\nabcdefghij\n```",
            None,
        ));
        assert_eq!(
            shown(&t, 6, 10),
            ["claude", " $ ech", "o", "abcdef", "ghij"]
        );
    }
}
