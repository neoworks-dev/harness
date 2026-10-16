//! The app state: chat, editor, overlays and keys, with no terminal or network in it.
//!
//! [`App::handle_key`], [`App::handle_event`] and [`App::handle_msg`] change the state and return
//! [`Effect`]s for the runtime to carry out against the daemon; [`App::draw`] paints it.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use suite_protocol::{
    AgentFocus, SessionEvent, SessionEventRecord, SessionInfo, SessionState, SettingName,
};
use suite_renderer_ratatui::{ButtonRef, Palette};
use suite_sdk::Event;

use crate::chat::{Chat, ChatOpts};
use crate::editor::Editor;
use crate::history::{Histories, HistoryStore};
use crate::menu::{self, Command, Entry, Switcher};
use crate::sessions::{Look, SIDEBAR_WIDTH, Sessions, SessionsAction};
use crate::status::{Approvals, HarnessSettings, Status, record_setting};
use crate::style::{Styles, plain_palette};
use crate::util::{fit_line, shorten_home, truncate, width};
use crate::widgets::{Panel, WidgetAction, Widgets};

/// How long a second Ctrl-C quits.
const QUIT_WINDOW_MS: i64 = 1500;
/// How long a transient message stays.
const NOTICE_MS: i64 = 4000;
/// How often a running turn's session info is refreshed (queue length, stuck turns).
const POLL_MS: i64 = 2000;
/// Rows of the slash menu.
const MENU_ROWS: usize = 8;
/// Chat lines one wheel notch scrolls.
const WHEEL_LINES: usize = 3;
/// Most rows the editor grows to.
const EDITOR_ROWS: usize = 8;
const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
/// The chat needs at least this much beside the sessions sidebar, else the sidebar is hidden.
const CHAT_MIN_WIDTH: u16 = 52;
/// The harness a session runs on until a `/harness` setting says otherwise.
const DEFAULT_HARNESS: &str = "pi";

/// What the runtime does on the app's behalf. Requests to the daemon block, so the runtime runs
/// them off the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send input as this app (the daemon follows it: the agent moves here).
    Input {
        session: String,
        text: String,
    },
    /// A button was pressed.
    Action {
        session: String,
        item: String,
        action: String,
        args: Option<serde_json::Value>,
    },
    Cancel {
        session: String,
    },
    /// Follow another session: stop following `from`, follow `to` (view and events) and, with
    /// `move_agent`, make it current and bring the agent to this app.
    Switch {
        from: Option<String>,
        to: String,
        cwd: String,
        move_agent: bool,
    },
    ListSessions,
    CreateSession {
        cwd: String,
    },
    /// Ask for the session's info again (queue length, whether it still runs).
    Refresh,
    /// Put text on the clipboard (OSC 52).
    Copy(String),
    Quit,
}

/// Answers from the runtime's worker thread.
#[derive(Debug)]
pub enum Msg {
    Sessions(Result<Vec<SessionInfo>, String>),
    Created(Result<SessionInfo, String>),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Help,
    Switcher,
}

/// What has the keyboard besides the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Sessions,
    Panel(Panel),
}

#[derive(Debug, Clone)]
struct Notice {
    text: String,
    error: bool,
    until: i64,
}

/// Which prompt (an item with buttons) the keys answer.
#[derive(Debug, Default)]
struct PromptState {
    /// Picked by hand (Ctrl-P/N) instead of the newest pending one.
    selected: Option<String>,
    /// The highlight was dismissed (Esc) or the prompt answered.
    dismissed: Option<String>,
    button: usize,
    /// The prompt last seen as active, to notice a new one.
    seen: Option<String>,
}

/// What [`App::new`] needs.
pub struct Config {
    /// This client's app id in the daemon (`terminal`, `terminal-2`…).
    pub app_id: String,
    pub session: String,
    pub cwd: String,
    pub home: Option<String>,
    pub no_color: bool,
    pub history: HistoryStore,
}

pub struct App {
    app_id: String,
    session: String,
    cwd: String,
    home: Option<String>,
    no_color: bool,
    palette: Palette,
    styles: Styles,

    widgets: Widgets,
    chat: Chat,
    editor: Editor,
    histories: Histories,
    overlay: Overlay,
    switcher: Switcher,
    focus: Option<Focus>,
    sessions: Sessions,
    /// The sessions sidebar is switched on (it still needs a wide enough terminal).
    sessions_on: bool,
    sessions_shown: bool,
    /// Ctrl-G while the sidebar was hidden: focus it once it is drawn.
    sessions_wanted: bool,
    prompt: PromptState,
    menu_sel: usize,
    /// The editor text the slash menu was closed for.
    menu_closed_for: Option<String>,

    expanded: bool,
    show_thoughts: bool,
    running: bool,
    turn_started_ms: i64,
    queued: usize,
    settings: Vec<HarnessSettings>,
    /// The session's agent, from `/harness`.
    harness: String,
    approvals: Approvals,
    connected: bool,
    /// Where the agent is (the last `agent.focus`).
    agent_app: Option<String>,
    /// After moving the agent here, focus events of other sessions are stale until this one
    /// shows up (or the time is up).
    awaiting: Option<(String, i64)>,

    notice: Option<Notice>,
    quit_armed_until: i64,
    now_ms: i64,
    last_poll_ms: i64,
    last_spin: i64,
    size: (u16, u16),
    sidebar_shown: bool,
    dirty: bool,
    clear: bool,
}

