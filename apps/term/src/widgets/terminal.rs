//! The terminal panel: the commands the agent and its guests ran (`execute` tool calls), as
//! a shell would show them: `$ command`, its output as it comes, and how it ended.
//!
//! A command Claude Code runs in the background ends its call at once, with where its output
//! goes (`Output is being written to: /tmp/…/<id>.output`); that file is tailed here as the
//! command's output, until the line `[exited with code N]` it ends with. The panel shows all the
//! commands, or one background command on its own.

use std::cell::Cell;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use neo_protocol::{ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind};
use neo_renderer_ratatui::{Palette, highlight_shell, strip_cd};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use super::text::clean_line;
use crate::util::fit_line;

/// Commands kept, and lines of output kept per command (the end of it).
const RUNS: usize = 50;
const OUTPUT_LINES: usize = 400;

#[derive(Debug, Clone)]
struct Run {
    id: String,
    agent: Option<String>,
    command: String,
    /// What the agent said it is for.
    description: Option<String>,
    /// Cleaned lines, the last [`OUTPUT_LINES`].
    output: Vec<String>,
    status: ToolCallStatus,
    exit: Option<i64>,
    /// Running in the background: its output comes from a file.
    job: Option<Job>,
    /// When its call was logged (ms).
    started_ms: i64,
}

/// A background command's output file and what has been read of it.
#[derive(Debug, Clone)]
struct Job {
    path: PathBuf,
    /// Bytes read so far, and the start of a line not ended yet.
    offset: u64,
    partial: Vec<u8>,
    /// Cleaned lines, the last [`OUTPUT_LINES`].
    output: Vec<String>,
    started_ms: i64,
    /// From the file's last line; None while it runs.
    exit: Option<i64>,
    ended_ms: Option<i64>,
    /// Read while it was still running: it ended while this app looked on. A command replayed
    /// with the session's history is found ended at once and isn't listed.
    seen_running: bool,
    /// The file isn't there (cleaned up, or on another machine).
    gone: bool,
}

/// A background command, as the list under the prompt shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRow {
    pub id: String,
    pub agent: Option<String>,
    pub command: String,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub exit: Option<i64>,
    /// Its last line of output.
    pub last: Option<String>,
}

/// How long a finished background command stays in the list.
pub const JOB_LINGER_MS: i64 = 5000;
/// Bytes read from an output file at a time, so a flood doesn't stall a frame.
const READ_CHUNK: u64 = 256 * 1024;

#[derive(Debug, Default)]
pub(crate) struct TerminalState {
    runs: Vec<Run>,
    /// The background command shown on its own (its run's id); None shows all commands.
    view: Option<String>,
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

    /// A call as it is logged at `ts`: a command started then.
    pub(crate) fn on_call_at(&mut self, call: &ToolCall, ts: i64) {
        let new = !self.runs.iter().any(|r| r.id == call.tool_call_id);
        self.on_call(call);
        if new && let Some(run) = self.runs.iter_mut().find(|r| r.id == call.tool_call_id) {
            run.started_ms = ts;
        }
    }

    /// The commands of the session's own agent that its turn waits for (not a subagent's, not
    /// one in the background): their call ids and when they started.
    pub(crate) fn foreground(&self) -> Vec<(String, i64)> {
        self.runs
            .iter()
            .filter(|r| {
                r.agent.is_none()
                    && r.job.is_none()
                    && matches!(
                        r.status,
                        ToolCallStatus::Pending | ToolCallStatus::InProgress
                    )
            })
            .map(|r| (r.id.clone(), r.started_ms))
            .collect()
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
        let description = description_of(call.raw_input.as_ref());
        if let Some(run) = self.runs.iter_mut().find(|r| r.id == call.tool_call_id) {
            run.command = command;
            run.description = description.or(run.description.take());
            run.status = call.status;
            return;
        }
        self.runs.push(Run {
            id: call.tool_call_id.clone(),
            agent: call.agent.clone(),
            command,
            description,
            output: Vec::new(),
            status: call.status,
            exit: None,
            job: None,
            started_ms: 0,
        });
        if self.runs.len() > RUNS {
            self.runs.remove(0);
        }
    }

