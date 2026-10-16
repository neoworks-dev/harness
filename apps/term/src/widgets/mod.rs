//! The side panels and the usage line under the prompt: plan (todo lists of the agent and its
//! guests), files (git status plus what the agent touched, with diffs) and usage (context and
//! cost). Each is state fed from session events plus a draw function; nothing here blocks the
//! UI thread (git runs on a background thread).
//!
//! This is the contract the shell (`app`) uses. Implementations are placeholders until filled in.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::text::Line;
use suite_protocol::SessionEventRecord;
use suite_renderer_ratatui::Palette;

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
}

impl Widgets {
    /// For a session working in `cwd`. Starts the first git refresh in the background.
    pub fn new(cwd: PathBuf, palette: Palette) -> Self {
        Widgets { cwd, palette }
    }

    /// Another session (or directory) is shown now: forget everything and start over.
    pub fn reset(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Every event of the shown session, replayed and live, in seq order: plans, usage, the
    /// files the agent and guests changed (diffs), turn ends (a git refresh).
    pub fn on_session_event(&mut self, _record: &SessionEventRecord) {}

    /// The daemon's file.changed for any app: schedule a (debounced) git refresh.
    pub fn on_file_changed(&mut self, _path: &str) {}

    /// Called on every loop iteration (about 20×/s): picks up finished background work.
    /// Returns true when something visible changed.
    pub fn tick(&mut self) -> bool {
        false
    }

    /// "main ↑1" for the status line; None outside a git repository.
    pub fn branch(&self) -> Option<String> {
        None
    }

    /// The usage line under the prompt, e.g. "ctx 35% · $0.18 · claude ctx 12% $0.42";
    /// None before any usage event.
    pub fn usage_line(&self) -> Option<Line<'static>> {
        None
    }

    /// Whether the side column is worth showing at this terminal width (there is a plan or
    /// changed files, and it fits).
    pub fn wants_sidebar(&self, _width: u16) -> bool {
        false
    }

    /// Draws the side column (plan above files) into `area`; `focus` highlights a panel.
    pub fn draw_sidebar(&self, _frame: &mut Frame, _area: Rect, _focus: Option<Panel>) {}

    /// A diff the user opened from the files panel, drawn over `area` (the chat's area), if
    /// one is open.
    pub fn draw_overlay(&self, _frame: &mut Frame, _area: Rect) -> bool {
        false
    }

    /// Whether a diff overlay is open (it takes the keys then).
    pub fn overlay_open(&self) -> bool {
        false
    }

    /// A key while a panel (or the diff overlay) has focus.
    pub fn handle_key(&mut self, _panel: Panel, _key: KeyEvent) -> WidgetAction {
        WidgetAction::Leave
    }
}