impl App {
    pub fn new(config: Config) -> App {
        let palette = if config.no_color {
            plain_palette()
        } else {
            Palette::default()
        };
        let now = crate::util::now_ms();
        let session = config.session.clone();
        App {
            widgets: Widgets::new(PathBuf::from(&config.cwd), palette),
            styles: Styles::new(&palette, config.no_color),
            palette,
            app_id: config.app_id,
            session: config.session,
            cwd: config.cwd,
            home: config.home,
            no_color: config.no_color,
            chat: Chat::new(),
            editor: Editor::default(),
            histories: Histories::new(config.history),
            overlay: Overlay::None,
            switcher: Switcher::new(&session),
            focus: None,
            sessions: Sessions::default(),
            sessions_on: true,
            sessions_shown: false,
            sessions_wanted: false,
            prompt: PromptState::default(),
            menu_sel: 0,
            menu_closed_for: None,
            expanded: false,
            show_thoughts: false,
            running: false,
            turn_started_ms: now,
            queued: 0,
            settings: Vec::new(),
            harness: DEFAULT_HARNESS.to_string(),
            approvals: Approvals::Ask,
            connected: true,
            agent_app: None,
            awaiting: Some((session, now + 3000)),
            notice: None,
            quit_armed_until: 0,
            now_ms: now,
            last_poll_ms: now,
            last_spin: 0,
            size: (80, 24),
            sidebar_shown: false,
            dirty: true,
            clear: false,
        }
    }

    // ---- accessors ----

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn approvals(&self) -> Approvals {
        self.approvals
    }

    pub fn harness(&self) -> &str {
        &self.harness
    }

    pub fn sessions(&self) -> &Sessions {
        &self.sessions
    }

    /// The sessions known at start (`list_sessions`).
    pub fn set_sessions(&mut self, list: Vec<SessionInfo>) {
        self.sessions.set_all(list);
        self.dirty = true;
    }

    pub fn editor_text(&self) -> &str {
        self.editor.text()
    }

    pub fn chat(&self) -> &Chat {
        &self.chat
    }

    pub fn widgets(&self) -> &Widgets {
        &self.widgets
    }

    /// Whether anything changed since the last call (so the screen needs painting).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Ctrl-L: the terminal should be cleared and repainted from scratch.
    pub fn take_clear(&mut self) -> bool {
        std::mem::take(&mut self.clear)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.size = (width, height);
        self.dirty = true;
    }

    // ---- clock ----

