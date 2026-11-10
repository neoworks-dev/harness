//! The terminal panel: the commands the agent and its guests ran (`execute` tool calls). Each
//! is a header on a faint band (how it stands, what it is for, the command, how it ended and how
//! long it took) and its output under a gutter, in its own colors, as it comes; long lines are
//! cut at the edge and the panel scrolls sideways; a blank row between commands.
//!
//! A command Claude Code runs in the background ends its call at once, with where its output
//! goes (`Output is being written to: /tmp/…/<id>.output`); that file is tailed here as the
//! command's output, until the line `[exited with code N]` it ends with. The panel shows all the
//! commands, or one background command on its own.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use neo_protocol::{ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind};
use neo_renderer_ratatui::{Palette, highlight_shell, strip_cd};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use super::ansi::{Segs, Sgr, parse_line, plain, window};
use super::text::clean_line;
use crate::util::fit_line;

/// Commands kept, and lines of output kept per command (the end of it).
const RUNS: usize = 50;
const OUTPUT_LINES: usize = 400;
/// Cells a Left or Right moves the output sideways.
const HSCROLL: usize = 8;

#[derive(Debug, Clone)]
struct Run {
    id: String,
    agent: Option<String>,
    command: String,
    /// What the agent said it is for.
    description: Option<String>,
    /// Cleaned lines with their colors, the last [`OUTPUT_LINES`].
    output: Vec<Segs>,
    status: ToolCallStatus,
    exit: Option<i64>,
    /// Running in the background: its output comes from a file.
    job: Option<Job>,
    /// When its call was logged, and when an update said it ended (ms).
    started_ms: i64,
    ended_ms: Option<i64>,
}

