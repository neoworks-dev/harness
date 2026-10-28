//! The side panels and the usage line under the prompt: plan (todo lists of the agent and its
//! guests), files (git status plus what the agent touched, with diffs) and usage (context and
//! cost of the session's own agent). Each is state fed from session events plus a draw function; nothing here blocks the
//! UI thread (git runs on a background thread).
//!
//! This is the contract the shell (`app`) uses. Behavior worth knowing:
//!
//! - **Plan**: every `plan` event replaces that agent's list (no `agent` = the session's own;
//!   empty = cleared). With several lists each gets a small header. When the panel is too small
//!   the in-progress entry stays in view and the rest is counted (`… N more`). Focused:
//!   Up/Down/j/k scroll, PgUp/PgDn, g/G.
//! - **Usage**: the newest `usage` event of the session's own agent (guests' are left out). Context fill is muted, accent from 60%,
//!   danger from 85%.
//! - **Files**: `git status` runs on a thread (debounced 200 ms after `on_file_changed`, turn
//!   ends and tool diffs; every 10 s anyway; one run at a time). Entries are git's changes plus
//!   files an agent changed this session (from `diff` content of tool calls), tagged
//!   `(agent)` / `(claude)`. Outside a repository only the touched files are listed and
//!   `branch()` is None. Drawn as a tree: folders first (a folder holding only one folder is
//!   joined with it, `apps/term/`), a folded one with the line counts of what is in it.
//!   Focused: Up/Down/j/k move, g/G ends, Enter or Space folds a folder and opens a file,
//!   Right/l opens a folder or goes into it, Left/h folds it or goes up to its folder.
//! - **File view**: Enter on a file opens the diff overlay with the whole file as context and
//!   unified (its changes marked in place); a file git sees no change in shows as it is.
//! - **Syntax highlighting** (the renderer's `syntax` module, shared with the chat's diffs): the
//!   diff and file view color code by bat's grammar for the file name (`+++ b/` headers for a
//!   pull request's diff), old and new lines parsed apart and each hunk starting over. Only
//!   what is on screen (and a screen above and below) is worked out, on a thread (`lit`): the
//!   code shows plain until it arrives. A far jump into a long hunk starts a fresh parse a little
//!   above the view, a guess that an exact parse replaces when it passes by. The colors are
//!   picked for a dark or light background, not the theme's few; changed lines keep their tint
//!   behind them and the sign its color. Without true colors code stays plain.
//! - **Diff overlay**: `git diff HEAD -- path` (the whole file for untracked ones; at most a
//!   million lines) loaded in the background and drawn over the chat, side by side from 90
//!   columns on. While it is open it takes every key whatever panel is focused, vim's way, with
//!   a cursor (on the new side of the split view, the old one where only it has a line):
//!   h/j/k/l and the arrows, w/b/e by words, 0/^/$ in the line, gg/G the ends, H/M/L the view's
//!   top, middle and bottom; Ctrl-D/Ctrl-U half a page, PgUp/PgDn and Ctrl-F/Ctrl-B a page;
//!   Ctrl-E/Ctrl-Y (and the wheel) scroll a line, the cursor only moving to stay in view; a count
//!   first repeats (`5j`) or names a line (`120G`); n/p or }/{ the next / previous hunk; `/`
//!   searches (smart case, Enter jumps, Esc gives up), then n/N the next / previous match; `v`
//!   and `V` select characters and lines (Esc leaves them). The leader is Space or `\`:
//!   `<leader>c` comments on the cursor's line or the selection's lines — a block above them
//!   splitting the file, written in place (Enter keeps it, Esc gives up, emptied it goes) and
//!   put in the prompt as `@path:start-end text` ([`Widgets::take_comment_changes`]); on a
//!   commented line it edits that comment, and the prompt's copy follows. The comments stay with
//!   their file while the view is closed, until a message carrying them is sent. Long lines
//!   wrap under their line number (each side of the split view on its own, the shorter one
//!   going on blank in its tint), and `<leader>w` switches to cutting them with a `…` instead,
//!   the view following the cursor sideways; s switches split and unified; f reloads with the
//!   whole file as context (and back); `<leader>e` asks for the file in the editor at the
//!   cursor's line ([`Widgets::take_edit`]); Esc or q close it. Closing returns `Redraw`, so
//!   focus stays on the files panel (to pick the next file); a second Esc there returns `Leave`.
//! - The GitHub view (`crate::github`) reuses the diff view for a pull request's diff.
//! - Keys the panels don't use return `Redraw` (harmless). Esc and q in a panel return `Leave`.

