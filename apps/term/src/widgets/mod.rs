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
//! - **Syntax highlighting** (`syntax.rs`): the diff and file view color code by syntect's
//!   grammar for the file name (`+++ b/` headers for a pull request's diff), worked out on a
//!   thread once the text is in (the code shows plain until then), old and new lines parsed
//!   apart and each hunk starting over. The colors are picked for a dark or light background,
//!   not the theme's few; changed lines keep their tint behind them and the sign its color.
//!   Without true colors, and past 20,000 lines, code stays plain.
//! - **Diff overlay**: `git diff HEAD -- path` (the whole file for untracked ones) loaded in the
//!   background and drawn over the chat, side by side from 90 columns on. While it is open it
//!   takes every key whatever panel is focused: Up/Down/j/k, PgUp/PgDn, g/G, Ctrl-U/Ctrl-D
//!   scroll; n/p jump to the next / previous hunk; Left/Right/h/l scroll sideways by 8 cells and
//!   0 returns to the left edge; s switches split and unified; f reloads with the whole file as
//!   context (and back; it no longer pages); Esc or q close it. Closing
//!   returns `Redraw`, so focus stays on the files panel (to pick the next file); a second Esc
//!   there returns `Leave`.
//! - The GitHub view (`crate::github`) reuses the diff view for a pull request's diff.
//! - Keys the panels don't use return `Redraw` (harmless). Esc and q in a panel return `Leave`.

mod diff;
mod files;
mod git;
mod plan;
mod syntax;
mod text;
mod usage;

#[cfg(test)]
mod testutil;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use suite_protocol::{SessionEvent, SessionEventRecord, ToolCallContent};
use suite_renderer_ratatui::Palette;

use diff::DiffRequest;
pub(crate) use diff::DiffView;
use files::{FileKey, FilesState};
use git::Runner;
use plan::PlanState;
use usage::UsageState;

/// The side column is only worth its width from here on.
const SIDEBAR_MIN_WIDTH: u16 = 100;

/// Which side panel has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Plan,
    Files,
}

/// What a key in a focused panel asks the shell to do.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetAction {
    /// The key was handled inside the panel (selection moved, diff scrolled…): just redraw.
    Redraw,
    /// Give focus back to the editor (Esc).
    Leave,
}

/// All panels of one session.
pub struct Widgets {
    cwd: PathBuf,
    palette: Palette,
    plan: PlanState,
    usage: UsageState,
    files: FilesState,
    diff: Option<DiffView>,
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
            usage: UsageState::default(),
            files: FilesState::default(),
            diff: None,
            tool_agents: HashMap::new(),
            runner,
        }
    }

    /// Another session (or directory) is shown now: forget everything and start over.
    pub fn reset(&mut self, cwd: PathBuf) {
        // A run still going for the old directory finds its channel closed and ends quietly.
        *self = Widgets::build(cwd, self.palette, self.runner.is_some());
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Every event of the shown session, replayed and live, in seq order: plans, usage, the
    /// files the agent and guests changed (diffs), turn ends (a git refresh).
    pub fn on_session_event(&mut self, record: &SessionEventRecord) {
        match &record.event {
            SessionEvent::Plan(p) => self.plan.apply(p),
            SessionEvent::Usage(u) => self.usage.apply(u),
            SessionEvent::ToolCall(call) => {
                self.tool_agents
                    .insert(call.tool_call_id.clone(), call.agent.clone());
            }
            SessionEvent::ToolCallUpdate(update) => {
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

    /// Called on every loop iteration (about 20×/s): picks up finished background work.
    /// Returns true when something visible changed.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
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
        width >= SIDEBAR_MIN_WIDTH && (self.plan.has_any() || self.files.has_any())
    }

    /// Draws the side column (plan above files) into `area`; `focus` highlights a panel.
    pub fn draw_sidebar(&self, frame: &mut Frame, area: Rect, focus: Option<Panel>) {
        // A thin border, a space, the content, a space.
        if area.width < 6 || area.height == 0 {
            return;
        }
        let p = &self.palette;
        let border = Style::new().fg(p.border);
        for y in area.y..area.bottom() {
            frame.buffer_mut()[(area.x, y)]
                .set_symbol("│")
                .set_style(border);
        }
        let x = area.x + 2;
        let w = area.width - 3;
        let height = area.height as usize;
        let has_plan = self.plan.has_any();
        let has_files = self.files.has_any();
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

    /// A key while a panel (or the diff overlay) has focus.
    pub fn handle_key(&mut self, panel: Panel, key: KeyEvent) -> WidgetAction {
        if key.kind == KeyEventKind::Release {
            return WidgetAction::Redraw;
        }
        if let Some(d) = &mut self.diff {
            if d.key(key) {
                self.diff = None;
            }
            return WidgetAction::Redraw;
        }
        match panel {
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
            self.diff = Some(DiffView::open_file(self.cwd.clone(), req));
        }
    }
}

fn title_line(title: &'static str, focused: bool, palette: &Palette) -> Line<'static> {
    let style = if focused {
        Style::new().fg(palette.accent).add_modifier(Modifier::BOLD)
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