    /// Called every loop iteration with the wall clock (unix ms).
    pub fn tick(&mut self, now_ms: i64) -> Vec<Effect> {
        self.now_ms = now_ms;
        let mut fx = Vec::new();
        if self.widgets.tick() {
            self.dirty = true;
        }
        if self.running || self.sessions_shown && self.sessions.any_running() {
            let frame = now_ms / 125;
            if frame != self.last_spin {
                self.last_spin = frame;
                self.dirty = true;
            }
            if now_ms - self.last_poll_ms >= POLL_MS {
                self.last_poll_ms = now_ms;
                fx.push(Effect::Refresh);
            }
        }
        if self.notice.as_ref().is_some_and(|n| n.until <= now_ms) {
            self.notice = None;
            self.dirty = true;
        }
        if self
            .awaiting
            .as_ref()
            .is_some_and(|(_, until)| *until <= now_ms)
        {
            self.awaiting = None;
        }
        fx
    }

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
            until: self.now_ms + NOTICE_MS,
        });
    }

    // ---- daemon events ----

    pub fn handle_event(&mut self, event: Event) -> Vec<Effect> {
        let mut fx = Vec::new();
        match event {
            Event::Focus(focus) => self.on_focus(focus, &mut fx),
            Event::Connectors(_) => {}
            Event::SessionChanged(info) => {
                if info.id == self.session {
                    self.sync_info(&info);
                }
                self.sessions.upsert(info);
            }
            Event::Session(record) => {
                if record.session == self.session {
                    self.on_record(&record);
                }
            }
            Event::Subscribed { session, .. } => {
                if session.id == self.session {
                    self.queued = usize::try_from(session.queued).unwrap_or(0);
                    if session.state == SessionState::Running && !self.running {
                        self.running = true;
                        self.turn_started_ms = self.now_ms;
                    } else if session.state == SessionState::Idle {
                        self.running = false;
                    }
                }
            }
            Event::Connection(up) => self.connected = up,
            Event::Theme(theme) => {
                if !self.no_color {
                    self.palette = Palette::from_theme(&theme);
                    self.styles = Styles::new(&self.palette, false);
                    self.widgets.set_palette(self.palette);
                }
            }
            Event::ViewSnapshot(snapshot) => {
                if snapshot.session == self.session {
                    self.chat.apply_snapshot(snapshot);
                    self.sync_prompt();
                }
            }
            Event::ViewPatch(patch) => {
                if patch.session == self.session {
                    self.chat.apply_patch(patch);
                    self.sync_prompt();
                }
            }
            Event::FileChanged(changed) => self.widgets.on_file_changed(&changed.path),
        }
        self.dirty = true;
        fx
    }

    fn on_focus(&mut self, focus: AgentFocus, fx: &mut Vec<Effect>) {
        self.agent_app = focus.app.clone();
        let here = focus.app.as_deref() == Some(self.app_id.as_str());
        if let Some((want, _)) = &self.awaiting {
            if *want == focus.session {
                self.awaiting = None;
            } else if here {
                // Left over from before we moved the agent here.
                return;
            }
        }
        if here && focus.session != self.session {
            // Someone made another session current while the agent is here (/clear, `suite
            // move`): follow it. A new conversation starts in the same directory.
            let cwd = self.cwd.clone();
            let from = self.session.clone();
            self.begin_session(&focus.session, &cwd);
            self.awaiting = None;
            fx.push(Effect::Switch {
                from: Some(from),
                to: focus.session,
                cwd,
                move_agent: false,
            });
        }
    }

    fn on_record(&mut self, record: &SessionEventRecord) {
        self.widgets.on_session_event(record);
        match &record.event {
            SessionEvent::UserMessage(_) => {
                self.running = true;
                self.turn_started_ms = record.ts;
                self.queued = self.queued.saturating_sub(1);
            }
            SessionEvent::TurnEnded(_) => self.running = false,
            SessionEvent::SettingChanged(change) => {
                let option = match change.option {
                    SettingName::Model => "model",
                    SettingName::Effort => "effort",
                    SettingName::Mode => "mode",
                    SettingName::Approvals => "approvals",
                    SettingName::Harness => "harness",
                };
                self.on_setting(&change.harness, option, &change.value);
            }
            _ => {}
        }
    }

    /// The suite's own settings (approvals, harness) and each harness's (model, effort, mode).
    fn on_setting(&mut self, harness: &str, option: &str, value: &str) {
        match (harness, option) {
            ("suite", "approvals") => {
                if let Some(mode) = Approvals::parse(value) {
                    self.approvals = mode;
                }
            }
            ("suite", "harness") => self.harness = value.to_string(),
            ("suite", _) | (_, "approvals" | "harness") => {}
            _ => record_setting(&mut self.settings, harness, option, value),
        }
    }

    /// What the daemon says about the shown session: how many inputs wait, and whether a turn
    /// still runs (a turn that just started is trusted for a few seconds).
    fn sync_info(&mut self, info: &SessionInfo) {
        self.queued = usize::try_from(info.queued).unwrap_or(0);
        if self.running
            && info.state == SessionState::Idle
            && self.now_ms - self.turn_started_ms > 3000
        {
            self.running = false;
        }
    }

    pub fn handle_msg(&mut self, msg: Msg) -> Vec<Effect> {
        self.dirty = true;
        let mut fx = Vec::new();
        match msg {
            Msg::Sessions(Ok(list)) => {
                if let Some(me) = list.iter().find(|s| s.id == self.session) {
                    self.sync_info(me);
                }
                self.sessions.set_all(list.clone());
                if self.overlay == Overlay::Switcher {
                    self.switcher.set_sessions(list);
                }
            }
            Msg::Sessions(Err(why)) => {
                if self.overlay == Overlay::Switcher {
                    self.switcher.error = Some(why);
                }
            }
            Msg::Created(Ok(info)) => {
                self.sessions.upsert(info.clone());
                let from = self.session.clone();
                self.begin_session(&info.id, &info.cwd);
                self.awaiting = Some((info.id.clone(), self.now_ms + 3000));
                fx.push(Effect::Switch {
                    from: Some(from),
                    to: info.id,
                    cwd: info.cwd,
                    move_agent: true,
                });
            }
            Msg::Created(Err(why)) => self.notify(format!("new session failed: {why}"), true),
            Msg::Error(why) => self.notify(why, true),
        }
        fx
    }

    /// Shows another session: everything of the old one is dropped.
    fn begin_session(&mut self, id: &str, cwd: &str) {
        self.session = id.to_string();
        self.cwd = cwd.to_string();
        self.chat = Chat::new();
        self.running = false;
        self.turn_started_ms = self.now_ms;
        self.queued = 0;
        self.settings.clear();
        self.harness = DEFAULT_HARNESS.to_string();
        self.approvals = Approvals::Ask;
        self.editor.clear();
        self.prompt = PromptState::default();
        self.overlay = Overlay::None;
        self.focus = None;
        self.menu_sel = 0;
        self.menu_closed_for = None;
        self.widgets.reset(PathBuf::from(cwd));
        self.dirty = true;
    }

    // ---- prompts (items with buttons) ----

    /// The prompt the keys answer: one picked by hand, else the newest one still pending.
    fn active_ref(&self) -> Option<&str> {
        if let Some(r) = &self.prompt.selected
            && !self.chat.buttons_of(r).is_empty()
        {
            return Some(r);
        }
        let r = self.chat.pending_prompt()?;
        (self.prompt.dismissed.as_deref() != Some(r)).then_some(r)
    }

    fn active_buttons(&self) -> &[ButtonRef] {
        self.active_ref().map_or(&[], |r| self.chat.buttons_of(r))
    }

    /// A new active prompt starts on its first button and is scrolled into view.
    fn sync_prompt(&mut self) {
        let current = self.active_ref().map(str::to_string);
        if current != self.prompt.seen {
            self.prompt.button = 0;
            if current.is_some() && self.prompt.selected.is_none() {
                self.chat.follow();
            }
            self.prompt.seen = current;
        }
    }

    fn move_button(&mut self, delta: isize) {
        let n = self.active_buttons().len();
        if n > 0 {
            self.prompt.button =
                (self.prompt.button as isize + delta).rem_euclid(n as isize) as usize;
        }
    }

    fn press(&mut self, index: usize, fx: &mut Vec<Effect>) {
        let Some(r) = self.active_ref().map(str::to_string) else {
            return;
        };
        let Some(button) = self.chat.buttons_of(&r).get(index).cloned() else {
            return;
        };
        if let Some(text) = button.copy_text() {
            fx.push(Effect::Copy(text.to_string()));
            self.notify("copied", false);
        } else {
            fx.push(Effect::Action {
                session: self.session.clone(),
                item: r.clone(),
                action: button.action,
                args: button.args,
            });
        }
        self.prompt.dismissed = Some(r);
        self.prompt.selected = None;
        self.sync_prompt();
    }

    /// Ctrl-P / Ctrl-N: pick an older (or newer) item with buttons.
    fn select_prompt(&mut self, delta: isize) -> bool {
        let refs: Vec<String> = self
            .chat
            .prompt_refs()
            .into_iter()
            .map(str::to_string)
            .collect();
        if refs.is_empty() {
            return false;
        }
        let current = self
            .active_ref()
            .and_then(|a| refs.iter().position(|r| r == a));
        let target = match (current, delta) {
            (Some(i), d) if d < 0 => Some(i.saturating_sub(1)),
            (Some(i), _) if i + 1 < refs.len() => Some(i + 1),
            (Some(_), _) => None,
            (None, d) if d < 0 => Some(refs.len() - 1),
            (None, _) => return true,
        };
        match target {
            Some(i) => {
                let r = refs[i].clone();
                self.prompt.selected = Some(r.clone());
                self.prompt.dismissed = None;
                self.chat.reveal(&r);
            }
            None => {
                self.prompt.selected = None;
                self.prompt.dismissed = None;
            }
        }
        self.sync_prompt();
        true
    }

    // ---- keys ----

    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut fx = Vec::new();
        if key.kind == KeyEventKind::Release {
            return fx;
        }
        self.dirty = true;
        if self.widgets.overlay_open() {
            self.widgets.handle_key(Panel::Files, key);
            return fx;
        }
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if !ctrl_c && self.quit_armed_until > 0 {
            self.quit_armed_until = 0;
            if self
                .notice
                .as_ref()
                .is_some_and(|n| n.text.starts_with("press ctrl-c"))
            {
                self.notice = None;
            }
        }
        match self.overlay {
            Overlay::Help => self.overlay = Overlay::None,
            Overlay::Switcher => self.switcher_key(key, &mut fx),
            Overlay::None => match self.focus {
                Some(focus) => self.panel_key(focus, key, &mut fx),
                None => self.editor_key(key, &mut fx),
            },
        }
        self.sync_prompt();
        fx
    }

    /// The mouse wheel scrolls the chat (or the open diff). Other mouse events are ignored.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let down = match mouse.kind {
            MouseEventKind::ScrollUp => false,
            MouseEventKind::ScrollDown => true,
            _ => return,
        };
        self.dirty = true;
        if self.widgets.overlay_open() {
            let code = if down { KeyCode::Down } else { KeyCode::Up };
            for _ in 0..WHEEL_LINES {
                self.widgets
                    .handle_key(Panel::Files, KeyEvent::new(code, KeyModifiers::NONE));
            }
        } else if down {
            self.chat.scroll_down(WHEEL_LINES);
        } else {
            self.chat.scroll_up(WHEEL_LINES);
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        self.dirty = true;
        match self.overlay {
            Overlay::Switcher => text
                .chars()
                .filter(|c| !c.is_control())
                .for_each(|c| self.switcher.type_char(c)),
            Overlay::Help => self.overlay = Overlay::None,
            Overlay::None => {
                if self.focus.is_none() && !self.widgets.overlay_open() {
                    self.editor.insert_str(text);
                    self.histories.of(&self.session).reset();
                }
            }
        }
    }

    fn panel_key(&mut self, focus: Focus, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => self.cycle_focus(),
            KeyCode::BackTab => self.cycle_approvals(fx),
            KeyCode::Char('c') if ctrl => self.ctrl_c(fx),
            KeyCode::Char('g') if ctrl => self.toggle_sessions_focus(),
            _ => match focus {
                Focus::Panel(panel) => {
                    if self.widgets.handle_key(panel, key) == WidgetAction::Leave {
                        self.focus = None;
                    }
                }
                Focus::Sessions => match self.sessions.key(key) {
                    SessionsAction::Redraw => {}
                    SessionsAction::Leave => self.focus = None,
                    SessionsAction::Open(info) => {
                        self.focus = None;
                        self.open_session(*info, fx);
                    }
                },
            },
        }
    }

    /// Tab: editor, sessions, plan, files (the ones on screen), editor.
    fn cycle_focus(&mut self) {
        let order = [
            (self.sessions_shown, Focus::Sessions),
            (self.sidebar_shown, Focus::Panel(Panel::Plan)),
            (self.sidebar_shown, Focus::Panel(Panel::Files)),
        ];
        let mut on_screen = order.iter().filter(|(shown, _)| *shown).map(|(_, f)| *f);
        self.focus = match self.focus {
            None => on_screen.next(),
            Some(current) => on_screen.skip_while(|f| *f != current).nth(1),
        };
        self.enter_focus();
    }

    /// Ctrl-G: the sessions sidebar gets the keys (it is switched on first when hidden), or gives
    /// them back.
    fn toggle_sessions_focus(&mut self) {
        if self.focus == Some(Focus::Sessions) {
            self.focus = None;
            return;
        }
        if self.sessions.is_empty() {
            self.notify("no sessions yet", false);
        } else if self.sessions_on && self.sessions_shown {
            self.focus = Some(Focus::Sessions);
            self.enter_focus();
        } else {
            // Switched on now: the next draw knows whether it fits, and focuses it or says why not.
            self.sessions_on = true;
            self.sessions_wanted = true;
        }
    }

    fn enter_focus(&mut self) {
        if self.focus == Some(Focus::Sessions) {
            self.sessions.focus(&self.session);
        }
    }

    /// `/sidebar`: show or hide the sessions sidebar.
    fn toggle_sessions(&mut self) {
        self.sessions_on = !self.sessions_on;
        if !self.sessions_on && self.focus == Some(Focus::Sessions) {
            self.focus = None;
        }
    }

    fn switcher_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('c' | 's') if ctrl => self.overlay = Overlay::None,
            KeyCode::Up => self.switcher.move_by(-1),
            KeyCode::Down => self.switcher.move_by(1),
            KeyCode::Char('p') if ctrl => self.switcher.move_by(-1),
            KeyCode::Char('n') if ctrl => self.switcher.move_by(1),
            KeyCode::Backspace => self.switcher.backspace(),
            KeyCode::Char('u') if ctrl => {
                while !self.switcher.filter.is_empty() {
                    self.switcher.backspace();
                }
            }
            KeyCode::Enter => match self.switcher.selected_entry() {
                Some(Entry::New) => {
                    self.overlay = Overlay::None;
                    fx.push(Effect::CreateSession {
                        cwd: self.cwd.clone(),
                    });
                }
                Some(Entry::Session(i)) => {
                    let picked = self
                        .switcher
                        .sessions
                        .as_ref()
                        .and_then(|s| s.get(i))
                        .cloned();
                    self.overlay = Overlay::None;
                    if let Some(info) = picked {
                        self.open_session(info, fx);
                    }
                }
                None => {}
            },
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.switcher.type_char(c);
            }
            _ => {}
        }
    }

    /// Shows another session and brings the agent to this app (the switcher and the sidebar).
    fn open_session(&mut self, info: SessionInfo, fx: &mut Vec<Effect>) {
        if info.id == self.session {
            return;
        }
        let from = self.session.clone();
        self.begin_session(&info.id, &info.cwd);
        self.awaiting = Some((info.id.clone(), self.now_ms + 3000));
        fx.push(Effect::Switch {
            from: Some(from),
            to: info.id,
            cwd: info.cwd,
            move_agent: true,
        });
    }

    fn open_switcher(&mut self, fx: &mut Vec<Effect>) {
        self.switcher = Switcher::new(&self.session);
        self.overlay = Overlay::Switcher;
        fx.push(Effect::ListSessions);
    }

    /// Shift-Tab: ask, edits, auto, ask…
    fn cycle_approvals(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Input {
            session: self.session.clone(),
            text: format!("/approvals {}", self.approvals.next().word()),
        });
    }

    fn ctrl_c(&mut self, fx: &mut Vec<Effect>) {
        if !self.editor.is_empty() {
            self.editor.clear();
            self.histories.of(&self.session).reset();
        } else if self.quit_armed_until > self.now_ms {
            fx.push(Effect::Quit);
        } else {
            self.quit_armed_until = self.now_ms + QUIT_WINDOW_MS;
            self.notice = Some(Notice {
                text: "press ctrl-c again to quit".into(),
                error: false,
                until: self.quit_armed_until,
            });
        }
    }

    fn esc(&mut self, fx: &mut Vec<Effect>) {
        if self.menu_visible() {
            self.menu_closed_for = Some(self.editor.text().to_string());
        } else if let Some(r) = self.active_ref().map(str::to_string) {
            self.prompt.dismissed = Some(r);
            self.prompt.selected = None;
        } else if self.running {
            fx.push(Effect::Cancel {
                session: self.session.clone(),
            });
            self.notify("cancelling…", false);
        } else if !self.chat.following() {
            self.chat.follow();
        }
    }

    fn editor_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let empty = self.editor.is_empty();
        let prompting = empty && !self.active_buttons().is_empty();
        let mut edited = true;
        match key.code {
            KeyCode::Char(c) if ctrl => edited = self.ctrl_key(c, fx),
            KeyCode::Char(c) if alt => match c {
                'b' => self.editor.word_left(),
                'f' => self.editor.word_right(),
                'd' => self.editor.delete_word_forward(),
                _ => edited = false,
            },
            KeyCode::Char(c) => {
                if prompting
                    && let Some(n) = c.to_digit(10).filter(|n| (1..=9).contains(n))
                    && (n as usize) <= self.active_buttons().len()
                {
                    self.press(n as usize - 1, fx);
                    return;
                }
                if c == '?' && empty {
                    self.overlay = Overlay::Help;
                    return;
                }
                self.editor.insert_char(c);
            }
            KeyCode::Enter if alt || shift => self.editor.newline(),
            KeyCode::Enter => self.enter(fx),
            KeyCode::BackTab => {
                self.cycle_approvals(fx);
                edited = false;
            }
            KeyCode::Tab => {
                edited = false;
                if self.menu_visible() {
                    self.complete_menu();
                } else if !self.active_buttons().is_empty() {
                    self.move_button(1);
                } else {
                    self.cycle_focus();
                }
            }
            KeyCode::Esc => {
                self.esc(fx);
                edited = false;
            }
            KeyCode::Backspace if alt => self.editor.delete_word_back(),
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Left if prompting => {
                self.move_button(-1);
                edited = false;
            }
            KeyCode::Right if prompting => {
                self.move_button(1);
                edited = false;
            }
            KeyCode::Left if ctrl || alt => self.editor.word_left(),
            KeyCode::Right if ctrl || alt => self.editor.word_right(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right => self.editor.right(),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.chat.scroll_up(1);
                edited = false;
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.chat.scroll_down(1);
                edited = false;
            }
            KeyCode::Up => {
                edited = false;
                if self.menu_visible() {
                    self.move_menu(-1);
                } else if prompting {
                    self.move_button(-1);
                } else if !self.editor.up() {
                    self.history_prev();
                }
            }
            KeyCode::Down => {
                edited = false;
                if self.menu_visible() {
                    self.move_menu(1);
                } else if prompting {
                    self.move_button(1);
                } else if !self.editor.down() {
                    self.history_next();
                }
            }
            KeyCode::Home if ctrl || empty => {
                self.chat.to_top();
                edited = false;
            }
            KeyCode::End if ctrl || empty => {
                self.chat.follow();
                edited = false;
            }
            KeyCode::Home => self.editor.line_start(),
            KeyCode::End => self.editor.line_end(),
            KeyCode::PageUp => {
                self.chat.page_up();
                edited = false;
            }
            KeyCode::PageDown => {
                self.chat.page_down();
                edited = false;
            }
            _ => edited = false,
        }
        if edited {
            self.histories.of(&self.session).reset();
        }
    }

    /// Returns whether the editor text was edited.
    fn ctrl_key(&mut self, c: char, fx: &mut Vec<Effect>) -> bool {
        let empty = self.editor.is_empty();
        match c {
            'c' => self.ctrl_c(fx),
            'd' if empty => {
                if self.chat.following() {
                    fx.push(Effect::Quit);
                } else {
                    self.chat.half_page_down();
                }
                return false;
            }
            'd' => self.editor.delete(),
            's' => {
                self.open_switcher(fx);
                return false;
            }
            'o' => {
                self.expanded = !self.expanded;
                return false;
            }
            't' => {
                self.show_thoughts = !self.show_thoughts;
                return false;
            }
            'l' => {
                self.clear = true;
                return false;
            }
            'g' => {
                self.toggle_sessions_focus();
                return false;
            }
            'p' | 'n' => {
                let delta = if c == 'p' { -1 } else { 1 };
                if self.menu_visible() {
                    self.move_menu(delta);
                } else if !self.select_prompt(delta) {
                    if delta < 0 {
                        self.history_prev();
                    } else {
                        self.history_next();
                    }
                }
                return false;
            }
            'a' => self.editor.line_start(),
            'e' => self.editor.line_end(),
            'b' => self.editor.left(),
            'f' => self.editor.right(),
            'w' => self.editor.delete_word_back(),
            'h' => self.editor.backspace(),
            'u' if empty => {
                self.chat.half_page_up();
                return false;
            }
            'u' => self.editor.delete_to_line_start(),
            'k' => self.editor.delete_to_line_end(),
            'j' => self.editor.newline(),
            _ => return false,
        }
        true
    }

    fn history_prev(&mut self) {
        let current = self.editor.text().to_string();
        if let Some(text) = self.histories.of(&self.session).older(&current) {
            self.editor.set_text(&text);
        }
    }

    fn history_next(&mut self) {
        if let Some(text) = self.histories.of(&self.session).newer() {
            self.editor.set_text(&text);
        }
    }

    // ---- sending and commands ----

    fn menu_matches(&self) -> Vec<&'static Command> {
        let text = self.editor.text();
        if self.focus.is_some()
            || self.overlay != Overlay::None
            || !menu::is_slash_word(text)
            || self.menu_closed_for.as_deref() == Some(text)
        {
            return Vec::new();
        }
        menu::matching(text)
    }

    fn menu_visible(&self) -> bool {
        !self.menu_matches().is_empty()
    }

    fn move_menu(&mut self, delta: isize) {
        let n = self.menu_matches().len() as isize;
        if n > 0 {
            self.menu_sel = (self.menu_sel as isize + delta).rem_euclid(n) as usize;
        }
    }

    fn complete_menu(&mut self) {
        let matches = self.menu_matches();
        if let Some(cmd) = matches.get(self.menu_sel.min(matches.len().saturating_sub(1))) {
            self.editor.set_text(&format!("/{} ", cmd.name));
            self.menu_sel = 0;
        }
    }

    fn enter(&mut self, fx: &mut Vec<Effect>) {
        let matches = self.menu_matches();
        if !matches.is_empty() {
            let cmd = matches[self.menu_sel.min(matches.len() - 1)];
            if cmd.needs_args {
                self.editor.set_text(&format!("/{} ", cmd.name));
                self.menu_sel = 0;
                return;
            }
            self.editor.clear();
            self.menu_sel = 0;
            self.run_command(cmd.name, &format!("/{}", cmd.name), fx);
            return;
        }
        let text = self.editor.text().trim().to_string();
        if text.is_empty() {
            if !self.active_buttons().is_empty() {
                self.press(self.prompt.button, fx);
            }
            return;
        }
        self.editor.clear();
        self.menu_sel = 0;
        let local = text
            .starts_with('/')
            .then(|| menu::lookup(&text))
            .flatten()
            .filter(|c| c.local);
        match local {
            Some(cmd) => self.run_command(cmd.name, &text, fx),
            None => self.send(text, fx),
        }
    }

    fn run_command(&mut self, name: &str, text: &str, fx: &mut Vec<Effect>) {
        match name {
            "sessions" => self.open_switcher(fx),
            "sidebar" => self.toggle_sessions(),
            "expand" => self.expanded = !self.expanded,
            "thoughts" => self.show_thoughts = !self.show_thoughts,
            "help" => self.overlay = Overlay::Help,
            "quit" => fx.push(Effect::Quit),
            _ => self.send(text.to_string(), fx),
        }
    }

    fn send(&mut self, text: String, fx: &mut Vec<Effect>) {
        self.histories.record(&self.session, &text);
        self.chat.follow();
        if self.running {
            self.queued += 1;
        }
        fx.push(Effect::Input {
            session: self.session.clone(),
            text,
        });
    }

    // ---- drawing ----

    fn areas(&mut self, area: Rect) -> Areas {
        let side_w = (area.width / 3).clamp(32, 40);
        let side = (area.width >= side_w + 1 + 40 && self.widgets.wants_sidebar(area.width))
            .then_some(side_w);
        let right_w = side.map_or(0, |w| w + 1);
        // The sessions sidebar yields to the plan / files column, and to the chat's width.
        let left = (self.sessions_on
            && !self.sessions.is_empty()
            && area.width >= SIDEBAR_WIDTH + right_w + CHAT_MIN_WIDTH)
            .then_some(Rect {
                width: SIDEBAR_WIDTH,
                ..area
            });
        let left_w = left.map_or(0, |r| r.width);
        let main = Rect {
            x: area.x + left_w,
            width: area.width - left_w - right_w,
            ..area
        };
        let side = side.map(|w| Rect {
            x: main.x + main.width + 1,
            y: area.y,
            width: w,
            height: area.height,
        });
        self.editor
            .set_width(usize::from(main.width.saturating_sub(2)).max(1));
        let wanted = self.editor.height().clamp(1, EDITOR_ROWS) as u16;
        let h = main.height;
        let status_h = h.saturating_sub(3).min(3);
        let rule_h = h.saturating_sub(status_h).min(1);
        let room = h - status_h - rule_h;
        let editor_h = wanted.min(room.saturating_sub(1)).max(room.min(1));
        let chat_h = room - editor_h;
        let at = |y: u16, height: u16| Rect {
            y: main.y + y,
            height,
            ..main
        };
        Areas {
            chat: at(0, chat_h),
            rule: at(chat_h, rule_h),
            editor: at(chat_h + rule_h, editor_h),
            status: at(chat_h + rule_h + editor_h, status_h),
            side,
            left,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.size = (area.width, area.height);
        let areas = self.areas(area);
        self.sidebar_shown = areas.side.is_some();
        self.sessions_shown = areas.left.is_some();
        if std::mem::take(&mut self.sessions_wanted) {
            if self.sessions_shown {
                self.focus = Some(Focus::Sessions);
                self.enter_focus();
            } else {
                self.notify("terminal too narrow for the sessions sidebar", false);
            }
        }
        match self.focus {
            Some(Focus::Panel(_)) if !self.sidebar_shown => self.focus = None,
            Some(Focus::Sessions) if !self.sessions_shown => self.focus = None,
            _ => {}
        }
        self.sync_prompt();

        if self.widgets.overlay_open() {
            self.widgets.draw_overlay(frame, areas.chat);
        } else {
            match self.overlay {
                Overlay::None => self.draw_chat(frame, areas.chat),
                Overlay::Help => self.draw_help(frame, areas.chat),
                Overlay::Switcher => self.draw_switcher(frame, areas.chat),
            }
            self.draw_menu(frame, areas.chat);
        }
        if let Some(left) = areas.left {
            self.draw_sessions(frame, left);
        }
        if let Some(side) = areas.side {
            let panel = match self.focus {
                Some(Focus::Panel(panel)) => Some(panel),
                _ => None,
            };
            self.widgets.draw_sidebar(frame, side, panel);
        }
        if areas.rule.height > 0 {
            let rule = "─".repeat(usize::from(areas.rule.width));
            frame.render_widget(
                Paragraph::new(Span::styled(rule, self.styles.border)),
                areas.rule,
            );
        }
        self.draw_editor(frame, areas.editor);
        self.draw_status(frame, areas.status);
    }

    /// The sessions on the left: the list, then a border and a gap before the chat.
    fn draw_sessions(&mut self, frame: &mut Frame, area: Rect) {
        let inner = area.width.saturating_sub(2);
        let list = Rect {
            width: inner,
            ..area
        };
        let spinner = SPINNER[usize::try_from(self.now_ms / 125).unwrap_or(0) % SPINNER.len()];
        let look = Look {
            current: &self.session,
            focused: self.focus == Some(Focus::Sessions),
            spinner,
            now_ms: self.now_ms,
        };
        let lines = self.sessions.render(
            usize::from(list.width),
            usize::from(list.height),
            &look,
            &self.styles,
        );
        frame.render_widget(Paragraph::new(lines), list);
        for y in area.y..area.bottom() {
            frame.buffer_mut()[(area.x + inner, y)]
                .set_symbol("│")
                .set_style(self.styles.border);
        }
    }

    fn chat_opts(&self) -> ChatOpts {
        ChatOpts {
            expanded: self.expanded,
            show_thoughts: self.show_thoughts,
            palette: self.palette,
            active: self
                .active_ref()
                .map(|r| (r.to_string(), self.prompt.button)),
            marker: self.styles.accent,
        }
    }

    fn draw_chat(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let content = Rect {
            width: if area.width >= 20 {
                area.width - 1
            } else {
                area.width
            },
            ..area
        };
        let opts = self.chat_opts();
        self.chat.layout(content.width, content.height, &opts);
        if self.chat.items().is_empty() {
            let hint = if self.chat.loaded() {
                "message the agent below · / for commands · ctrl-s sessions · /help for keys"
            } else {
                "connecting to the session…"
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate(hint, usize::from(content.width)),
                    self.styles.muted,
                ))),
                content,
            );
            return;
        }
        frame.render_widget(Paragraph::new(self.chat.visible()), content);
        let total = self.chat.total_lines();
        let height = usize::from(content.height);
        if total > height && area.width >= 20 {
            let bar = Rect {
                x: area.x + area.width - 1,
                width: 1,
                ..area
            };
            let mut state = ScrollbarState::new(total - height).position(self.chat.top());
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(None)
                    .thumb_symbol("│")
                    .thumb_style(self.styles.border),
                bar,
                &mut state,
            );
        }
        if !self.chat.following() {
            let text = if self.chat.has_unseen() {
                "↓ new output (End)"
            } else {
                "↓ End to follow"
            };
            let w = width(text) as u16;
            if content.width > w + 2 {
                let at = Rect {
                    x: content.x + content.width - w - 1,
                    y: content.y + content.height - 1,
                    width: w,
                    height: 1,
                };
                frame.render_widget(Clear, at);
                frame.render_widget(Paragraph::new(Span::styled(text, self.styles.muted)), at);
            }
        }
    }

    fn draw_menu(&mut self, frame: &mut Frame, chat: Rect) {
        let matches = self.menu_matches();
        if matches.is_empty() || chat.height == 0 {
            return;
        }
        self.menu_sel = self.menu_sel.min(matches.len() - 1);
        // A rule above the entries sets them off from the chat.
        let rows = matches
            .len()
            .min(MENU_ROWS)
            .min(usize::from(chat.height).saturating_sub(1));
        if rows == 0 {
            return;
        }
        let start = (self.menu_sel + 1).saturating_sub(rows);
        let name_w = matches
            .iter()
            .map(|c| width(c.name) + 1 + width(c.args) + usize::from(!c.args.is_empty()))
            .max()
            .unwrap_or(0)
            .min(32);
        let w = usize::from(chat.width);
        let lines: Vec<Line<'static>> = matches
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(i, c)| {
                let selected = i == self.menu_sel;
                let label = if c.args.is_empty() {
                    format!("/{}", c.name)
                } else {
                    format!("/{} {}", c.name, c.args)
                };
                let pad = " ".repeat((name_w + 1).saturating_sub(width(&label)));
                let (marker, name_style) = if selected {
                    ("› ", self.styles.accent.add_modifier(Modifier::BOLD))
                } else {
                    ("  ", Style::new())
                };
                fit_line(
                    Line::from(vec![
                        Span::styled(marker, self.styles.accent),
                        Span::styled(label, name_style),
                        Span::raw(format!("{pad}  ")),
                        Span::styled(c.desc, self.styles.muted),
                    ]),
                    w,
                )
            })
            .collect();
        let rule = Line::from(Span::styled("─".repeat(w), self.styles.border));
        let lines: Vec<Line<'static>> = std::iter::once(rule).chain(lines).collect();
        let rect = Rect {
            y: chat.y + chat.height - rows as u16 - 1,
            height: rows as u16 + 1,
            ..chat
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(lines), rect);
    }

    fn draw_editor(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let prompt = Span::styled("> ", self.styles.accent);
        if self.editor.is_empty() {
            let hint = truncate(
                "message the agent · / for commands",
                usize::from(area.width.saturating_sub(2)),
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    prompt,
                    Span::styled(hint, self.styles.muted),
                ])),
                area,
            );
        } else {
            let rows = self.editor.rows();
            let (row, col) = self.editor.cursor_pos();
            let h = usize::from(area.height);
            let start = if row >= h { row + 1 - h } else { 0 };
            let lines: Vec<Line<'static>> = rows
                .iter()
                .enumerate()
                .skip(start)
                .take(h)
                .map(|(i, r)| {
                    let lead = if i == 0 {
                        prompt.clone()
                    } else {
                        Span::raw("  ")
                    };
                    let text = self.editor.text()[r.start..r.end]
                        .trim_end_matches('\n')
                        .to_string();
                    Line::from(vec![lead, Span::raw(text)])
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), area);
            if self.focus.is_none() && self.overlay != Overlay::Switcher {
                let x = (2 + col).min(usize::from(area.width) - 1) as u16;
                frame.set_cursor_position(Position::new(area.x + x, area.y + (row - start) as u16));
            }
            return;
        }
        if self.focus.is_none() && self.overlay != Overlay::Switcher {
            frame.set_cursor_position(Position::new(area.x + 2.min(area.width - 1), area.y));
        }
    }

    fn status_lines(&self, width: usize) -> [Line<'static>; 3] {
        let cwd = shorten_home(&self.cwd, self.home.as_deref());
        let branch = self.widgets.branch();
        let agent_in = self
            .agent_app
            .as_deref()
            .filter(|a| *a != self.app_id.as_str());
        let notice = self.notice.as_ref().map(|n| (n.text.as_str(), n.error));
        Status {
            cwd: &cwd,
            branch: branch.as_deref(),
            settings: &self.settings,
            harness: &self.harness,
            agent_in,
            usage: self.widgets.usage_line(),
            working_secs: self
                .running
                .then(|| u64::try_from((self.now_ms - self.turn_started_ms) / 1000).unwrap_or(0)),
            spinner: SPINNER[usize::try_from(self.now_ms / 125).unwrap_or(0) % SPINNER.len()],
            queued: self.queued,
            connected: self.connected,
            approvals: self.approvals,
            notice,
            styles: &self.styles,
        }
        .lines(width)
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let lines = self.status_lines(usize::from(area.width));
        frame.render_widget(Paragraph::new(lines.to_vec()), area);
    }

    fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let key = |k: &str, what: &str| {
            Line::from(vec![
                Span::styled(format!("  {k:<26}"), self.styles.accent),
                Span::raw(what.to_string()),
            ])
        };
        let head = |t: &str| Line::from(Span::styled(t.to_string(), Modifier::BOLD));
        let mut lines = vec![
            head("keys (any key closes this)"),
            key("enter", "send"),
            key("alt-enter, ctrl-j", "new line"),
            key("up / down", "move in the text, then through history"),
            key("ctrl-a / ctrl-e", "start / end of line"),
            key("alt-b / alt-f", "word left / right"),
            key(
                "ctrl-w / ctrl-u / ctrl-k",
                "delete word / to line start / to line end",
            ),
            key("pgup / pgdn", "scroll the chat (also the mouse wheel)"),
            key("shift-up / shift-down", "scroll the chat a line"),
            key(
                "ctrl-u / ctrl-d",
                "half page up / down (empty input; ctrl-d quits at the end)",
            ),
            key(
                "home / end",
                "top / bottom of the chat (empty input; ctrl-home/end always)",
            ),
            key("ctrl-o", "full tool results"),
            key("ctrl-t", "show thoughts"),
            key("esc", "close menu, dismiss prompt, cancel the running turn"),
            key("ctrl-c", "clear input; twice to quit"),
            key("shift-tab", "approvals: ask / accept edits / auto"),
            key("tab", "complete command, next button, next panel"),
            key("ctrl-s", "switch session (picker)"),
            key("ctrl-g", "sessions sidebar: focus (up/down, enter, esc)"),
            key("1-9, arrows, enter", "answer a prompt (empty input)"),
            key("ctrl-p / ctrl-n", "pick an older prompt"),
            Line::default(),
            head("commands"),
        ];
        for c in menu::COMMANDS {
            let label = if c.args.is_empty() {
                format!("/{}", c.name)
            } else {
                format!("/{} {}", c.name, c.args)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<30}"), self.styles.accent),
                Span::styled(c.desc, self.styles.muted),
            ]));
        }
        frame.render_widget(Clear, area);
        let lines = lines
            .into_iter()
            .map(|l| fit_line(l, usize::from(area.width)))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_switcher(&self, frame: &mut Frame, area: Rect) {
        let w = usize::from(area.width);
        let s = &self.switcher;
        let mut lines = vec![
            Line::from(vec![
                Span::styled("sessions", Modifier::BOLD),
                Span::styled(
                    "  type to filter · enter switch · esc close",
                    self.styles.muted,
                ),
            ]),
            Line::from(vec![
                Span::styled("> ", self.styles.accent),
                Span::raw(s.filter.clone()),
            ]),
        ];
        let entries = s.entries();
        let rows = usize::from(area.height).saturating_sub(lines.len()).max(1);
        let start = (s.selected + 1).saturating_sub(rows);
        for (i, entry) in entries.iter().enumerate().skip(start).take(rows) {
            let selected = i == s.selected;
            let (marker, style) = if selected {
                ("› ", self.styles.accent.add_modifier(Modifier::BOLD))
            } else {
                ("  ", Style::new())
            };
            let mut spans = vec![Span::styled(marker, self.styles.accent)];
            match entry {
                Entry::New => spans.push(Span::styled(
                    format!(
                        "+ new session in {}",
                        shorten_home(&self.cwd, self.home.as_deref())
                    ),
                    style,
                )),
                Entry::Session(idx) => {
                    let info = &s.sessions.as_ref().expect("entries come from the list")[*idx];
                    let (name, rest) = menu::session_label(info, self.home.as_deref(), self.now_ms);
                    let here = if info.id == s.current { "● " } else { "  " };
                    spans.push(Span::styled(here, self.styles.success));
                    spans.push(Span::styled(truncate(&name, 40.min(w / 2)), style));
                    spans.push(Span::styled(format!("  {rest}"), self.styles.muted));
                }
            }
            lines.push(fit_line(Line::from(spans), w));
        }
        if s.sessions.is_none() {
            let (text, style) = match &s.error {
                Some(e) => (format!("could not list sessions: {e}"), self.styles.danger),
                None => ("loading…".to_string(), self.styles.muted),
            };
            lines.push(Line::from(Span::styled(text, style)));
        } else if entries.len() == 1 && !s.filter.is_empty() {
            lines.push(Line::from(Span::styled(
                "no session matches",
                self.styles.muted,
            )));
        }
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines), area);
    }
}

struct Areas {
    chat: Rect,
    rule: Rect,
    editor: Rect,
    status: Rect,
    side: Option<Rect>,
    /// The sessions sidebar.
    left: Option<Rect>,
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