mod diff;
mod files;
mod git;
mod lit;
mod plan;
mod terminal;
mod text;
mod todos;
mod usage;

pub(crate) use text::truncate_start;

#[cfg(test)]
mod testutil;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use neo_protocol::{SessionEvent, SessionEventRecord, ToolCallContent};
use neo_renderer_ratatui::Palette;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(crate) use diff::DiffView;
use diff::{Comment, DiffRequest};
use files::{FileKey, FilesState};
use git::Runner;
use plan::PlanState;
use terminal::TerminalState;
pub use terminal::{JOB_LINGER_MS, JobRow};
use todos::{TodoKey, TodosState};
use usage::UsageState;
pub(crate) use usage::format_cost;

/// Lines PgUp/PgDn move in the terminal panel.
const TERMINAL_PAGE: usize = 10;
/// How often background commands' output files are read.
const JOBS_POLL_MS: i64 = 250;

/// The side column is only worth its width from here on.
const SIDEBAR_MIN_WIDTH: u16 = 100;

/// Which side panel has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Todos,
    Plan,
    Files,
    Terminal,
}

/// What a key in a focused panel asks the shell to do.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetAction {
    /// The key was handled inside the panel (selection moved, diff scrolled…): just redraw.
    Redraw,
    /// Give focus back to the editor (Esc).
    Leave,
    /// The editor, to write a new todo in (`a` in the todos panel).
    AddTodo,
    /// Change the shared todo list: `neo.todos` with these args.
    Todo(serde_json::Value),
}

/// All panels of one session.
pub struct Widgets {
    cwd: PathBuf,
    palette: Palette,
    plan: PlanState,
    todos: TodosState,
    usage: UsageState,
    files: FilesState,
    terminal: TerminalState,
    /// When the background commands' files were last read.
    jobs_polled_ms: i64,
    /// `/diff split`: files open side by side where they fit.
    split_diffs: bool,
    diff: Option<DiffView>,
    /// The comments written in the diff view, by file, kept while it is closed and until the
    /// message they are in is sent.
    comments: HashMap<String, Vec<Comment>>,
    /// toolCallId -> the guest that made the call (None: the session's own agent).
    tool_agents: HashMap<String, Option<String>>,
    /// None only in tests that don't want a git process.
    runner: Option<Runner>,
}

impl Widgets {
    /// For a session working in `cwd`. Starts the first git refresh in the background.
    pub fn new(cwd: PathBuf, palette: Palette) -> Self {
        Widgets::build(cwd, palette, true)
    }

    fn build(cwd: PathBuf, palette: Palette, git: bool) -> Self {
        let mut runner = git.then(|| Runner::for_dir(&cwd, Instant::now()));
        if let Some(r) = &mut runner {
            r.poll(Instant::now());
        }
        Widgets {
            cwd,
            palette,
            plan: PlanState::default(),
            todos: TodosState::default(),
            usage: UsageState::default(),
            files: FilesState::default(),
            terminal: TerminalState::default(),
            jobs_polled_ms: 0,
            split_diffs: false,
            diff: None,
            comments: HashMap::new(),
            tool_agents: HashMap::new(),
            runner,
        }
    }

    /// Another session (or directory) is shown now: forget everything and start over.
    pub fn reset(&mut self, cwd: PathBuf) {
        let (icons, split) = (self.files.icons, self.split_diffs);
        // A run still going for the old directory finds its channel closed and ends quietly.
        *self = Widgets::build(cwd, self.palette, self.runner.is_some());
        self.files.icons = icons;
        self.split_diffs = split;
    }