    /// Output replaces what was there (a running command's updates carry the tail so far). A
    /// result that says the command went to the background starts tailing its file; `ts` is
    /// when (the event's time).
    pub(crate) fn on_update(&mut self, update: &ToolCallUpdate, ts: i64) {
        if self.others.contains(&update.tool_call_id) {
            return;
        }
        let Some(run) = self.runs.iter_mut().find(|r| r.id == update.tool_call_id) else {
            return;
        };
        if let Some(command) = command_of(update.raw_input.as_ref()) {
            run.command = command;
        }
        if let Some(description) = description_of(update.raw_input.as_ref()) {
            run.description = Some(description);
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
            let text = text.join("\n");
            if run.job.is_none()
                && let Some(path) = background_output(&text)
            {
                run.job = Some(Job {
                    path,
                    offset: 0,
                    partial: Vec::new(),
                    output: Vec::new(),
                    started_ms: ts,
                    exit: None,
                    ended_ms: None,
                    seen_running: false,
                    gone: false,
                });
            }
            run.output = output_lines(&text);
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
        if let Some(run) = self
            .view()
            .and_then(|id| self.runs.iter().find(|r| r.id == id))
        {
            let all = job_lines(run, width, cwd, p);
            let max = all.len().saturating_sub(height);
            self.max_back.set(max);
            let back = self.back.unwrap_or(0).min(max);
            let end = all.len() - back;
            let start = end.saturating_sub(height);
            let mut shown = all[start..end].to_vec();
            pin_head(&mut shown, &all, &[(0, all.len())], start);
            return shown;
        }
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
        // Where each run starts and how many rows it has before its rule.
        let mut runs = Vec::new();
        let mut at = 0;
        for (i, block) in blocks.iter().rev().enumerate() {
            let rule = usize::from(i + 1 < blocks.len());
            runs.push((at, block.len() - rule));
            at += block.len();
        }
        let all: Vec<Line<'static>> = blocks.into_iter().rev().flatten().collect();
        // Up as far as there is; while some runs were left out, at least what was drawn.
        let max = all.len().saturating_sub(height);
        self.max_back
            .set(if complete { max } else { max.max(want) });
        let back = self.back.unwrap_or(0).min(max);
        let end = all.len() - back;
        let start = end.saturating_sub(height);
        let mut shown = all[start..end].to_vec();
        pin_head(&mut shown, &all, &runs, start);
        shown
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

    /// Reads what the running background commands wrote since the last time; true when any
    /// wrote something or ended.
    pub(crate) fn poll_jobs(&mut self, now_ms: i64) -> bool {
        let mut changed = false;
        for job in self.runs.iter_mut().filter_map(|r| r.job.as_mut()) {
            if job.exit.is_none() && !job.gone {
                changed |= job.poll(now_ms);
            }
        }
        changed
    }

    /// The background commands to list: those running, and those that ended a moment ago.
    pub(crate) fn jobs(&self, now_ms: i64) -> Vec<JobRow> {
        self.runs
            .iter()
            .filter_map(|r| r.job.as_ref().map(|j| (r, j)))
            .filter(|(_, j)| {
                // Listed once read and found running (not before the first read: a replayed
                // one may have ended long ago).
                !j.gone
                    && j.seen_running
                    && j.ended_ms.is_none_or(|end| now_ms - end < JOB_LINGER_MS)
            })
            .map(|(r, j)| JobRow {
                id: r.id.clone(),
                agent: r.agent.clone(),
                command: r.command.clone(),
                started_ms: j.started_ms,
                ended_ms: j.ended_ms,
                exit: j.exit,
                last: j
                    .output
                    .iter()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .cloned(),
            })
            .collect()
    }

    /// Stops a running background command: SIGTERM to the process groups writing its file
    /// (Claude Code runs each in a group of its own and then ends the file with the exit
    /// code). The number of groups signalled, or why there was none.
    pub(crate) fn stop_job(&self, id: &str) -> Result<usize, String> {
        let job = self
            .runs
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.job.as_ref())
            .filter(|j| j.exit.is_none())
            .ok_or("it has ended")?;
        let groups = writers(&job.path)?;
        if groups.is_empty() {
            return Err("no process writes its output".into());
        }
        for pgid in &groups {
            std::process::Command::new("kill")
                .args(["-TERM", "--", &format!("-{pgid}")])
                .status()
                .map_err(|e| format!("kill: {e}"))?;
        }
        Ok(groups.len())
    }

    /// Shows one background command on its own (`Some(id)`), or all commands.
    pub(crate) fn set_view(&mut self, id: Option<String>) {
        if self.view != id {
            self.view = id;
            self.back = None;
        }
    }

    /// The background command shown on its own, while it is still there.
    pub(crate) fn view(&self) -> Option<&str> {
        self.view
            .as_deref()
            .filter(|id| self.runs.iter().any(|r| r.id == *id && r.job.is_some()))
    }
}

impl Job {
    /// Reads the file from where the last read ended; true when there was something new.
    fn poll(&mut self, now_ms: i64) -> bool {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            // Not written yet, or gone: gone for good once something was read, or after a while.
            Err(_) => {
                if self.offset > 0 || now_ms - self.started_ms > 10_000 {
                    self.gone = true;
                }
                return false;
            }
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut bytes = Vec::new();
        let _ = file.take(READ_CHUNK).read_to_end(&mut bytes);
        self.offset += bytes.len() as u64;
        self.partial.extend(bytes);
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else {
            // No line ended: it runs on.
            self.seen_running = true;
            return false;
        };
        let done: Vec<u8> = self.partial.drain(..=end).collect();
        let text = String::from_utf8_lossy(&done);
        let mut lines = output_lines(&text);
        // `[exited with code N]` ends the file, after a blank line.
        match lines.last().and_then(|l| exit_code(l)) {
            Some(code) => {
                lines.pop();
                if lines.last().is_some_and(|l| l.is_empty()) {
                    lines.pop();
                }
                self.exit = Some(code);
                self.ended_ms = Some(now_ms);
            }
            None => self.seen_running = true,
        }
        self.output.extend(lines);
        let skip = self.output.len().saturating_sub(OUTPUT_LINES);
        self.output.drain(..skip);
        true
    }
}

/// The output file a background command's result names: Claude Code's `Command running in
/// background with ID: … Output is being written to: <path>.`
fn background_output(text: &str) -> Option<PathBuf> {
    if !text.contains("running in background") {
        return None;
    }
    let (_, rest) = text.split_once("Output is being written to: ")?;
    let path = rest.split_whitespace().next()?.trim_end_matches('.');
    path.starts_with('/').then(|| PathBuf::from(path))
}

/// The process groups of the processes whose output (fd 1 or 2) goes to `path`, found in
/// `/proc`; never this app's own group.
fn writers(path: &std::path::Path) -> Result<Vec<i64>, String> {
    let procs = std::fs::read_dir("/proc").map_err(|_| "no /proc to find its process in")?;
    let own = pgid_of("self");
    let mut groups = Vec::new();
    for entry in procs.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let writes = ["1", "2"].iter().any(|fd| {
            std::fs::read_link(format!("/proc/{pid}/fd/{fd}")).is_ok_and(|target| target == path)
        });
        if let Some(pgid) = pgid_of(pid).filter(|&g| writes && g > 1 && Some(g) != own)
            && !groups.contains(&pgid)
        {
            groups.push(pgid);
        }
    }
    Ok(groups)
}