/// A background command's output file and what has been read of it.
#[derive(Debug, Clone)]
struct Job {
    path: PathBuf,
    /// Bytes read so far, and the start of a line not ended yet.
    offset: u64,
    partial: Vec<u8>,
    /// Cleaned lines with their colors, the last [`OUTPUT_LINES`].
    output: Vec<Segs>,
    /// What the SGR sequences read so far have set, for the next lines.
    sgr: Sgr,
    started_ms: i64,
    /// From the file's last line; None while it runs.
    exit: Option<i64>,
    ended_ms: Option<i64>,
    /// Read while it was still running: it ended while this app looked on. A command replayed
    /// with the session's history is found ended at once and isn't listed.
    seen_running: bool,
    /// The file isn't there (cleaned up, or on another machine).
    gone: bool,
    /// The file ends with `[killed]`: the command was stopped, and it is dropped.
    killed: bool,
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
    /// The chat is scrolled up: the newest command shown is the one at that point (its run's
    /// id; None: there was none yet). None shows them all.
    upto: Option<Option<String>>,
    /// Cells the output is scrolled sideways (long lines are cut at the edge, not wrapped).
    hscroll: usize,
    /// The keys scrolled it up or down since the app last asked (the chat then follows it).
    scrolled: bool,
    /// The run on the bottom row of the last draw: where the chat follows it to.
    bottom: RefCell<Option<String>>,
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
        // Not a command, or one that was killed and dropped: passed over from now on.
        if call.kind != ToolKind::Execute {
            self.others.insert(call.tool_call_id.clone());
        }
        if self.others.contains(&call.tool_call_id) {
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
            ended_ms: None,
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
            let ended = matches!(status, ToolCallStatus::Completed | ToolCallStatus::Failed);
            if ended && run.ended_ms.is_none() {
                run.ended_ms = Some(ts);
            }
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
                    sgr: Sgr::default(),
                    started_ms: ts,
                    exit: None,
                    ended_ms: None,
                    seen_running: false,
                    gone: false,
                    killed: false,
                });
            }
            run.output = output_lines(&text, &mut Sgr::default());
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
        let look = Look::new(width, self.hscroll, cwd, p);
        if let Some(run) = self
            .view()
            .and_then(|id| self.runs.iter().find(|r| r.id == id))
        {
            let all = run_lines(run, &look);
            let max = all.len().saturating_sub(height);
            self.max_back.set(max);
            *self.bottom.borrow_mut() = Some(run.id.clone());
            let back = self.back.unwrap_or(0).min(max);
            let end = all.len() - back;
            let start = end.saturating_sub(height);
            let mut shown = all[start..end].to_vec();
            if pinned(shown.len(), &[(0, all.len())], start).is_some() {
                shown[0] = pin_line(run, &look, false);
            }
            return shown;
        }
        // Where the chat is scrolled to, the commands up to there.
        let runs = match &self.upto {
            None => &self.runs[..],
            Some(upto) => {
                let end = upto
                    .as_ref()
                    .and_then(|id| self.runs.iter().position(|r| r.id == *id))
                    .map_or(0, |at| at + 1);
                &self.runs[..end]
            }
        };
        // From the newest run back, only as many as fill the view; a blank row between runs.
        let mut blocks: Vec<Vec<Line<'static>>> = Vec::new();
        let mut ids: Vec<&str> = Vec::new();
        let mut have = 0;
        let mut complete = true;
        for (i, run) in runs.iter().enumerate().rev() {
            let mut lines = run_lines(run, &look);
            if i + 1 < runs.len() {
                lines.push(Line::default());
            }
            have += lines.len();
            blocks.push(lines);
            ids.push(&run.id);
            if have >= want {
                complete = i == 0;
                break;
            }
        }
        // Where each run starts and how many rows it has before the blank row.
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
        // The run the bottom row is in (or the blank row after).
        ids.reverse();
        let last = runs.iter().rposition(|&(at, _)| at < end);
        *self.bottom.borrow_mut() = last.and_then(|i| ids.get(i)).map(|id| id.to_string());
        let mut shown = all[start..end].to_vec();
        let linked = self.is_linked();
        let run_at = |i: usize| {
            ids.get(i)
                .and_then(|id| self.runs.iter().find(|r| r.id == *id))
        };
        let mut from = 0;
        if let Some(i) = pinned(shown.len(), &runs, start)
            && let Some(run) = run_at(i)
        {
            shown[0] = pin_line(run, &look, linked && last == Some(i));
            from = 1;
        }
        if linked
            && let Some(i) = last
            && let Some(run) = run_at(i)
        {
            mark_run(&mut shown, runs[i], start, from, run, &look);
        }
        shown
    }

    /// Whether the keys scrolled it since the last time this was asked.
    pub(crate) fn take_scrolled(&mut self) -> bool {
        std::mem::take(&mut self.scrolled)
    }

    /// The run on the bottom row of the last draw (its toolCallId).
    pub(crate) fn bottom_run(&self) -> Option<String> {
        self.bottom.borrow().clone()
    }

    /// Whether the panel and the chat are at the same command: one was scrolled away from the
    /// end and the other follows it. Following the newest output they are not tied together.
    fn is_linked(&self) -> bool {
        self.view().is_none() && (self.back.is_some() || self.upto.is_some())
    }

    /// The run both the panel and the chat mark, while they are tied together.
    pub(crate) fn linked_run(&self) -> Option<String> {
        self.is_linked().then(|| self.bottom_run()).flatten()
    }

    /// Keys while focused; true leaves the panel.
    pub(crate) fn key(&mut self, key: KeyEvent, page: usize) -> bool {
        let cur = self.back.unwrap_or(0);
        let max = self.max_back.get();
        let to = |n: usize| (n > 0).then_some(n.min(max));
        let before = self.back;
        let leave = self.move_by(key, cur, page, to);
        if self.back != before {
            self.scrolled = true;
        }
        leave
    }

    fn move_by(
        &mut self,
        key: KeyEvent,
        cur: usize,
        page: usize,
        to: impl Fn(usize) -> Option<usize>,
    ) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Up | KeyCode::Char('k') => self.back = to(cur + 1),
            KeyCode::Down | KeyCode::Char('j') => self.back = to(cur.saturating_sub(1)),
            KeyCode::PageUp => self.back = to(cur + page),
            KeyCode::PageDown => self.back = to(cur.saturating_sub(page)),
            KeyCode::Home | KeyCode::Char('g') => self.back = to(usize::MAX),
            KeyCode::End | KeyCode::Char('G') => self.back = None,
            // Long lines are cut at the edge: sideways to read them.
            KeyCode::Left | KeyCode::Char('h') => {
                self.hscroll = self.hscroll.saturating_sub(HSCROLL);
            }
            KeyCode::Right | KeyCode::Char('l') => self.hscroll += HSCROLL,
            KeyCode::Char('0') => self.hscroll = 0,
            _ => {}
        }
        false
    }

    /// Whether the view follows the newest output.
    pub(crate) fn following(&self) -> bool {
        self.back.is_none()
    }

    /// Whether a run of this toolCallId is kept.
    pub(crate) fn has_run(&self, id: &str) -> bool {
        self.runs.iter().any(|r| r.id == id)
    }

    /// The chat scrolled: show the commands up to the run `Some(id)` (`Some(None)`: none), or
    /// (None) all of them. Moving starts at the end of what is shown again.
    pub(crate) fn set_upto(&mut self, upto: Option<Option<String>>) {
        if self.upto != upto {
            self.upto = upto;
            self.back = None;
        }
    }

    /// Whether the commands end where the chat is scrolled to, not with the newest.
    pub(crate) fn with_chat(&self) -> bool {
        self.upto.is_some()
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
        // A command that was killed isn't kept: not in the panel, not in the list.
        let before = self.runs.len();
        let others = &mut self.others;
        self.runs.retain(|r| {
            let killed = r.job.as_ref().is_some_and(|j| j.killed);
            if killed {
                others.insert(r.id.clone());
            }
            !killed
        });
        changed || self.runs.len() != before
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
                    .map(plain)
                    .find(|l| !l.trim().is_empty()),
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
        let mut lines = output_lines(&text, &mut self.sgr);
        // A command that was stopped ends its file with `[killed]`.
        if lines.iter().any(|l| plain(l).trim() == "[killed]") {
            self.killed = true;
            return true;
        }
        // `[exited with code N]` ends the file, after a blank line.
        match lines.last().and_then(|l| exit_code(&plain(l))) {
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

/// Which run (its place in `runs`) the top row, row `start` of the rows, is inside when that
/// run's header has scrolled off: its pinned line then takes the top's place, so the output
/// keeps saying whose it is. `runs` are each run's first row and its rows before the rule
/// under it; `shown` is how many rows are on screen.
fn pinned(shown: usize, runs: &[(usize, usize)], start: usize) -> Option<usize> {
    if shown < 2 {
        return None;
    }
    runs.iter()
        .position(|&(first, rows)| start > first && start < first + rows)
}

/// The line that stays at the top while a run's output scrolls: what the command is for, or
/// the command itself when nobody said.
fn pin_line(run: &Run, look: &Look, linked: bool) -> Line<'static> {
    header(run, look, true, linked).swap_remove(0)
}

/// Puts the bar in the margin of the run the chat is at: on its header and down its output's
/// gutter, as the chat marks its item. `at` is the run's first row among the rows, `start` the
/// first row shown, `from` the first shown row that isn't a pinned line.
fn mark_run(
    shown: &mut [Line<'static>],
    (at, rows): (usize, usize),
    start: usize,
    from: usize,
    run: &Run,
    look: &Look,
) {
    let head = header(run, look, false, true);
    let bar = Span::styled("▎ ", Style::new().fg(look.p.accent));
    for (i, line) in shown.iter_mut().enumerate().skip(from) {
        let row = start + i;
        if row < at || row >= at + rows {
            continue;
        }
        match head.get(row - at) {
            Some(h) => *line = h.clone(),
            None => {
                if !line.spans.is_empty() {
                    line.spans[0] = bar.clone();
                }
            }
        }
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

/// Output as lines to draw, with their colors (`sgr` goes on from earlier output): other escape
/// sequences gone, a carriage return keeping what came after it (progress bars), a fence around
/// it (some adapters send one) dropped.
fn output_lines(text: &str, sgr: &mut Sgr) -> Vec<Segs> {
    let mut lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    if lines.len() >= 2 && lines[0].starts_with("```") && lines[lines.len() - 1] == "```" {
        lines = lines[1..lines.len() - 1].to_vec();
    }
    if lines == [""] {
        return Vec::new();
    }
    let skip = lines.len().saturating_sub(OUTPUT_LINES);
    lines[..skip].iter().for_each(|l| drop(parse_line(l, sgr)));
    lines[skip..]
        .iter()
        .map(|l| {
            let l = l.trim_end_matches('\r');
            parse_line(l.rsplit('\r').next().unwrap_or(l), sgr)
        })
        .collect()
}

/// How the runs are drawn: the width, how far the output is scrolled sideways, the session's
/// directory, the colors, and the header's band (none without true colors).
struct Look<'a> {
    width: usize,
    hscroll: usize,
    cwd: Option<&'a str>,
    p: &'a Palette,
    band: Option<Color>,
    /// Output without colors of its own: a step back from the text color.
    output: Style,
}

impl<'a> Look<'a> {
    fn new(width: usize, hscroll: usize, cwd: Option<&'a str>, p: &'a Palette) -> Look<'a> {
        let base = if p.panel == Color::Reset {
            p.bg
        } else {
            p.panel
        };
        let output =
            crate::chat::mix(p.fg, base, 0.22).map_or(Style::new(), |c| Style::new().fg(c));
        Look {
            width,
            hscroll,
            cwd,
            p,
            band: crate::chat::mix(base, p.fg, 0.07),
            output,
        }
    }
}

/// How a run went, as its header's right end says it: `✓ 0.8s`, `✗ exit 1`, `● running`.
fn outcome(run: &Run, p: &Palette) -> (Span<'static>, Span<'static>) {
    let job = run.job.as_ref().filter(|j| !j.gone);
    let (status, exit, ended) = match job {
        Some(j) => {
            let status = match j.exit {
                None => ToolCallStatus::InProgress,
                Some(0) => ToolCallStatus::Completed,
                Some(_) => ToolCallStatus::Failed,
            };
            (status, j.exit, j.ended_ms.map(|e| e - j.started_ms))
        }
        None => (
            run.status,
            run.exit,
            // A call whose start wasn't logged (only in tests) says nothing of how long.
            run.ended_ms
                .filter(|_| run.started_ms > 0)
                .map(|e| e - run.started_ms),
        ),
    };
    let took = ended.filter(|ms| *ms >= 0).map(took);
    let style = |c: Color| Style::new().fg(c);
    match status {
        ToolCallStatus::Pending => (
            Span::styled("?", style(p.warning).add_modifier(Modifier::BOLD)),
            Span::styled("waiting for approval", style(p.warning)),
        ),
        ToolCallStatus::InProgress => (
            Span::styled("●", style(p.info)),
            Span::styled("running", style(p.info)),
        ),
        ToolCallStatus::Completed => (
            Span::styled("✓", style(p.success)),
            Span::styled(took.unwrap_or_default(), style(p.muted)),
        ),
        ToolCallStatus::Failed => {
            let what = exit.map_or_else(|| "failed".to_string(), |code| format!("exit {code}"));
            let what = match took {
                Some(t) => format!("{what} · {t}"),
                None => what,
            };
            (
                Span::styled("✗", style(p.danger).add_modifier(Modifier::BOLD)),
                Span::styled(what, style(p.danger)),
            )
        }
    }
}

/// `0.8s`, `12s`, `1m 12s`.
fn took(ms: i64) -> String {
    match ms {
        ..1000 => format!("{:.1}s", ms as f64 / 1000.0),
        1000..60_000 => format!("{}s", ms / 1000),
        _ => format!("{}m {}s", ms / 60_000, ms / 1000 % 60),
    }
}

/// The run's header on a band across the panel: its state, what it is for in bold and the
/// command after it muted (the command highlighted when there is no description), and how it
/// went at the right end. One row, cut at the edge; a command waiting for approval is read in
/// full, its lines under it.
fn header(run: &Run, look: &Look, pin: bool, linked: bool) -> Vec<Line<'static>> {
    let p = look.p;
    // The run the chat is at has the bar in the margin, as the chat's item has.
    let bar = if linked { 2 } else { 0 };
    let width = look.width.saturating_sub(bar);
    let (icon, how) = outcome(run, p);
    let mut spans = vec![icon, Span::raw(" ")];
    if let Some(agent) = &run.agent {
        spans.push(Span::styled(format!("{agent} "), Style::new().fg(p.muted)));
    }
    let command = clean_command(strip_cd(&run.command, look.cwd));
    match &run.description {
        Some(description) => {
            spans.push(Span::styled(
                format!("{description}  "),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            // The pinned line says what it is for; the command is under it in the header.
            if !pin {
                spans.push(Span::styled(command, Style::new().fg(p.muted)));
            }
        }
        None => {
            spans.push(Span::styled(
                "$ ",
                Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            ));
            spans.extend(highlight_shell(&command, p, Style::new()));
        }
    }
    let how_w = super::text::width(&how.content);
    let mut lines = if run.status == ToolCallStatus::Pending {
        let mut lines = wrap_spans(spans, width);
        lines.push(Line::from(vec![Span::raw("  "), how]));
        lines
    } else {
        let mut lines = wrap_spans(spans, usize::MAX);
        let more = lines.len() > 1;
        let mut first = lines.swap_remove(0);
        if more {
            first
                .spans
                .push(Span::styled(" …", Style::new().fg(p.muted)));
        }
        let room = width.saturating_sub(if how_w > 0 { how_w + 2 } else { 0 });
        let mut first = fit_line(first, room);
        if how_w > 0 {
            let pad = width.saturating_sub(first.width() + how_w);
            first.spans.push(Span::raw(" ".repeat(pad)));
            first.spans.push(how);
        }
        vec![first]
    };
    if linked {
        for line in &mut lines {
            line.spans.insert(0, Span::styled("▎ ", Style::new().fg(p.accent)));
        }
    }
    if let Some(bg) = look.band {
        for line in &mut lines {
            let pad = look.width.saturating_sub(line.width());
            line.spans.push(Span::raw(" ".repeat(pad)));
            for span in &mut line.spans {
                span.style = span.style.bg(bg);
            }
        }
    }
    lines
}

/// A line of output under a muted gutter, in its own colors, cut at the edge (scrolled sideways
/// as far as the panel is).
fn output_row(line: &Segs, look: &Look) -> Line<'static> {
    let room = look.width.saturating_sub(2);
    let (mut spans, cut) = window(line, look.hscroll, room, look.output, look.p);
    if cut && room > 0 {
        // The last cell says there is more.
        let less = room - 1;
        spans = window(line, look.hscroll, less, look.output, look.p).0;
        spans.push(Span::styled("…", Style::new().fg(look.p.muted)));
    }
    let mut out = vec![Span::styled("│ ", Style::new().fg(look.p.border))];
    out.extend(spans);
    Line::from(out)
}

/// The header, then the output under it. A background command shows what its file holds
/// instead of its call's result.
fn run_lines(run: &Run, look: &Look) -> Vec<Line<'static>> {
    let mut out = header(run, look, false, false);
    let output = match run.job.as_ref().filter(|j| !j.gone) {
        Some(job) => &job.output,
        None => &run.output,
    };
    // Blank lines at the end say nothing.
    let end = output
        .iter()
        .rposition(|l| !plain(l).trim().is_empty())
        .map_or(0, |i| i + 1);
    out.extend(output[..end].iter().map(|line| output_row(line, look)));
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
mod tests;