    /// The same session works in another directory now (/cd): the files and git start over
    /// there; the plan, todos, usage and commands stay.
    pub fn move_to(&mut self, cwd: PathBuf) {
        let icons = self.files.icons;
        self.files = FilesState::default();
        self.files.icons = icons;
        self.diff = None;
        // Their paths were relative to the old directory.
        self.comments.clear();
        if self.runner.is_some() {
            let mut runner = Runner::for_dir(&cwd, Instant::now());
            runner.poll(Instant::now());
            self.runner = Some(runner);
        }
        self.cwd = cwd;
    }

    /// File type icons in the files panel.
    pub fn set_icons(&mut self, on: bool) {
        self.files.icons = on;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Every event of the shown session, replayed and live, in seq order: plans, usage, the
    /// files the agent and guests changed (diffs), turn ends (a git refresh).
    pub fn on_session_event(&mut self, record: &SessionEventRecord) {
        match &record.event {
            SessionEvent::Plan(p) => self.plan.apply(p),
            SessionEvent::Todos(t) => self.todos.apply(t),
            SessionEvent::Usage(u) => self.usage.apply(u),
            SessionEvent::ContextCleared(_) => self.usage.clear_context(),
            SessionEvent::ToolCall(call) => {
                self.tool_agents
                    .insert(call.tool_call_id.clone(), call.agent.clone());
                self.terminal.on_call_at(call, record.ts);
            }
            SessionEvent::ToolCallUpdate(update) => {
                self.terminal.on_update(update, record.ts);
                let mut changed = false;
                for content in update.content.iter().flatten() {
                    let ToolCallContent::DiffContent(d) = content else {
                        continue;
                    };
                    if d.path.is_empty() {
                        continue;
                    }
                    let tag = match self.tool_agents.get(&update.tool_call_id) {
                        Some(Some(guest)) => guest.as_str(),
                        _ => "agent",
                    };
                    let rel = relativize(&self.cwd, &d.path);
                    self.files.touch(&rel, tag, d.old_text.is_none());
                    changed = true;
                }
                if changed {
                    self.request_refresh();
                }
            }
            SessionEvent::TurnEnded(_) => self.request_refresh(),
            _ => {}
        }
    }

    /// The daemon's file.changed for any app: schedule a (debounced) git refresh.
    pub fn on_file_changed(&mut self, _path: &str) {
        self.request_refresh();
    }

    fn request_refresh(&mut self) {
        if let Some(r) = &mut self.runner {
            r.request(Instant::now());
        }
    }

    /// Called on every loop iteration (about 20×/s): picks up finished background work and
    /// what background commands wrote. Returns true when something visible changed.
    pub fn tick(&mut self, now_ms: i64) -> bool {
        let mut changed = false;
        if now_ms - self.jobs_polled_ms >= JOBS_POLL_MS {
            self.jobs_polled_ms = now_ms;
            changed |= self.terminal.poll_jobs(now_ms);
        }
        if let Some(runner) = &mut self.runner
            && let Some(status) = runner.poll(Instant::now())
        {
            changed |= self.files.set_status(status);
        }
        if let Some(d) = &mut self.diff {
            changed |= d.poll();
        }
        changed
    }

    /// "main ↑1" for the status line; None outside a git repository.
    pub fn branch(&self) -> Option<String> {
        self.files.branch()
    }

    /// The usage line under the prompt, e.g. "ctx 35% · $0.18";
    /// None before any usage event.
    pub fn usage_line(&self) -> Option<Line<'static>> {
        self.usage.line(&self.palette)
    }

    /// Whether the side column is worth showing at this terminal width (there is a plan or
    /// changed files, and it fits).
    pub fn wants_sidebar(&self, width: u16) -> bool {
        width >= SIDEBAR_MIN_WIDTH && self.has_panels()
    }