/// A process's group, from `/proc/<pid>/stat` (its fifth field, after the name in parens).
fn pgid_of(pid: &str) -> Option<i64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(2)?.parse().ok()
}

/// When the top of `shown` (row `start` of `all`) is inside a run whose `$ command` row has
/// scrolled off, that row takes the top's place, so the output keeps saying whose it is. `runs`
/// are each run's first row in `all` and its rows before the rule under it.
fn pin_head(
    shown: &mut [Line<'static>],
    all: &[Line<'static>],
    runs: &[(usize, usize)],
    start: usize,
) {
    if shown.len() < 2 {
        return;
    }
    let inside = runs
        .iter()
        .find(|&&(first, rows)| start > first && start < first + rows);
    if let Some(&(first, _)) = inside {
        shown[0] = all[first].clone();
    }
}

/// `[exited with code 0]`: the code.
fn exit_code(line: &str) -> Option<i64> {
    line.trim()
        .strip_prefix("[exited with code ")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// `rawInput.description`: what the agent said the command is for (Claude Code's Bash), on one
/// line.
fn description_of(input: Option<&serde_json::Value>) -> Option<String> {
    let text = input?.get("description")?.as_str()?;
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
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

/// A background command on its own: `$ command`, what it wrote to its file, and `running…`,
/// `exit 0` or a red `exit 1`.
fn job_lines(run: &Run, width: usize, cwd: Option<&str>, p: &Palette) -> Vec<Line<'static>> {
    let mut out = command_lines(run, width, cwd, p);
    let Some(job) = &run.job else {
        return out;
    };
    for line in &job.output {
        out.extend(wrap_spans(vec![Span::raw(line.clone())], width));
    }
    let (text, style) = match job.exit {
        None => ("running…".to_string(), Style::new().fg(p.muted)),
        Some(0) => ("exit 0".to_string(), Style::new().fg(p.muted)),
        Some(code) => (format!("exit {code}"), Style::new().fg(p.danger)),
    };
    out.push(Line::from(Span::styled(text, style)));
    out
}

/// `$ command`, highlighted; one row once it started.
fn command_lines(run: &Run, width: usize, cwd: Option<&str>, p: &Palette) -> Vec<Line<'static>> {
    let mut lead = Vec::new();
    if let Some(agent) = &run.agent {
        lead.push(Span::styled(format!("{agent} "), Style::new().fg(p.muted)));
    }
    // What it is for, in front of it: `Build term  $ cargo build`.
    if let Some(description) = &run.description {
        lead.push(Span::styled(
            format!("{description}  "),
            Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
        ));
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
    // What waits for approval is read in full; after that the output is what matters, and the
    // command is one row: its first line, cut at the edge.
    if run.status == ToolCallStatus::Pending {
        return wrap_spans(spans, width);
    }
    let mut lines = wrap_spans(spans, usize::MAX);
    let more = lines.len() > 1;
    let mut first = lines.swap_remove(0);
    if more {
        first
            .spans
            .push(Span::styled(" …", Style::new().fg(p.muted)));
    }
    vec![fit_line(first, width)]
}

/// `$ command` (highlighted, continuation lines under it while it waits for approval, one row
/// once it started), the output, and a muted `exit 1` (danger) or `running…` line.
/// A background command shows what its file holds instead of its call's result.
fn run_lines(run: &Run, width: usize, cwd: Option<&str>, p: &Palette) -> Vec<Line<'static>> {
    if run.job.as_ref().is_some_and(|j| !j.gone) {
        return job_lines(run, width, cwd, p);
    }
    let mut out = command_lines(run, width, cwd, p);
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

    #[test]
    fn the_agents_description_goes_in_front_of_the_command() {
        let mut t = TerminalState::default();
        let mut c = call("1", "execute", "cargo build");
        c.raw_input = Some(json!({"command": "cargo build", "description": "Build\n  term"}));
        t.on_call(&c);
        assert_eq!(shown(&t, 60, 10), ["Build term  $ cargo build", "running…"]);
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
        t.on_update(&update("1", Some("completed"), "https://x/42", None), 0);
        assert_eq!(
            shown(&t, 60, 10),
            ["$ gh issue create --body 'a …", "https://x/42"]
        );
    }

    #[test]
    fn commands_show_with_their_output_as_it_comes() {
        let mut t = TerminalState::default();
        t.on_call(&call("a", "read", "x"));
        assert!(!t.has_any(), "only commands");
        t.on_call(&call("1", "execute", "cargo test\n  --all"));
        t.on_update(&update("1", None, "\u{1b}[32mok\u{1b}[0m 1", None), 0);
        assert_eq!(shown(&t, 40, 10), ["$ cargo test …", "ok 1", "running…"]);
        t.on_update(&update("1", Some("failed"), "ok 1\nboom\n", Some(101)), 0);
        t.on_call(&call("2", "execute", "ls"));
        t.on_update(&update("2", Some("completed"), "50%\r100%", None), 0);
        assert_eq!(
            shown(&t, 40, 10),
            [
                "$ cargo test …",
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
        t.on_update(
            &update("1", Some("completed"), &numbers.join("\n"), None),
            0,
        );
        // The command stays on top while its output scrolls under it.
        assert_eq!(shown(&t, 20, 3), ["$ seq 20", "19", "20"]);
        t.key(KeyEvent::from(KeyCode::PageUp), 3);
        assert!(!t.following());
        assert_eq!(shown(&t, 20, 3), ["$ seq 20", "16", "17"]);
        t.key(KeyEvent::from(KeyCode::Char('g')), 3);
        assert_eq!(shown(&t, 20, 3), ["$ seq 20", "1", "2"]);
        t.key(KeyEvent::from(KeyCode::Char('G')), 3);
        assert!(t.following());
        assert!(t.key(KeyEvent::from(KeyCode::Esc), 3));
    }

    #[test]
    fn the_command_on_top_is_the_one_whose_output_shows() {
        let mut t = TerminalState::default();
        t.on_call(&call("1", "execute", "seq 5"));
        t.on_update(&update("1", Some("completed"), "1\n2\n3\n4\n5", None), 0);
        t.on_call(&call("2", "execute", "seq 2"));
        t.on_update(&update("2", Some("completed"), "1\n2", None), 0);
        let rule = "─".repeat(10);
        // On the rule, nothing is pinned: the next command starts right under it.
        assert_eq!(shown(&t, 10, 4), [rule.as_str(), "$ seq 2", "1", "2"]);
        t.key(KeyEvent::from(KeyCode::Up), 4);
        t.key(KeyEvent::from(KeyCode::Up), 4);
        assert_eq!(shown(&t, 10, 4), ["$ seq 5", "5", rule.as_str(), "$ seq 2"]);
    }

    #[test]
    fn long_lines_wrap_and_guests_are_named() {
        let mut t = TerminalState::default();
        let mut c = call("1", "execute", "echo");
        c.agent = Some("claude".into());
        t.on_call(&c);
        t.on_update(
            &update("1", Some("completed"), "```\nabcdefghij\n```", None),
            0,
        );
        // The output wraps; the command that ran is cut to a row.
        assert_eq!(shown(&t, 6, 10), ["claud…", "abcdef", "ghij"]);
        assert_eq!(shown(&t, 20, 10), ["claude $ echo", "abcdefghij"]);
    }

    /// Claude Code's result for a command it runs in the background.
    fn backgrounded(path: &std::path::Path) -> String {
        format!(
            "Command running in background with ID: b1. Output is being written to: {}. You \
             will be notified when it completes.",
            path.display()
        )
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("neo-term-jobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_background_commands_file_is_tailed_until_it_ends() {
        let path = temp("live.output");
        std::fs::write(&path, "tick 1\n").unwrap();
        let mut t = TerminalState::default();
        t.on_call(&call("1", "execute", "for i in 1 2; do echo tick $i; done"));
        t.on_update(
            &update("1", Some("completed"), &backgrounded(&path), None),
            1_000,
        );
        assert!(t.jobs(1_000).is_empty(), "not before its file was read");
        assert!(t.poll_jobs(1_250));
        let jobs = t.jobs(1_250);
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            (jobs[0].exit, jobs[0].last.as_deref()),
            (None, Some("tick 1"))
        );
        // Among all commands it shows what its file holds.
        assert_eq!(
            shown(&t, 60, 10),
            [
                "$ for i in 1 2; do echo tick $i; done",
                "tick 1",
                "running…"
            ]
        );

        // A half-written line waits for its end.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        write!(f, "tick").unwrap();
        assert!(!t.poll_jobs(1_500));
        write!(f, " 2\n\n[exited with code 3]\n").unwrap();
        assert!(t.poll_jobs(1_750));
        let jobs = t.jobs(1_750);
        assert_eq!((jobs[0].exit, jobs[0].ended_ms), (Some(3), Some(1_750)));
        t.set_view(Some("1".into()));
        assert_eq!(t.view(), Some("1"));
        assert_eq!(
            shown(&t, 60, 10),
            [
                "$ for i in 1 2; do echo tick $i; done",
                "tick 1",
                "tick 2",
                "exit 3"
            ]
        );
        // It lingers a moment, then leaves the list.
        assert!(t.jobs(1_750 + JOB_LINGER_MS).is_empty());
        std::fs::remove_file(&path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stopping_signals_the_group_that_writes_the_file() {
        use std::os::unix::process::CommandExt;
        let path = temp("stop.output");
        let out = std::fs::File::create(&path).unwrap();
        // Like Claude Code: a group of its own, its output into the file.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "echo up; sleep 30"])
            .stdout(out)
            .process_group(0)
            .spawn()
            .unwrap();
        let mut t = TerminalState::default();
        t.on_call(&call("1", "execute", "sleep 30"));
        t.on_update(
            &update("1", Some("completed"), &backgrounded(&path), None),
            0,
        );
        assert_eq!(t.stop_job("1"), Ok(1));
        let status = child.wait().unwrap();
        assert!(!status.success(), "{status:?}");
        assert!(t.stop_job("nope").is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_replayed_background_command_that_ended_is_not_listed() {
        let ended = temp("ended.output");
        std::fs::write(&ended, "ok\n\n[exited with code 0]\n").unwrap();
        let mut t = TerminalState::default();
        t.on_call(&call("1", "execute", "make"));
        t.on_update(
            &update("1", Some("completed"), &backgrounded(&ended), None),
            0,
        );
        // And one whose file was cleaned up.
        t.on_call(&call("2", "execute", "make again"));
        let gone = temp("gone.output");
        t.on_update(
            &update("2", Some("completed"), &backgrounded(&gone), None),
            0,
        );
        t.poll_jobs(60_000);
        assert!(t.jobs(60_000).is_empty());
        // Among all commands it still shows what its file held.
        let all = shown(&t, 200, 20);
        assert_eq!(all[..3], ["$ make", "ok", "exit 0"]);
        std::fs::remove_file(&ended).unwrap();
    }
}