    /// Whether the side column has todos, a plan or changed files to show (else at most the
    /// agent's commands).
    pub fn has_panels(&self) -> bool {
        self.plan.has_any() || self.files.worth_showing() || self.todos.has_any()
    }

    /// Columns the side column wants (border and margins included): `max` with a plan, whose
    /// text wraps; else as wide as the widest file row, at least `min`.
    pub fn sidebar_width(&self, min: u16, max: u16) -> u16 {
        if self.plan.has_any() || self.todos.has_any() {
            return max;
        }
        let rows = self.files.natural_width(&self.palette);
        u16::try_from(rows + 3).unwrap_or(max).clamp(min, max)
    }

    /// Where the side column in `area` puts its panels (todos, plan, files) and, with `terminal`,
    /// the agent's commands under them: the panels take what they need, up to half, a blank row
    /// parts them, and the commands get the rest. Both are inside the border and margins.
    fn side_split(&self, area: Rect, terminal: bool) -> (Option<Rect>, Option<Rect>) {
        // A thin border, a space, the content, a space.
        if area.width < 6 || area.height == 0 {
            return (None, None);
        }
        let area = pane_content(area, &self.palette);
        let area = Rect {
            x: area.x + 2,
            width: area.width - 3,
            ..area
        };
        let panels = self.has_panels();
        if !terminal || area.height < 2 {
            return (Some(area), None);
        }
        if !panels || area.height < 6 {
            return (None, Some(area));
        }
        let w = usize::from(area.width);
        // The todos always show whole; the rest of the panels take up to half.
        let need = self.panels_height(w);
        let top = need
            .min(usize::from(area.height / 2).max(self.todos_height(w)))
            .min(usize::from(area.height));
        let top = u16::try_from(top).unwrap_or(u16::MAX);
        // No room left for the commands' title and a row: the panels get it all.
        if area.height - top < 3 {
            return (Some(area), None);
        }
        let below = Rect {
            y: area.y + top + 1,
            height: area.height - top - 1,
            ..area
        };
        (
            Some(Rect {
                height: top,
                ..area
            }),
            Some(below),
        )
    }

    /// Rows the todos would fill `w` columns wide, their title included; none without any.
    fn todos_height(&self, w: usize) -> usize {
        if self.todos.has_any() {
            1 + self.todos.natural_height(w, &self.palette)
        } else {
            0
        }
    }

    /// Rows the todos, plan and files would fill `w` columns wide, their titles and the blank
    /// rows between them included.
    fn panels_height(&self, w: usize) -> usize {
        let p = &self.palette;
        let mut parts = Vec::new();
        if self.todos.has_any() {
            parts.push(self.todos_height(w));
        }
        if self.plan.has_any() {
            parts.push(1 + self.plan.natural_height(w, p));
        }
        if self.files.worth_showing() {
            parts.push(1 + self.files.natural_height());
        }
        parts.iter().sum::<usize>() + parts.len().saturating_sub(1)
    }

    /// Where [`Widgets::draw_sidebar`] puts the agent's commands in `area`, if anywhere.
    pub fn side_terminal(&self, area: Rect, terminal: bool) -> Option<Rect> {
        self.side_split(area, terminal).1
    }

    /// Draws the side column into `area`: the todos, plan and files, and with `terminal` the
    /// agent's commands under them; `focus` highlights a panel.
    pub fn draw_sidebar(
        &self,
        frame: &mut Frame,
        area: Rect,
        focus: Option<Panel>,
        terminal: bool,
    ) {
        let (panels, commands) = self.side_split(area, terminal);
        if panels.is_none() && commands.is_none() {
            return;
        }
        pane(frame, area, &self.palette);
        if let Some(at) = panels {
            self.draw_panels(frame, at, focus);
            if commands.is_some() {
                rule(frame, at.x, at.y + at.height, at.width, &self.palette);
            }
        }
        if let Some(at) = commands {
            self.draw_terminal(frame, at, focus == Some(Panel::Terminal));
        }
    }

    /// The todos on top, then the plan above the files, in `area` (inside the margins).
    fn draw_panels(&self, frame: &mut Frame, area: Rect, focus: Option<Panel>) {
        if area.height == 0 {
            return;
        }
        let p = &self.palette;
        let x = area.x;
        let w = area.width;
        let has_plan = self.plan.has_any();
        let has_files = self.files.worth_showing();
        // The shared todos on top while there are any, whole; the plan and the files below.
        let todos_h = self.todos_height(w as usize).min(area.height as usize);
        let todos_at = |y: u16, h: usize| Rect {
            x,
            y: area.y + y,
            width: w,
            height: h as u16,
        };
        if todos_h > 0 {
            let title = title_line("Todos", focus == Some(Panel::Todos), p);
            frame.render_widget(Paragraph::new(title), todos_at(0, 1));
        }
        if todos_h > 1 {
            let focused = focus == Some(Panel::Todos);
            let lines = self.todos.render(w as usize, todos_h - 1, focused, p);
            frame.render_widget(Paragraph::new(lines), todos_at(1, todos_h - 1));
        }
        let parted = todos_h > 0 && (has_plan || has_files) && todos_h < area.height as usize;
        if parted {
            rule(frame, x, area.y + todos_h as u16, w, p);
        }
        let below = todos_h + usize::from(parted);
        let area = Rect {
            y: area.y + below as u16,
            height: area.height.saturating_sub(below as u16),
            ..area
        };
        let height = area.height as usize;
        let plan_need = 1 + self.plan.natural_height(w as usize, p);
        let files_need = 1 + self.files.natural_height();
        let (plan_h, files_h) = match (has_plan, has_files) {
            (true, true) => {
                // The plan takes what it needs, up to half; the files get the rest.
                let avail = height.saturating_sub(1);
                let plan_h = plan_need.min((avail / 2).max(avail.saturating_sub(files_need)));
                (plan_h, avail - plan_h)
            }
            (true, false) => (height.min(plan_need), 0),
            (false, true) => (0, height),
            (false, false) => (0, 0),
        };
        let at = |y: usize, h: usize| Rect {
            x,
            y: area.y + y as u16,
            width: w,
            height: h as u16,
        };
        if plan_h > 0 {
            let title = title_line("Plan", focus == Some(Panel::Plan), p);
            frame.render_widget(Paragraph::new(title), at(0, 1));
            if plan_h > 1 {
                let lines = self.plan.render(w as usize, plan_h - 1, p);
                frame.render_widget(Paragraph::new(lines), at(1, plan_h - 1));
            }
        }
        if files_h > 0 {
            if plan_h > 0 {
                rule(frame, x, area.y + plan_h as u16, w, p);
            }
            let top = if plan_h > 0 { plan_h + 1 } else { 0 };
            let focused = focus == Some(Panel::Files);
            let title = title_line("Files", focused, p);
            frame.render_widget(Paragraph::new(title), at(top, 1));
            if files_h > 1 {
                let lines = self.files.render(w as usize, files_h - 1, focused, p);
                frame.render_widget(Paragraph::new(lines), at(top + 1, files_h - 1));
            }
        }
    }

    /// The newest usage of the session's own agent (context, tokens, cost).
    pub fn usage(&self) -> Option<&neo_protocol::Usage> {
        self.usage.last()
    }

    /// Whether the shared todo list has items (its panel is drawn).
    pub fn has_todos(&self) -> bool {
        self.todos.has_any()
    }

    /// The commands the session's agent's turn waits for: call ids and when they started.
    pub fn foreground_commands(&self) -> Vec<(String, i64)> {
        self.terminal.foreground()
    }

    /// The background commands to list under the prompt.
    pub fn jobs(&self, now_ms: i64) -> Vec<JobRow> {
        self.terminal.jobs(now_ms)
    }

    /// Stops a running background command (see [`TerminalState::stop_job`]).
    pub fn stop_job(&self, id: &str) -> Result<usize, String> {
        self.terminal.stop_job(id)
    }

    /// Shows one background command in the terminal panel on its own, or (None) all commands.
    pub fn show_job(&mut self, id: Option<String>) {
        self.terminal.set_view(id);
    }

    /// Whether there is a command to show in the terminal panel.
    pub fn has_commands(&self) -> bool {
        self.terminal.has_any()
    }

    /// Draws the terminal panel into `inner` (inside the margins): a title, the commands.
    fn draw_terminal(&self, frame: &mut Frame, inner: Rect, focused: bool) {
        if inner.width == 0 || inner.height < 2 {
            return;
        }
        let p = &self.palette;
        let mut title = title_line("Terminal", focused, p);
        if self.terminal.view().is_some() {
            title.spans.push(Span::styled(
                " · background command · esc all commands",
                Style::new().fg(p.muted),
            ));
        }
        if !self.terminal.following() {
            title.spans.push(Span::styled(
                "  scrolled · G follows",
                Style::new().fg(p.muted),
            ));
        }
        frame.render_widget(Paragraph::new(title), Rect { height: 1, ..inner });
        let body = Rect {
            y: inner.y + 1,
            height: inner.height - 1,
            ..inner
        };
        let lines = self.terminal.render(
            usize::from(body.width),
            usize::from(body.height),
            self.cwd.to_str(),
            p,
        );
        frame.render_widget(Paragraph::new(lines), body);
    }

    /// A diff the user opened from the files panel, drawn over `area` (the chat's area), if
    /// one is open.
    pub fn draw_overlay(&self, frame: &mut Frame, area: Rect) -> bool {
        match &self.diff {
            Some(d) => {
                d.draw(frame, area, &self.palette);
                true
            }
            None => false,
        }
    }

    /// Whether a diff overlay is open (it takes the keys then).
    pub fn overlay_open(&self) -> bool {
        self.diff.is_some()
    }

    /// The file `<leader>e` in the diff overlay asked to open in the editor, and the line.
    pub fn take_edit(&mut self) -> Option<(PathBuf, Option<usize>)> {
        self.diff.as_mut()?.take_edit()
    }

    /// What the comments written in the diff overlay changed: each as the prompt had it
    /// (`@path:12-14 text`) and has it now; none: not there.
    pub fn take_comment_changes(&mut self) -> Vec<(Option<String>, Option<String>)> {
        self.diff
            .as_mut()
            .map(DiffView::take_comment_changes)
            .unwrap_or_default()
    }

    /// A message went: the comments it has in it are done.
    pub fn comments_sent(&mut self, text: &str) {
        for (rel, list) in &mut self.comments {
            list.retain(|c| !text.contains(&c.mention(rel)));
        }
        self.comments.retain(|_, list| !list.is_empty());
    }

    /// The diff view, with the comments written on its file before.
    fn with_comments(&self, view: DiffView, rel: &str) -> DiffView {
        view.with_comments(self.comments.get(rel).cloned().unwrap_or_default())
    }

    /// Closes the diff view, keeping its comments.
    fn close_diff(&mut self) {
        if let Some(mut d) = self.diff.take()
            && let Some(rel) = d.rel().map(str::to_string)
        {
            let comments = d.take_comments();
            if comments.is_empty() {
                self.comments.remove(&rel);
            } else {
                self.comments.insert(rel, comments);
            }
        }
    }

    /// A key while a panel (or the diff overlay) has focus.
    pub fn handle_key(&mut self, panel: Panel, key: KeyEvent) -> WidgetAction {
        if key.kind == KeyEventKind::Release {
            return WidgetAction::Redraw;
        }
        if let Some(d) = &mut self.diff {
            if d.key(key) {
                self.close_diff();
            }
            return WidgetAction::Redraw;
        }
        match panel {
            Panel::Todos => match self.todos.key(key) {
                TodoKey::Handled => {}
                TodoKey::Leave => return WidgetAction::Leave,
                TodoKey::Add => return WidgetAction::AddTodo,
                TodoKey::Change(args) => return WidgetAction::Todo(args),
            },
            Panel::Plan => {
                if self.plan.key(key) {
                    return WidgetAction::Leave;
                }
            }
            Panel::Files => match self.files.key(key) {
                FileKey::Leave => return WidgetAction::Leave,
                FileKey::Open => self.open_diff(),
                FileKey::Handled => {}
            },
            // Leaving it shows all commands again.
            Panel::Terminal => {
                if self.terminal.key(key, TERMINAL_PAGE) {
                    self.terminal.set_view(None);
                    return WidgetAction::Leave;
                }
            }
        }
        WidgetAction::Redraw
    }

    fn open_diff(&mut self) {
        if let Some(e) = self.files.selected() {
            let req = DiffRequest {
                rel: e.rel.clone(),
                orig: e.orig.clone(),
                mark: e.mark,
                in_repo: self.files.in_repo(),
            };
            let rel = req.rel.clone();
            let view = DiffView::open_file(self.cwd.clone(), req).with_split(self.split_diffs);
            self.diff = Some(self.with_comments(view, &rel));
        }
    }

    /// `/diff`: whether files open side by side (where they fit) or unified.
    pub fn set_split_diffs(&mut self, split: bool) {
        self.split_diffs = split;
    }

    /// Opens the diff overlay on `path` (as a tool's diff names it), as the files panel would;
    /// a file the panel doesn't list counts as modified.
    pub fn open_path(&mut self, path: &str) {
        let rel = relativize(&self.cwd, path);
        let entry = self.files.entries().iter().find(|e| e.rel == rel);
        let req = DiffRequest {
            orig: entry.and_then(|e| e.orig.clone()),
            mark: entry.map_or(files::Mark::Modified, |e| e.mark),
            in_repo: self.files.in_repo(),
            rel: rel.clone(),
        };
        let view = DiffView::open_file(self.cwd.clone(), req).with_split(self.split_diffs);
        self.diff = Some(self.with_comments(view, &rel));
    }
}

/// A pane at the right: its first column a gap (a border without fills), the rest filled.
fn pane(frame: &mut Frame, area: Rect, palette: &Palette) {
    let buf = frame.buffer_mut();
    crate::style::pane_edge(buf, area.x, area, palette);
    let filled = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    crate::style::fill(buf, filled, palette.panel);
}

/// A thin rule parting two sections of the side column, on row `y`.
fn rule(frame: &mut Frame, x: u16, y: u16, w: u16, palette: &Palette) {
    let line = Line::from(Span::styled(
        "─".repeat(usize::from(w)),
        Style::new().fg(palette.border),
    ));
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, w, 1));
}

/// Where a pane's content goes: a row down from the top when filled.
fn pane_content(area: Rect, palette: &Palette) -> Rect {
    let top = u16::from(palette.panel != ratatui::style::Color::Reset && area.height > 4);
    Rect {
        y: area.y + top,
        height: area.height - top,
        ..area
    }
}

/// A section's heading: bold where panes are filled, muted on a border's side; the accent when
/// the section has the keys.
fn title_line(title: &'static str, focused: bool, palette: &Palette) -> Line<'static> {
    let style = if focused {
        Style::new().fg(palette.accent).add_modifier(Modifier::BOLD)
    } else if palette.panel != ratatui::style::Color::Reset {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(palette.muted)
    };
    Line::from(Span::styled(title, style))
}

/// A path from a tool's diff, relative to `cwd` when it is inside it.
fn relativize(cwd: &Path, path: &str) -> String {
    let p = Path::new(path);
    let rel = if p.is_absolute() {
        p.strip_prefix(cwd)
            .map_or_else(|_| path.to_string(), |r| r.to_string_lossy().into_owned())
    } else {
        path.strip_prefix("./").unwrap_or(path).to_string()
    };
    if rel.is_empty() { ".".to_string() } else { rel }
}

#[cfg(test)]
mod tests;
