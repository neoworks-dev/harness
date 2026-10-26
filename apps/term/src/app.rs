//! The app state: chat, editor, overlays and keys, with no terminal or network in it.
//!
//! [`App::handle_key`], [`App::handle_event`] and [`App::handle_msg`] change the state and return
//! [`Effect`]s for the runtime to carry out against the daemon; [`App::draw`] paints it.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use neo_protocol::{
    AgentFocus, SecretRequest, SessionEvent, SessionEventRecord, SessionInfo, SessionState,
    SettingName, SlashCommand, WaitingInput,
};
use neo_renderer_ratatui::{
    ButtonRef, ImageStatus, Options, Palette, UiKind, UiTarget, openui_targets,
};
use neo_sdk::Event;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::agents::{Agents, AgentsAction, MainRow, Transcript};
use crate::attach;
use crate::chat::{self, Chat, ChatOpts};
use crate::choice;
use crate::editor::Editor;
use crate::forms::{Forms, Outcome, Pending};
use crate::github::{Github, Outgoing};
use crate::history::{Histories, HistoryStore};
use crate::images::Images;
use crate::jobs::{Jobs, JobsAction};
use crate::mention::{self, Candidate, Mentions};
use crate::menu::{self, Entry, Item, Switcher, session_title};
use crate::secret::{self, Secret, SecretValue};
use crate::sessions::{Look, SIDEBAR_WIDTH, Sessions, SessionsAction, sidebar_width};
use crate::status::{Approvals, HarnessSettings, Status, record_setting};
use crate::style::{Styles, plain_palette};
use crate::theme::{Prefs, Theme};
use crate::util::{fit_line, relative_time, shorten_home, split_line, truncate, width};
use crate::widgets::{Panel, WidgetAction, Widgets};

/// How long a second Ctrl-C quits.
const QUIT_WINDOW_MS: i64 = 1500;
/// How long a transient message stays.
const NOTICE_MS: i64 = 4000;
/// How often a running turn's session info is refreshed (queue length, stuck turns).
const POLL_MS: i64 = 2000;
/// Idle this long, a session's prompt cache is gone with Anthropic (5 minutes, 1 hour with long
/// retention) and OpenAI (5 minutes to an hour); DeepSeek's disk cache may outlast it.
const CACHE_COLD_MS: i64 = 60 * 60 * 1000;
/// Rows of the slash menu.
const MENU_ROWS: usize = 8;
/// Chat lines one wheel notch scrolls.
const WHEEL_LINES: usize = 3;
/// Most rows the editor grows to.
const EDITOR_ROWS: usize = 8;
/// Rows a pasted image's thumbnail takes above the editor (its marker goes under it), and the
/// most columns.
const PREVIEW_ROWS: u16 = 6;
const PREVIEW_COLS: u16 = 24;
/// Most rows the inputs waiting behind the running turn take above the editor.
const WAITING_ROWS: usize = 3;
const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
/// The chat needs at least this much beside the sessions sidebar, else the sidebar is hidden.
const CHAT_MIN_WIDTH: u16 = 52;
/// The narrowest the plan / files column gets.
const SIDE_MIN_WIDTH: u16 = 24;
/// Sessions an empty chat lists from the same repository.
const EMPTY_RECENT: usize = 5;

/// The terminal panel's width: two fifths of what the chat would have, within these.
const TERMINAL_MIN_WIDTH: u16 = 40;
const TERMINAL_MAX_WIDTH: u16 = 100;
/// Columns Ctrl-] / Ctrl-[ widen or narrow a pane by.
const RESIZE_STEP: i16 = 4;
/// Most rows a prompt takes where the editor is (its question, preview, buttons and hint).
const PANEL_ROWS: u16 = 16;
/// How long the editor must have been idle before a prompt takes its place.
const TYPING_PAUSE_MS: i64 = 800;
/// The harness a session runs on until a `/harness` setting says otherwise.
const DEFAULT_HARNESS: &str = "neo";

/// What the runtime does on the app's behalf. Requests to the daemon block, so the runtime runs
/// them off the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send input as this app (the daemon follows it: the agent moves here).
    Input {
        session: String,
        text: String,
        /// Absolute paths of attached images.
        images: Vec<String>,
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
    /// Follow the events of a session a subagent runs as (its activity in the agents list).
    Watch {
        session: String,
    },
    /// Follow (`on`) or stop following the view of one agent's work in a session.
    AgentView {
        session: String,
        agent: String,
        on: bool,
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
    /// Answer a `secret.request`: the typed value, or none when cancelled. `Debug` hides it.
    Secret {
        id: String,
        value: Option<SecretValue>,
    },
    /// Put text on the clipboard (OSC 52).
    Copy(String),
    /// Save the clipboard's image to a file and attach it (`Msg::Pasted`).
    PasteImage,
    /// Take this app's inputs back out of the session's queue (`Msg::Unqueued`).
    Unqueue {
        session: String,
    },
    /// Open a link of an OpenUI button in the browser (xdg-open).
    OpenUrl(String),
    /// Open a file in the user's editor at a line, the screen handed over until it exits.
    Edit {
        path: std::path::PathBuf,
        line: Option<usize>,
    },
    Quit,
}

/// Answers from the runtime's worker thread.
#[derive(Debug)]
pub enum Msg {
    Sessions(Result<Vec<SessionInfo>, String>),
    Created(Result<Box<SessionInfo>, String>),
    Error(String),
    /// Something to tell the user that is not an error.
    Notice(String),
    /// The clipboard's image, saved; none when it holds no image.
    Pasted(Option<String>),
    /// The inputs taken back out of the queue of `session`, oldest first.
    Unqueued {
        session: String,
        inputs: Vec<WaitingInput>,
    },
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
    /// The chat's file lines: the edits and each turn's changed files.
    Chat,
    Sessions,
    Panel(Panel),
    /// The subagents list under the prompt bar.
    Agents,
    /// The background commands under the prompt bar (and the subagents).
    Jobs,
}

/// A pane Ctrl-] / Ctrl-[ resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Sessions,
    Side,
}

/// Columns each pane was widened (or, negative, narrowed) by with Ctrl-] / Ctrl-[.
#[derive(Debug, Clone, Copy, Default)]
struct Resized {
    sessions: i16,
    side: i16,
}

/// `width` changed by `delta`, within `min` and `max` (`max` wins when they cross).
fn resized(width: u16, delta: i16, min: u16, max: u16) -> u16 {
    let w = (i32::from(width) + i32::from(delta)).clamp(0, i32::from(u16::MAX)) as u16;
    w.max(min).min(max)
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
    /// Picked by hand (Ctrl-P/N) instead of the oldest pending one.
    selected: Option<String>,
    /// Prompts that were dismissed (Esc) or answered: not the active one any more.
    dismissed: Vec<String>,
    button: usize,
    /// What was typed to find an option of a long choice.
    filter: String,
    /// The prompt last seen as active, to notice a new one.
    seen: Option<String>,
    /// The prompt has taken the editor's place: keys go to it alone.
    engaged: bool,
    /// When the user last typed in the editor (ms): a prompt waits for a pause.
    typed_ms: i64,
    /// The first button line shown when the buttons don't fit the panel; None: not drawn yet.
    panel_top: Cell<Option<usize>>,
}

/// The keys line under a prompt in the input area.
fn keys_hint(focused: Option<&ButtonRef>, buttons: usize, filter: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(button) = focused {
        parts.push(format!("enter {}", button.label.to_lowercase()));
        if takes_note(button) {
            parts.push("tab add a note".to_string());
        }
    }
    if buttons > 1 {
        parts.insert(1.min(parts.len()), "←/→ choose".to_string());
    }
    parts.push(
        match filter {
            Some(f) if !f.is_empty() => "esc clear",
            _ => "esc dismiss",
        }
        .to_string(),
    );
    parts.join(" · ")
}

/// A note being typed for the answer of the focused button of a permission or choice card.
#[derive(Debug)]
struct Note {
    /// The card's item ref.
    r#ref: String,
    /// The button the note goes with.
    button: usize,
    text: String,
}

/// Buttons that answer a question of the daemon's can carry a note for the agent.
fn takes_note(button: &ButtonRef) -> bool {
    matches!(button.action.as_str(), "neo.permit" | "neo.choose")
        && button
            .args
            .as_ref()
            .is_some_and(serde_json::Value::is_object)
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
    /// Where `/theme` is kept.
    pub prefs: Prefs,
}

pub struct App {
    app_id: String,
    session: String,
    cwd: String,
    home: Option<String>,
    no_color: bool,
    /// What is drawn with: the theme's colors (`/theme`) made of the daemon's.
    palette: Palette,
    /// The daemon's theme, as it last said.
    daemon_palette: Palette,
    theme: Theme,
    prefs: Prefs,
    styles: Styles,

    widgets: Widgets,
    github: Github,
    /// The subagents running (or just ended) under the prompt bar, and the transcript of one.
    agents: Agents,
    jobs: Jobs,
    /// Background commands listed at the last tick, and its time: a change redraws.
    jobs_shown: usize,
    last_tick_ms: i64,
    transcript: Option<Transcript>,
    /// Last whole second the agents list was painted for (its clocks run).
    agents_tick: i64,
    chat: Chat,
    /// Pictures in the chat; None shows the `[image …]` placeholder.
    images: Option<Images>,
    /// File type icons in the chat and the files panel (`NEO_TERM_ICONS=off`: none).
    icons: bool,
    /// `/settings split-diffs`: diffs side by side, in the chat and the file view.
    split_diffs: bool,
    /// What Ctrl-] / Ctrl-[ did to the panes' widths.
    resized: Resized,
    /// The daemon's connectors (apps and services), for the sidebar's servers.
    connectors: Vec<neo_protocol::ConnectorInfo>,
    editor: Editor,
    /// Images pasted into the draft: the number of the `[image N]` marker that places each, and
    /// its path. One whose marker is deleted is not sent.
    attachments: Vec<(usize, String)>,
    /// The draft Ctrl-S put aside, with its images; it comes back with Ctrl-S or once the one
    /// written meanwhile is sent.
    stash: Option<(String, Vec<(usize, String)>)>,
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
    /// The visual state of the OpenUI docs in the chat and the values on their way out.
    forms: Forms,
    /// The note typed for the focused button's answer (Tab on a card).
    note: Option<Note>,
    /// A secret the daemon asked for: a masked field that has the keys and the editor's place.
    secret: Option<Secret>,
    menu_sel: usize,
    /// The editor text the slash menu was closed for.
    menu_closed_for: Option<String>,
    /// The `@` file list: the walked trees, the selected row, and the text Esc closed it for.
    mentions: Mentions,
    mention_sel: usize,
    mention_closed_for: Option<String>,

    expanded: bool,
    show_thoughts: bool,
    running: bool,
    turn_started_ms: i64,
    /// The inputs waiting behind the running turn (the daemon's list; ours until it answers).
    waiting: Vec<WaitingInput>,
    settings: Vec<HarnessSettings>,
    /// The session's agent, from `/harness`.
    harness: String,
    /// The slash commands each harness listed in this session (`commands_available`).
    offered: HashMap<String, Vec<SlashCommand>>,
    approvals: Approvals,
    connected: bool,
    /// Where the agent is (the last `agent.focus`).
    agent_app: Option<String>,
    /// After moving the agent here, focus events of other sessions are stale until this one
    /// shows up (or the time is up).
    awaiting: Option<(String, i64)>,

    notice: Option<Notice>,
    /// The session's last turn is more than `CACHE_COLD_MS` ago: the next one starts uncached.
    cache_cold: bool,
    quit_armed_until: i64,
    now_ms: i64,
    last_poll_ms: i64,
    last_spin: i64,
    size: (u16, u16),
    sidebar_shown: bool,
    /// `/terminal` hides the terminal panel; it shows while there are commands and room.
    terminal_on: bool,
    /// Where the terminal panel was drawn last (the wheel scrolls it there).
    terminal_area: Option<Rect>,
    dirty: bool,
    clear: bool,
}

impl App {
    pub fn new(config: Config) -> App {
        let theme = config.prefs.theme();
        let palette = if config.no_color {
            plain_palette()
        } else {
            theme.palette(&Palette::default())
        };
        let widgets = Widgets::new(PathBuf::from(&config.cwd), palette);
        let now = crate::util::now_ms();
        let session = config.session.clone();
        let chat = Chat::for_app(&config.app_id).in_dir(&config.cwd);
        App {
            widgets,
            github: Github::new(PathBuf::from(&config.cwd), palette),
            agents: Agents::default(),
            jobs: Jobs::default(),
            jobs_shown: 0,
            last_tick_ms: 0,
            transcript: None,
            agents_tick: 0,
            styles: Styles::new(&palette, config.no_color),
            palette,
            daemon_palette: Palette::default(),
            theme,
            prefs: config.prefs,
            app_id: config.app_id,
            session: config.session,
            cwd: config.cwd,
            home: config.home,
            no_color: config.no_color,
            chat,
            images: None,
            icons: false,
            // Until the daemon's user.settings says otherwise.
            split_diffs: false,
            resized: Resized::default(),
            connectors: Vec::new(),
            editor: Editor::default(),
            attachments: Vec::new(),
            stash: None,
            histories: Histories::new(config.history),
            overlay: Overlay::None,
            switcher: Switcher::new(&session),
            focus: None,
            sessions: Sessions::default(),
            sessions_on: true,
            sessions_shown: false,
            sessions_wanted: false,
            prompt: PromptState::default(),
            forms: Forms::default(),
            note: None,
            secret: None,
            menu_sel: 0,
            menu_closed_for: None,
            mentions: Mentions::default(),
            mention_sel: 0,
            mention_closed_for: None,
            expanded: false,
            show_thoughts: false,
            running: false,
            turn_started_ms: now,
            waiting: Vec::new(),
            settings: Vec::new(),
            harness: DEFAULT_HARNESS.to_string(),
            offered: HashMap::new(),
            approvals: Approvals::Ask,
            connected: true,
            agent_app: None,
            awaiting: Some((session, now + 3000)),
            notice: None,
            cache_cold: false,
            quit_armed_until: 0,
            now_ms: now,
            last_poll_ms: now,
            last_spin: 0,
            size: (80, 24),
            sidebar_shown: false,
            terminal_on: true,
            terminal_area: None,
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

    /// Shows images in the chat (the runtime hands this over once it knows the terminal).
    pub fn set_images(&mut self, images: Images) {
        self.images = Some(images);
        self.dirty = true;
    }

    /// File type icons (Nerd Font glyphs) in the chat and the files panel.
    pub fn set_icons(&mut self, on: bool) {
        self.icons = on;
        self.widgets.set_icons(on);
        self.dirty = true;
    }

    pub fn widgets(&self) -> &Widgets {
        &self.widgets
    }

    #[cfg(test)]
    pub(crate) fn github_mut(&mut self) -> &mut Github {
        &mut self.github
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
        // Background commands' output, their seconds counting up, one leaving the list.
        let tick_s = now_ms / 1000 != self.last_tick_ms / 1000;
        self.last_tick_ms = now_ms;
        let jobs = self.widgets.jobs(now_ms).len();
        if self.widgets.tick(now_ms) || tick_s && jobs > 0 || jobs != self.jobs_shown {
            self.jobs_shown = jobs;
            self.dirty = true;
        }
        if let Some(images) = self.images.as_mut() {
            self.dirty |= images.poll();
        }
        let gh = self.github.tick();
        self.dirty |= gh.changed;
        if let Some(out) = gh.out {
            self.take_github(out, &mut fx);
        }
        if self.running || self.sessions_shown && self.sessions.any_running() {
            let frame = now_ms / 80;
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
        // The session's last event marks its last turn; one nothing was said in has no cache.
        let cold = !self.running
            && !self.chat.items().is_empty()
            && self
                .sessions
                .get(&self.session)
                .is_some_and(|s| now_ms - s.last_activity > CACHE_COLD_MS);
        if cold != self.cache_cold {
            self.cache_cold = cold;
            self.dirty = true;
        }
        self.update_engaged();
        self.send_due(&mut fx);
        if self.agents.ticks(now_ms, self.agents_tick) {
            self.dirty = true;
        }
        self.agents_tick = now_ms;
        if self.focus == Some(Focus::Agents) && !self.agents.visible(now_ms) {
            self.focus = None;
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

    /// Draws with the theme's colors from now on (none with `NO_COLOR`).
    fn apply_palette(&mut self) {
        if self.no_color {
            return;
        }
        self.palette = self.theme.palette(&self.daemon_palette);
        self.styles = Styles::new(&self.palette, false);
        self.widgets.set_palette(self.palette);
        self.github.set_palette(self.palette);
    }

    /// `/theme [dark|light|terminal]`; no name: the next one.
    fn switch_theme(&mut self, text: &str) {
        let arg = text.split_whitespace().nth(1);
        let theme = match arg {
            None => self.theme.next(),
            Some(name) => match Theme::parse(name) {
                Some(theme) => theme,
                None => {
                    self.notify("usage: /theme [dark|light|terminal]", true);
                    return;
                }
            },
        };
        self.theme = theme;
        self.apply_palette();
        match self.prefs.set_theme(theme) {
            Ok(()) if self.no_color => {
                self.notify(format!("theme: {} (NO_COLOR is set)", theme.name()), false)
            }
            Ok(()) => self.notify(format!("theme: {}", theme.name()), false),
            Err(why) => self.notify(format!("theme: {} (not kept: {why})", theme.name()), true),
        }
    }

    /// Changes the shared todo list as the user (`neo.todos`, silent: no turn, no chat line).
    fn change_todos(&mut self, args: serde_json::Value, fx: &mut Vec<Effect>) {
        fx.push(Effect::Action {
            session: self.session.clone(),
            item: "todos".into(),
            action: "neo.todos".into(),
            args: Some(args),
        });
    }

    /// `/todo <text>`: a new item on the shared list.
    fn add_todo(&mut self, text: &str, fx: &mut Vec<Effect>) {
        let item = text.trim_start().strip_prefix("/todo").unwrap_or("").trim();
        if item.is_empty() {
            self.notify("usage: /todo <what to do>", true);
            return;
        }
        self.change_todos(serde_json::json!({ "action": "add", "text": item }), fx);
    }

    /// Ctrl-] / Ctrl-[: the focused pane (else the side column while it holds the terminal panel,
    /// else the sessions sidebar) gets `by` columns more or less, within what leaves the chat its
    /// room.
    fn resize_pane(&mut self, by: i16) {
        let pane = match self.focus {
            Some(Focus::Sessions) => Pane::Sessions,
            Some(Focus::Panel(_)) | Some(Focus::Jobs) => Pane::Side,
            _ if self.terminal_area.is_some() => Pane::Side,
            _ if self.sessions_shown => Pane::Sessions,
            _ if self.sidebar_shown => Pane::Side,
            _ => return self.notify("no pane to resize", false),
        };
        let delta = match pane {
            Pane::Sessions => &mut self.resized.sessions,
            Pane::Side => &mut self.resized.side,
        };
        *delta = delta.saturating_add(by);
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
            Event::Connectors(connectors) => self.connectors = connectors,
            // `/settings`' side-by-side diffs, for the chat and the file view.
            Event::UserSettings(settings) => {
                self.split_diffs = settings.split_diffs;
                self.widgets.set_split_diffs(settings.split_diffs);
            }
            Event::SessionChanged(info) => {
                if info.id == self.session {
                    self.sync_info(&info);
                }
                self.sessions.upsert(info);
            }
            Event::SessionRemoved(id) => self.sessions.remove(&id),
            Event::Session(record) => {
                if record.session == self.session {
                    self.on_record(&record);
                    for session in self.agents.take_wanted() {
                        fx.push(Effect::Watch { session });
                    }
                } else if self.agents.follows(&record.session) {
                    self.agents.on_child_record(&record);
                }
            }
            Event::Subscribed { session, .. } => {
                if session.id == self.session {
                    self.waiting = session.waiting.clone().unwrap_or_default();
                    if session.state == SessionState::Running && !self.running {
                        self.running = true;
                        self.turn_started_ms = self.now_ms;
                    } else if session.state == SessionState::Idle {
                        self.running = false;
                    }
                }
            }
            Event::Connection(up) => {
                self.connected = up;
                // The daemon drops a question with its connection: an answer would find none.
                if !up && self.secret.take().is_some() {
                    self.notify("connection lost: the question was dropped", true);
                }
            }
            Event::Secret(request) => self.on_secret(request, &mut fx),
            Event::Theme(theme) => {
                self.daemon_palette = Palette::from_theme(&theme);
                self.apply_palette();
            }
            Event::ViewSnapshot(snapshot) => {
                if snapshot.session == self.session {
                    match (&snapshot.agent, self.transcript.as_mut()) {
                        (None, _) => {
                            self.chat.apply_snapshot(snapshot);
                            self.refresh_forms();
                            self.sync_prompt();
                        }
                        (Some(agent), Some(t)) if *agent == t.agent.harness => {
                            t.chat.apply_snapshot(snapshot);
                        }
                        _ => {}
                    }
                }
            }
            Event::ViewPatch(patch) => {
                if patch.session == self.session {
                    match (&patch.agent, self.transcript.as_mut()) {
                        (None, _) => {
                            self.chat.apply_patch(patch);
                            self.refresh_forms();
                            self.sync_prompt();
                        }
                        (Some(agent), Some(t)) if *agent == t.agent.harness => {
                            t.chat.apply_patch(patch);
                        }
                        _ => {}
                    }
                }
            }
            Event::FileChanged(changed) => self.widgets.on_file_changed(&changed.path),
        }
        self.dirty = true;
        fx
    }

    /// The daemon asks this app alone for a secret. It is shown in the editor's place when the
    /// question is for the shown session, else answered as cancelled (nothing could show it). A
    /// newer question replaces the open one, which is answered as cancelled.
    fn on_secret(&mut self, request: SecretRequest, fx: &mut Vec<Effect>) {
        if request.session != self.session {
            fx.push(Effect::Secret {
                id: request.id,
                value: None,
            });
            return;
        }
        self.cancel_secret(fx);
        self.secret = Some(Secret::new(request.id, request.title, request.placeholder));
        self.chat.follow();
    }

    /// Closes the open secret question, if any, answering it as cancelled.
    fn cancel_secret(&mut self, fx: &mut Vec<Effect>) {
        if let Some(secret) = self.secret.take() {
            fx.push(Effect::Secret {
                id: secret.id.clone(),
                value: None,
            });
        }
    }

    /// Keys while a secret is asked for: they are all the field's. Enter answers, Esc cancels.
    fn secret_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let Some(secret) = self.secret.as_mut() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let value = match secret.key(key.code, ctrl, alt) {
            secret::Outcome::Typing => return,
            secret::Outcome::Submit(value) => Some(value),
            secret::Outcome::Cancel => None,
        };
        if let Some(Secret { id, .. }) = self.secret.take() {
            fx.push(Effect::Secret { id, value });
        }
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
            // Someone made another session current while the agent is here (/clear, `neo
            // move`): follow it. A new conversation starts in the same directory.
            let cwd = self.cwd.clone();
            let from = self.session.clone();
            self.cancel_secret(fx);
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
        self.agents.on_record(record);
        match &record.event {
            SessionEvent::UserMessage(message) => {
                // A turn starts with it, or the running one took it in.
                if !self.running {
                    self.turn_started_ms = record.ts;
                }
                self.running = true;
                let text = message.shown.as_ref().unwrap_or(&message.text);
                if let Some(at) = self.waiting.iter().position(|w| w.text == *text) {
                    self.waiting.remove(at);
                }
            }
            SessionEvent::TurnEnded(_) => self.running = false,
            SessionEvent::CommandsAvailable(offer) => {
                self.offered
                    .insert(offer.harness.clone(), offer.commands.clone());
            }
            SessionEvent::SettingChanged(change) => {
                let option = match change.option {
                    SettingName::Model => "model",
                    SettingName::Effort => "effort",
                    SettingName::Mode => "mode",
                    SettingName::Approvals => "approvals",
                    SettingName::Harness => "harness",
                    SettingName::TitleModel => "title_model",
                };
                self.on_setting(&change.harness, option, &change.value);
            }
            _ => {}
        }
    }

    /// Neo's own settings (approvals, harness) and each harness's (model, effort, mode).
    fn on_setting(&mut self, harness: &str, option: &str, value: &str) {
        match (harness, option) {
            ("daemon", "approvals") => {
                if let Some(mode) = Approvals::parse(value) {
                    self.approvals = mode;
                }
            }
            ("daemon", "harness") => self.harness = value.to_string(),
            ("daemon", _) | (_, "approvals" | "harness") => {}
            _ => record_setting(&mut self.settings, harness, option, value),
        }
    }

    /// What the daemon says about the shown session: how many inputs wait, whether a turn still
    /// runs (a turn that just started is trusted for a few seconds), and where it works (/cd
    /// moves it: the files, git and paths follow).
    fn sync_info(&mut self, info: &SessionInfo) {
        if info.cwd != self.cwd {
            self.cwd = info.cwd.clone();
            self.chat.set_dir(&info.cwd);
            self.widgets.move_to(PathBuf::from(&info.cwd));
            self.github.reset(PathBuf::from(&info.cwd));
        }
        self.waiting = info.waiting.clone().unwrap_or_default();
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
                let info = *info;
                self.sessions.upsert(info.clone());
                let from = self.session.clone();
                self.cancel_secret(&mut fx);
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
            Msg::Notice(text) => self.notify(text, false),
            Msg::Pasted(Some(path)) => self.attach(vec![path]),
            Msg::Pasted(None) => self.notify("no image on the clipboard", false),
            Msg::Unqueued { session, inputs } if session == self.session => self.redraft(inputs),
            Msg::Unqueued { .. } => {}
        }
        fx
    }

    /// Shows another session: everything of the old one is dropped.
    fn begin_session(&mut self, id: &str, cwd: &str) {
        self.session = id.to_string();
        self.cwd = cwd.to_string();
        self.chat = Chat::for_app(&self.app_id).in_dir(cwd);
        self.running = false;
        self.turn_started_ms = self.now_ms;
        self.waiting.clear();
        self.settings.clear();
        self.harness = DEFAULT_HARNESS.to_string();
        self.offered.clear();
        self.approvals = Approvals::Ask;
        self.editor.clear();
        self.attachments.clear();
        self.prompt = PromptState::default();
        self.forms = Forms::default();
        self.note = None;
        self.secret = None;
        self.overlay = Overlay::None;
        self.focus = None;
        self.menu_sel = 0;
        self.menu_closed_for = None;
        self.widgets.reset(PathBuf::from(cwd));
        self.github.reset(PathBuf::from(cwd));
        self.agents = Agents::default();
        self.transcript = None;
        self.dirty = true;
    }

    // ---- prompts (items with buttons) ----

    /// The prompt the keys answer: one picked by hand, else the oldest one still pending.
    fn active_ref(&self) -> Option<&str> {
        if let Some(r) = &self.prompt.selected
            && self.chat.is_prompt(r)
        {
            return Some(r);
        }
        self.chat
            .pending_prompts()
            .into_iter()
            .find(|r| !self.prompt.dismissed.iter().any(|d| d == r))
    }

    /// The pending prompts that are not dismissed, and the active one's place among them.
    fn queue(&self) -> (usize, usize) {
        let pending: Vec<&str> = self
            .chat
            .pending_prompts()
            .into_iter()
            .filter(|r| !self.prompt.dismissed.iter().any(|d| d == r))
            .collect();
        let at = self
            .active_ref()
            .and_then(|a| pending.iter().position(|r| *r == a));
        (at.map_or(1, |i| i + 1), pending.len().max(1))
    }

    /// A prompt takes the editor's place once the user has paused typing (at once when they
    /// weren't typing), and gives it back, draft intact, when it is answered or dismissed.
    fn update_engaged(&mut self) {
        let quiet = self.now_ms - self.prompt.typed_ms >= TYPING_PAUSE_MS;
        let engaged = self.active_ref().is_some() && (self.prompt.engaged || quiet);
        if engaged != self.prompt.engaged {
            self.prompt.engaged = engaged;
            self.dirty = true;
        }
    }

    /// A prompt is open but the user is typing: the editor stays, with a hint above it.
    fn prompt_waits(&self) -> bool {
        !self.prompt.engaged && self.active_ref().is_some()
    }

    fn active_buttons(&self) -> &[ButtonRef] {
        self.active_ref().map_or(&[], |r| self.chat.buttons_of(r))
    }

    /// The active prompt is a choice long enough to be filtered by typing.
    fn filtering(&self) -> bool {
        choice::is_long(self.active_buttons())
    }

    /// The options of the active prompt that match the filter (all of them for a card that
    /// isn't filtered).
    fn shown_buttons(&self) -> Vec<usize> {
        let buttons = self.active_buttons();
        if choice::is_long(buttons) {
            choice::matching(buttons, &self.prompt.filter)
        } else {
            (0..buttons.len()).collect()
        }
    }

    /// The filter changed: the focus goes to the first match (the current option when it is
    /// empty) and the window of the list starts over.
    fn refilter(&mut self) {
        if let Some(at) = choice::refocus(self.active_buttons(), &self.prompt.filter) {
            self.prompt.button = at;
        }
        self.prompt.panel_top.set(None);
    }

    /// A typed character, Backspace or Ctrl-U on a filtered choice.
    fn edit_filter(&mut self, edit: impl FnOnce(&mut String)) {
        edit(&mut self.prompt.filter);
        self.refilter();
    }

    /// A new active prompt starts on its current or first button and is scrolled into view.
    fn sync_prompt(&mut self) {
        let current = self.active_ref().map(str::to_string);
        // The card was answered elsewhere, or another one took over: the note is moot.
        if self
            .note
            .as_ref()
            .is_some_and(|n| Some(&n.r#ref) != current.as_ref())
        {
            self.note = None;
        }
        if current != self.prompt.seen {
            if let Some(old) = self.prompt.seen.clone() {
                self.forms.blur(&old, self.now_ms);
            }
            // A picker starts on the value in effect now, anything else on its first button.
            self.prompt.button = current
                .as_deref()
                .and_then(|r| self.chat.buttons_of(r).iter().position(|b| b.current))
                .unwrap_or(0);
            if current.is_some() && self.prompt.selected.is_none() {
                self.chat.follow();
            }
            self.prompt.panel_top.set(None);
            self.prompt.filter.clear();
            self.prompt.seen = current.clone();
            if let Some(r) = current.filter(|r| self.chat.has_targets(r)) {
                let targets = self.form_targets(&r);
                self.forms.enter(&r, &targets, self.now_ms);
                self.chat.reveal_focus(&r);
            }
        }
        self.update_engaged();
    }

    fn move_button(&mut self, delta: isize) {
        if self.filtering() {
            let shown = self.shown_buttons();
            self.prompt.button = choice::step(&shown, self.prompt.button, delta);
            return;
        }
        let n = self.active_buttons().len();
        if n > 0 {
            self.prompt.button =
                (self.prompt.button as isize + delta).rem_euclid(n as isize) as usize;
        }
    }

    fn press(&mut self, index: usize, fx: &mut Vec<Effect>) {
        self.press_with(index, None, fx);
    }

    /// Presses a button; `note` (what the user added for the agent) goes along in its args.
    fn press_with(&mut self, index: usize, note: Option<&str>, fx: &mut Vec<Effect>) {
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
            let mut args = button.args;
            if let (Some(note), Some(serde_json::Value::Object(map))) = (note, args.as_mut()) {
                map.insert("note".into(), note.into());
            }
            fx.push(Effect::Action {
                session: self.session.clone(),
                item: r.clone(),
                action: button.action,
                args,
            });
        }
        self.prompt.dismissed.push(r);
        self.prompt.selected = None;
        self.sync_prompt();
    }

    /// Tab on a card: a one-line note for the agent under it, sent with the focused button.
    /// Cards that aren't the daemon's questions keep Tab as "next button".
    fn tab_on_prompt(&mut self) {
        let Some(r) = self.active_ref().map(str::to_string) else {
            return;
        };
        let button = self.prompt.button;
        if self.filtering() && !self.shown_buttons().contains(&button) {
            return;
        }
        if self.chat.buttons_of(&r).get(button).is_some_and(takes_note) {
            self.note = Some(Note {
                r#ref: r,
                button,
                text: String::new(),
            });
            self.chat.follow();
        } else {
            self.move_button(1);
        }
    }

    /// Keys while a note is being typed: text, Enter answers with it, Esc goes back to the
    /// buttons.
    fn note_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(note) = self.note.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.note = None,
            KeyCode::Char('c') if ctrl => self.note = None,
            KeyCode::Char('u') if ctrl => note.text.clear(),
            KeyCode::Char('w') if ctrl => {
                let kept = note.text.trim_end().rfind(' ').map_or(0, |i| i + 1);
                note.text.truncate(kept);
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                note.text.push(c);
            }
            KeyCode::Backspace => {
                note.text.pop();
            }
            KeyCode::Enter => {
                let Some(Note { button, text, .. }) = self.note.take() else {
                    return;
                };
                let text = text.trim();
                self.press_with(button, (!text.is_empty()).then_some(text), fx);
            }
            _ => {}
        }
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
                self.prompt.dismissed.retain(|d| *d != r);
                self.prompt.selected = Some(r.clone());
                // Asked for by hand: no waiting for a pause in typing.
                self.prompt.typed_ms = 0;
                self.chat.reveal(&r);
            }
            None => {
                self.prompt.selected = None;
                self.prompt.dismissed.clear();
            }
        }
        self.sync_prompt();
        true
    }

    // ---- OpenUI forms ----

    /// The active prompt when it is an OpenUI item: keys move through its targets.
    fn active_form(&self) -> Option<&str> {
        self.active_ref().filter(|r| self.chat.has_targets(r))
    }

    /// The focus targets of an item's OpenUI docs, as drawn with their visual state.
    fn form_targets(&self, r: &str) -> Vec<UiTarget> {
        let Some(item) = self.chat.item(r) else {
            return Vec::new();
        };
        let opts = Options {
            openui: self.forms.doc_states(r),
            ..Options::default()
        };
        openui_targets(item, &opts)
    }

    /// The daemon sent items again: values it has caught up with are not held here any more.
    fn refresh_forms(&mut self) {
        let refs: Vec<String> = self.forms.all().keys().cloned().collect();
        for r in refs {
            if self.chat.has_targets(&r) {
                let targets = self.form_targets(&r);
                self.forms.refresh(&r, &targets, self.now_ms);
            }
        }
    }

    fn input_effect(&self, p: Pending) -> Effect {
        Effect::Action {
            session: self.session.clone(),
            item: p.item,
            action: "openui.input".into(),
            args: Some(serde_json::json!({"doc": p.doc, "key": p.key, "value": p.value})),
        }
    }

    /// Sends the values whose pause is over.
    fn send_due(&mut self, fx: &mut Vec<Effect>) {
        if !self.forms.has_pending() {
            return;
        }
        for p in self.forms.due(self.now_ms) {
            fx.push(self.input_effect(p));
        }
    }

    /// Keys while an OpenUI item has the editor's place.
    fn form_key(&mut self, r: &str, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if let KeyCode::Char(c) = key.code
            && ctrl
            && matches!(c, 'c' | 'p' | 'n' | 'o' | 't' | 'l' | 'g' | 'x')
        {
            match c {
                'c' => self.ctrl_c(fx),
                'p' => {
                    self.select_prompt(-1);
                }
                'n' => {
                    self.select_prompt(1);
                }
                _ => {
                    self.ctrl_key(c, fx);
                }
            }
            return;
        }
        match key.code {
            KeyCode::Up if shift => return self.chat.scroll_up(1),
            KeyCode::Down if shift => return self.chat.scroll_down(1),
            _ => {}
        }
        let targets = self.form_targets(r);
        let out = self.forms.key(r, &targets, key, self.now_ms);
        if !out.taken {
            match key.code {
                KeyCode::PageUp => self.chat.page_up(),
                KeyCode::PageDown => self.chat.page_down(),
                KeyCode::Home => self.chat.to_top(),
                KeyCode::End => self.chat.follow(),
                _ => {}
            }
            return;
        }
        self.chat.reveal_focus(r);
        self.apply_outcome(r, out, fx);
        self.send_due(fx);
    }

    fn apply_outcome(&mut self, r: &str, out: Outcome, fx: &mut Vec<Effect>) {
        if let Some(url) = out.url {
            self.open_url(url, fx);
        }
        if let Some(text) = out.copy {
            fx.push(Effect::Copy(text));
            self.notify("copied", false);
        }
        if let Some((doc, key)) = out.press {
            // What was typed goes first, so the press finds it.
            for p in self.forms.flush(r) {
                fx.push(self.input_effect(p));
            }
            fx.push(Effect::Action {
                session: self.session.clone(),
                item: r.to_string(),
                action: "openui.press".into(),
                args: Some(serde_json::json!({"doc": doc, "key": key})),
            });
        }
        if out.leave {
            self.esc(fx);
        }
    }

    /// A link in the browser, for the schemes that make sense there; anything else is shown.
    fn open_url(&mut self, url: String, fx: &mut Vec<Effect>) {
        let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
        if matches!(scheme.as_str(), "http" | "https" | "mailto") {
            self.notify(format!("opening {url}"), false);
            fx.push(Effect::OpenUrl(url));
        } else {
            self.notify(format!("link: {url}"), false);
        }
    }

    /// One line under a form: what the focused target does with the keys.
    fn form_hint(&self, r: &str, width: u16) -> Line<'static> {
        let targets = self.form_targets(r);
        let focused = self.forms.focused(r, &targets);
        let mut parts: Vec<String> = Vec::new();
        match focused {
            Some(t) => {
                if !t.label.is_empty() {
                    parts.push(t.label.clone());
                }
                parts.push(
                    match &t.kind {
                        UiKind::Input { .. } | UiKind::Date => "type · enter submits",
                        UiKind::TextArea => "type · enter newline",
                        UiKind::Select { .. } if self.forms.is_open(r, t) => {
                            "↑/↓ choose · enter picks"
                        }
                        UiKind::Select { .. } => "enter opens · ←/→ choose",
                        UiKind::Radio { .. } => "←/→ choose",
                        UiKind::CheckBox | UiKind::Switch => "space toggles",
                        UiKind::CheckBoxGroup { .. } | UiKind::SwitchGroup { .. } => {
                            "↑/↓ move · space toggles"
                        }
                        UiKind::Slider { .. } => "←/→ adjust",
                        UiKind::Button { .. } => "enter presses",
                        UiKind::Tabs { .. } => "←/→ switch tab",
                        UiKind::Accordion => "enter opens / closes",
                        UiKind::Carousel { .. } => "←/→ scroll",
                    }
                    .to_string(),
                );
                if !self.forms.is_open(r, t) {
                    parts.push("tab next".to_string());
                }
            }
            None => parts.push("tab to start".to_string()),
        }
        parts.push(
            if focused.is_some_and(|t| self.forms.is_open(r, t)) {
                "esc closes"
            } else {
                "esc input"
            }
            .to_string(),
        );
        let text = parts.join(" · ");
        let w = usize::from(width);
        chat::barred(
            Line::from(Span::styled(
                truncate(&text, w.saturating_sub(2)),
                self.styles.muted,
            )),
            self.styles.accent,
        )
    }

    // ---- keys ----

    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut fx = Vec::new();
        if key.kind == KeyEventKind::Release {
            return fx;
        }
        self.dirty = true;
        if self.secret.is_some() {
            self.secret_key(key, &mut fx);
            return fx;
        }
        if self.widgets.overlay_open() {
            self.widgets.handle_key(Panel::Files, key);
            if let Some((path, line)) = self.widgets.take_edit() {
                fx.push(Effect::Edit { path, line });
            }
            return fx;
        }
        if self.github.is_open() {
            if let Some(out) = self.github.key(key) {
                self.take_github(out, &mut fx);
            }
            return fx;
        }
        if self.transcript.is_some() {
            self.transcript_key(key, &mut fx);
            return fx;
        }
        // Ctrl-] / Ctrl-[ widen / narrow the focused pane (legacy terminals send Ctrl-] as
        // Ctrl-5 and Ctrl-[ as Esc).
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && let KeyCode::Char(c @ (']' | '5' | '[')) = key.code
        {
            self.resize_pane(if c == '[' { -RESIZE_STEP } else { RESIZE_STEP });
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
            Overlay::None if self.note.is_some() => self.note_key(key, &mut fx),
            Overlay::None => match self.focus {
                Some(focus) => self.panel_key(focus, key, &mut fx),
                None => self.editor_key(key, &mut fx),
            },
        }
        self.sync_prompt();
        fx
    }

    /// The mouse wheel scrolls the chat (or the open diff, or the terminal panel under the
    /// pointer). Other mouse events are ignored.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let down = match mouse.kind {
            MouseEventKind::ScrollUp => false,
            MouseEventKind::ScrollDown => true,
            _ => return,
        };
        self.dirty = true;
        if self.github.is_open() {
            self.github.wheel(down);
        } else if let Some(t) = self.transcript.as_mut() {
            t.wheel(down);
        } else if self
            .terminal_area
            .is_some_and(|a| a.contains(ratatui::layout::Position::new(mouse.column, mouse.row)))
        {
            let code = if down { KeyCode::Down } else { KeyCode::Up };
            for _ in 0..WHEEL_LINES {
                self.widgets
                    .handle_key(Panel::Terminal, KeyEvent::new(code, KeyModifiers::NONE));
            }
        } else if self.widgets.overlay_open() {
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

    pub fn handle_paste(&mut self, text: &str) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.paste(text, &mut fx);
        fx
    }

    fn paste(&mut self, text: &str, fx: &mut Vec<Effect>) {
        self.dirty = true;
        if let Some(secret) = self.secret.as_mut() {
            secret.paste(text);
            return;
        }
        if self.github.is_open() {
            self.github.paste(text);
            return;
        }
        match self.overlay {
            Overlay::Switcher => text
                .chars()
                .filter(|c| !c.is_control())
                .for_each(|c| self.switcher.type_char(c)),
            Overlay::Help => self.overlay = Overlay::None,
            Overlay::None if self.note.is_some() => {
                if let Some(note) = self.note.as_mut() {
                    note.text.extend(text.chars().filter(|c| !c.is_control()));
                }
            }
            Overlay::None if self.prompt.engaged && self.active_form().is_some() => {
                if let Some(r) = self.active_form().map(str::to_string) {
                    let targets = self.form_targets(&r);
                    self.forms.paste(&r, &targets, text, self.now_ms);
                    self.chat.reveal_focus(&r);
                }
            }
            Overlay::None => {
                if self.focus.is_none() && !self.widgets.overlay_open() && !self.prompt.engaged {
                    self.prompt.typed_ms = self.now_ms;
                    // Files dropped on the terminal arrive as their paths. An empty paste is what
                    // some terminals send when the clipboard holds only an image.
                    if text.is_empty() {
                        return fx.push(Effect::PasteImage);
                    }
                    if let Some(paths) = attach::image_paths(text, &self.cwd) {
                        return self.attach(paths);
                    }
                    self.editor.insert_str(text);
                    self.histories.of(&self.session).reset();
                }
            }
        }
    }

    /// Up with messages waiting: asks for this app's back, to edit them (`redraft`).
    fn unqueue(&mut self, fx: &mut Vec<Effect>) -> bool {
        if !self.waiting.iter().any(|w| w.app == self.app_id) {
            return false;
        }
        fx.push(Effect::Unqueue {
            session: self.session.clone(),
        });
        true
    }

    /// The messages taken back out of the queue go into the editor, one per paragraph, before
    /// whatever was typed meanwhile; their images come along.
    fn redraft(&mut self, inputs: Vec<WaitingInput>) {
        let app = self.app_id.clone();
        self.waiting.retain(|w| w.app != app);
        if inputs.is_empty() {
            return;
        }
        self.dirty = true;
        let mut parts = Vec::new();
        for input in inputs {
            let next = self.attachments.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
            let images = input.images.unwrap_or_default();
            let (text, attached) = attach::redraft(&input.text, &images, next);
            self.attachments.extend(attached);
            parts.push(text);
        }
        let typed = self.editor.text().trim().to_string();
        if !typed.is_empty() {
            parts.push(typed);
        }
        self.editor.set_text(&parts.join("\n\n"));
        self.histories.of(&self.session).reset();
    }

    /// Pastes images into the draft: each gets a number and its `[image N]` marker goes in at
    /// the cursor, so the message says where it belongs.
    fn attach(&mut self, paths: Vec<String>) {
        self.dirty = true;
        self.prompt.typed_ms = self.now_ms;
        for path in paths {
            let n = self.attachments.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
            self.attachments.push((n, path));
            let before = self.editor.text()[..self.editor.cursor()]
                .chars()
                .next_back();
            if before.is_some_and(|c| !c.is_whitespace()) {
                self.editor.insert_char(' ');
            }
            self.editor.insert_str(&attach::marker(n));
            self.editor.insert_char(' ');
        }
        self.histories.of(&self.session).reset();
    }

    fn panel_key(&mut self, focus: Focus, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => self.cycle_focus(),
            KeyCode::BackTab => self.cycle_approvals(fx),
            KeyCode::Char('c') if ctrl => self.ctrl_c(fx),
            KeyCode::Char('g') if ctrl => self.toggle_sessions_focus(),
            _ => match focus {
                Focus::Panel(panel) => match self.widgets.handle_key(panel, key) {
                    WidgetAction::Redraw => {}
                    WidgetAction::Leave => self.focus = None,
                    // A new todo is written in the editor, as /todo.
                    WidgetAction::AddTodo => {
                        self.focus = None;
                        self.editor.set_text("/todo ");
                    }
                    WidgetAction::Todo(args) => self.change_todos(args, fx),
                },
                Focus::Sessions => match self.sessions.key(key) {
                    SessionsAction::Redraw => {}
                    SessionsAction::Leave => self.focus = None,
                    SessionsAction::Open(info) => {
                        self.focus = None;
                        self.open_session(*info, fx);
                    }
                    SessionsAction::New => {
                        self.focus = None;
                        fx.push(Effect::CreateSession {
                            cwd: self.cwd.clone(),
                        });
                    }
                },
                Focus::Agents => self.agents_key(key, fx),
                Focus::Jobs => self.jobs_key(key),
                Focus::Chat => self.chat_files_key(key),
            },
        }
    }

    /// Keys while the chat's file lines have focus: move between them, open one's diff.
    fn chat_files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.focus = None,
            KeyCode::Up | KeyCode::Char('k') => self.chat.move_file(-1),
            KeyCode::Down | KeyCode::Char('j') => self.chat.move_file(1),
            KeyCode::Home | KeyCode::Char('g') => self.chat.pick_file(0),
            KeyCode::End | KeyCode::Char('G') => self.chat.pick_file(-1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(path) = self.chat.picked_file().map(str::to_string) {
                    self.widgets.open_path(&path);
                }
            }
            _ => {}
        }
    }

    // ---- subagents ----

    /// Ctrl-X, or Down from an empty editor: the keys go to the agents list.
    fn focus_agents(&mut self) {
        if self.agents.visible(self.now_ms) {
            self.agents.focus(self.now_ms);
            self.focus = Some(Focus::Agents);
        } else {
            self.notify("no subagents running", false);
        }
    }

    // ---- background commands ----

    /// Down past the editor (and the subagents): the keys go to the background commands, and
    /// the terminal panel shows the selected one.
    fn focus_jobs(&mut self) {
        let rows = self.widgets.jobs(self.now_ms);
        let id = self.jobs.focus(&rows);
        if id.is_some() {
            self.terminal_on = true;
            self.widgets.show_job(id);
            self.focus = Some(Focus::Jobs);
        }
    }

    fn jobs_key(&mut self, key: KeyEvent) {
        let rows = self.widgets.jobs(self.now_ms);
        match self.jobs.key(key, &rows) {
            JobsAction::Redraw => {}
            JobsAction::Show(id) => self.widgets.show_job(Some(id)),
            JobsAction::Up if self.agents.visible(self.now_ms) => {
                self.widgets.show_job(None);
                self.agents.focus(self.now_ms);
                self.focus = Some(Focus::Agents);
            }
            JobsAction::Up | JobsAction::Leave => {
                self.widgets.show_job(None);
                self.focus = None;
            }
            // It ends (and leaves the list) once its file says so.
            JobsAction::Stop(id) => match self.widgets.stop_job(&id) {
                Ok(_) => self.notify("stopping…", false),
                Err(why) => self.notify(format!("can't stop it: {why}"), true),
            },
            JobsAction::Open(id) => {
                self.widgets.show_job(Some(id));
                if self.terminal_area.is_some() {
                    self.focus = Some(Focus::Panel(Panel::Terminal));
                } else {
                    self.notify("too narrow for the terminal panel", false);
                }
            }
        }
    }

    fn agents_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        // Down from the last subagent goes on to the background commands.
        if matches!(key.code, KeyCode::Down | KeyCode::Char('j'))
            && self.agents.on_last(self.now_ms)
            && !self.widgets.jobs(self.now_ms).is_empty()
        {
            self.focus_jobs();
            return;
        }
        match self.agents.key(key, self.now_ms) {
            AgentsAction::Redraw => {}
            AgentsAction::Leave => self.focus = None,
            AgentsAction::Transcript(id) => {
                let Some(agent) = self.agents.get(&id).cloned() else {
                    return;
                };
                self.focus = None;
                fx.push(Effect::AgentView {
                    session: self.session.clone(),
                    agent: agent.harness.clone(),
                    on: true,
                });
                self.transcript = Some(Transcript::new(agent));
            }
            AgentsAction::Session(child) => {
                self.focus = None;
                match self.sessions.get(&child).cloned() {
                    Some(info) => self.open_session(info, fx),
                    None => self.notify("that session isn't known yet", false),
                }
            }
            AgentsAction::Stop(id) => {
                self.notify("stopping…", false);
                fx.push(Effect::Action {
                    session: self.session.clone(),
                    item: "agents".into(),
                    action: "neo.cancel_agent".into(),
                    args: Some(serde_json::json!({ "id": id })),
                });
            }
        }
    }

    /// Keys while a subagent's transcript is open: scrolling, expanding, and the way out.
    fn transcript_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter if !ctrl => {
                if let Some(t) = self.transcript.take() {
                    fx.push(Effect::AgentView {
                        session: self.session.clone(),
                        agent: t.agent.harness,
                        on: false,
                    });
                }
            }
            KeyCode::Char('c') if ctrl => self.ctrl_c(fx),
            KeyCode::Char('o') if ctrl => self.expanded = !self.expanded,
            KeyCode::Char('t') if ctrl => self.show_thoughts = !self.show_thoughts,
            _ if !ctrl => {
                if let Some(t) = self.transcript.as_mut() {
                    t.scroll_key(key);
                }
            }
            _ => {}
        }
    }

    /// Tab: editor, the chat's files, sessions, terminal, plan, files (the ones on screen),
    /// editor.
    fn cycle_focus(&mut self) {
        let order = [
            (self.chat.has_files(), Focus::Chat),
            (self.sessions_shown, Focus::Sessions),
            // The side column top to bottom.
            (
                self.sidebar_shown && self.widgets.has_todos(),
                Focus::Panel(Panel::Todos),
            ),
            (self.sidebar_shown, Focus::Panel(Panel::Plan)),
            (self.sidebar_shown, Focus::Panel(Panel::Files)),
            (self.terminal_area.is_some(), Focus::Panel(Panel::Terminal)),
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
        match self.focus {
            Some(Focus::Sessions) => self.sessions.focus(&self.session),
            // The latest file, unless one is picked already.
            Some(Focus::Chat) if self.chat.picked_file().is_none() => self.chat.pick_file(-1),
            Some(Focus::Chat) => self.chat.move_file(0),
            _ => {}
        }
    }

    /// `/files`: the files panel gets the keys.
    fn focus_files(&mut self) {
        if self.sidebar_shown {
            self.focus = Some(Focus::Panel(Panel::Files));
        } else if self.widgets.wants_sidebar(u16::MAX) {
            self.notify("terminal too narrow for the files panel", false);
        } else {
            self.notify("no changed files", false);
        }
    }

    /// `/terminal`: show or hide the agent's commands next to the chat.
    fn toggle_terminal(&mut self) {
        self.terminal_on = !self.terminal_on;
        if self.terminal_on && !self.widgets.has_commands() {
            self.notify(
                "no commands yet; they show here once the agent runs one",
                false,
            );
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
            KeyCode::Char('c') if ctrl => self.overlay = Overlay::None,
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
        self.cancel_secret(fx);
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

    /// Shift-Tab: ask, review, edits, auto, ask…
    fn cycle_approvals(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Input {
            session: self.session.clone(),
            text: format!("/approvals {}", self.approvals.next().word()),
            images: Vec::new(),
        });
    }

    fn ctrl_c(&mut self, fx: &mut Vec<Effect>) {
        if (!self.editor.is_empty() || !self.attachments.is_empty()) && !self.prompt.engaged {
            self.editor.clear();
            self.attachments.clear();
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
        if self.prompt.engaged && self.filtering() && !self.prompt.filter.is_empty() {
            self.edit_filter(String::clear);
        } else if self.menu_visible() {
            self.menu_closed_for = Some(self.editor.text().to_string());
        } else if let Some(r) = self.active_ref().map(str::to_string) {
            self.prompt.dismissed.push(r);
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

    /// Keys while a prompt has taken the editor's place: they answer it, and nothing else
    /// reaches the editor (and its draft).
    fn prompt_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        if let Some(r) = self.active_form().map(str::to_string) {
            return self.form_key(&r, key, fx);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char(c) if ctrl => match c {
                'c' => self.ctrl_c(fx),
                'u' if self.filtering() => self.edit_filter(String::clear),
                'p' => {
                    self.select_prompt(-1);
                }
                'n' => {
                    self.select_prompt(1);
                }
                'o' | 't' | 's' | 'l' | 'g' | 'x' => {
                    self.ctrl_key(c, fx);
                }
                _ => {}
            },
            // On a long choice every character is for the filter, digits too.
            KeyCode::Char(c) if self.filtering() => {
                if !key.modifiers.contains(KeyModifiers::ALT) {
                    self.edit_filter(|f| f.push(c));
                }
            }
            KeyCode::Backspace if self.filtering() => {
                self.edit_filter(|f| {
                    f.pop();
                });
            }
            KeyCode::Char(c) => {
                if let Some(n) = c.to_digit(10).filter(|n| (1..=9).contains(n))
                    && (n as usize) <= self.active_buttons().len()
                {
                    self.press(n as usize - 1, fx);
                }
            }
            KeyCode::Enter => {
                // With nothing matching there is nothing to pick.
                if !self.filtering() || self.shown_buttons().contains(&self.prompt.button) {
                    self.press(self.prompt.button, fx);
                }
            }
            KeyCode::Tab => self.tab_on_prompt(),
            KeyCode::BackTab => self.cycle_approvals(fx),
            KeyCode::Esc => self.esc(fx),
            KeyCode::Up if shift => self.chat.scroll_up(1),
            KeyCode::Down if shift => self.chat.scroll_down(1),
            KeyCode::Left | KeyCode::Up => self.move_button(-1),
            KeyCode::Right | KeyCode::Down => self.move_button(1),
            KeyCode::PageUp => self.chat.page_up(),
            KeyCode::PageDown => self.chat.page_down(),
            KeyCode::Home => self.chat.to_top(),
            KeyCode::End => self.chat.follow(),
            _ => {}
        }
    }

    fn editor_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        if self.prompt.engaged {
            return self.prompt_key(key, fx);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let empty = self.editor.is_empty();
        // The `@` file list takes the arrows, Tab, Enter and Esc while it is open.
        if !ctrl && !alt && !shift && self.mention_visible() {
            match key.code {
                KeyCode::Up => return self.move_mention(-1),
                KeyCode::Down => return self.move_mention(1),
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_mention();
                    self.prompt.typed_ms = self.now_ms;
                    return;
                }
                KeyCode::Esc => {
                    self.mention_closed_for = Some(self.editor.text().to_string());
                    return;
                }
                _ => {}
            }
        }
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
                if c == '?' && empty {
                    self.overlay = Overlay::Help;
                    return;
                }
                self.editor.insert_char(c);
            }
            KeyCode::Enter if alt || shift => self.editor.newline(),
            // Ctrl-Enter: now, not after the running turn (which is cancelled for it).
            KeyCode::Enter if ctrl => {
                if self.running && !self.editor.is_empty() && !self.menu_visible() {
                    fx.push(Effect::Cancel {
                        session: self.session.clone(),
                    });
                }
                if self.enter(fx) {
                    self.histories.of(&self.session).reset();
                    edited = false;
                }
            }
            KeyCode::Enter => {
                if self.enter(fx) {
                    // Sent: the typing is over (see send), not just paused.
                    self.histories.of(&self.session).reset();
                    edited = false;
                }
            }
            KeyCode::BackTab => {
                self.cycle_approvals(fx);
                edited = false;
            }
            KeyCode::Tab => {
                edited = false;
                if self.menu_visible() {
                    self.complete_menu();
                } else {
                    self.cycle_focus();
                }
            }
            KeyCode::Esc => {
                self.esc(fx);
                edited = false;
            }
            KeyCode::Backspace if alt => self.editor.delete_word_back(),
            // A pasted image's marker goes as a whole (and the image with it).
            KeyCode::Backspace => {
                match attach::marker_before(self.editor.text(), self.editor.cursor()) {
                    Some(len) => self.editor.delete_before(len),
                    None => self.editor.backspace(),
                }
            }
            KeyCode::Delete => self.editor.delete(),
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
                } else if !self.editor.up()
                    && !(empty && !self.histories.of(&self.session).browsing() && self.unqueue(fx))
                {
                    self.history_prev();
                }
            }
            KeyCode::Down => {
                edited = false;
                if self.menu_visible() {
                    self.move_menu(1);
                } else if !self.editor.down() {
                    // Past the end of an empty draft, Down goes on to the subagents list, then
                    // the background commands.
                    let below = empty && !self.histories.of(&self.session).browsing();
                    if below && self.agents.visible(self.now_ms) {
                        self.focus_agents();
                    } else if below && !self.widgets.jobs(self.now_ms).is_empty() {
                        self.focus_jobs();
                    } else {
                        self.history_next();
                    }
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
            self.prompt.typed_ms = self.now_ms;
            self.mention_sel = 0;
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
            'v' => {
                fx.push(Effect::PasteImage);
                return false;
            }
            's' => self.swap_stash(),
            'r' => {
                self.github.open();
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
            'x' => {
                self.focus_agents();
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
            self.recall(&text);
        }
    }

    fn history_next(&mut self) {
        if let Some(text) = self.histories.of(&self.session).newer() {
            self.recall(&text);
        }
    }

    /// A recalled `/command` keeps the menu closed, or the next Up/Down would move in it
    /// instead of going on through the history.
    fn recall(&mut self, text: &str) {
        self.editor.set_text(text);
        self.menu_closed_for = Some(text.to_string());
    }

    // ---- sending and commands ----

    /// The slash menu's commands: ours, and those of the harness the session runs on.
    fn commands(&self) -> Vec<Item> {
        let listed = self
            .offered
            .get(&self.harness)
            .map_or(&[][..], Vec::as_slice);
        menu::items_for(&self.harness, listed)
    }

    fn menu_matches(&self) -> Vec<Item> {
        let text = self.editor.text();
        if self.focus.is_some()
            || self.prompt.engaged
            || self.overlay != Overlay::None
            || !menu::is_slash_word(text)
            || self.menu_closed_for.as_deref() == Some(text)
        {
            return Vec::new();
        }
        menu::matching(text, &self.commands())
    }

    /// The files the `@word` at the cursor could be (none while anything else has the keys).
    fn mention_matches(&self) -> Vec<Candidate> {
        let text = self.editor.text();
        if self.focus.is_some()
            || self.prompt.engaged
            || self.overlay != Overlay::None
            || self.mention_closed_for.as_deref() == Some(text)
        {
            return Vec::new();
        }
        let Some(m) = mention::at_cursor(text, self.editor.cursor()) else {
            return Vec::new();
        };
        self.mentions
            .candidates(std::path::Path::new(&self.cwd), &m.query)
    }

    fn mention_visible(&self) -> bool {
        !self.mention_matches().is_empty()
    }

    fn move_mention(&mut self, delta: isize) {
        let n = self.mention_matches().len() as isize;
        if n > 0 {
            self.mention_sel = (self.mention_sel as isize + delta).rem_euclid(n) as usize;
        }
    }

    /// Puts the selected file in the `@word`'s place: a folder stays open to go on in, a file
    /// ends the word with a space.
    fn accept_mention(&mut self) {
        let matches = self.mention_matches();
        let Some(pick) = matches.get(self.mention_sel.min(matches.len().saturating_sub(1))) else {
            return;
        };
        let text = self.editor.text().to_string();
        let cursor = self.editor.cursor();
        let Some(m) = mention::at_cursor(&text, cursor) else {
            return;
        };
        let word = format!("@{}{}", pick.path, if pick.dir { "" } else { " " });
        let after = &text[cursor..];
        let new = format!("{}{word}{after}", &text[..m.start]);
        self.editor.set_text(&new);
        // The cursor after the word: set_text leaves it at the end.
        for _ in 0..after.chars().count() {
            self.editor.left();
        }
        self.mention_sel = 0;
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

    /// Enter in the editor; true when the text was sent or run, false when there's more to type.
    fn enter(&mut self, fx: &mut Vec<Effect>) -> bool {
        let matches = self.menu_matches();
        if !matches.is_empty() {
            let cmd = &matches[self.menu_sel.min(matches.len() - 1)];
            if cmd.needs_args {
                self.editor.set_text(&format!("/{} ", cmd.name));
                self.menu_sel = 0;
                return false;
            }
            self.editor.clear();
            self.menu_sel = 0;
            self.run_command(&cmd.name, &format!("/{}", cmd.name), fx);
            return true;
        }
        let text = self.editor.text().trim().to_string();
        if text.is_empty() {
            return false;
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
        true
    }

    fn run_command(&mut self, name: &str, text: &str, fx: &mut Vec<Effect>) {
        match name {
            "sessions" => self.open_switcher(fx),
            "sidebar" => self.toggle_sessions(),
            "expand" => self.expanded = !self.expanded,
            "thoughts" => self.show_thoughts = !self.show_thoughts,
            "help" => self.overlay = Overlay::Help,
            "github" => self.github.open(),
            "theme" => self.switch_theme(text),
            "todo" => self.add_todo(text, fx),
            "files" => self.focus_files(),
            "terminal" => self.toggle_terminal(),
            "quit" => fx.push(Effect::Quit),
            _ => self.send(text.to_string(), fx),
        }
    }

    /// What the GitHub view hands over: text for the editor, or a message to send right away.
    fn take_github(&mut self, out: Outgoing, fx: &mut Vec<Effect>) {
        match out {
            Outgoing::Attach(text) => {
                // After what is already typed, with room to add instructions below.
                self.editor.text_end();
                if !self.editor.is_empty() {
                    self.editor.insert_str("\n\n");
                }
                self.editor.insert_str(&text);
                self.editor.insert_str("\n\n");
                self.histories.of(&self.session).reset();
            }
            Outgoing::Send(text) => self.send(text, fx),
        }
    }

    fn send(&mut self, text: String, fx: &mut Vec<Effect>) {
        self.histories.record(&self.session, &text);
        self.chat.follow();
        // Sending ends the typing: a card it brings (/model's choice) takes the editor's place at
        // once instead of showing in the chat first.
        self.prompt.typed_ms = 0;
        let (text, images) = attach::outgoing(&text, &self.attachments);
        self.attachments.retain(|(_, path)| !images.contains(path));
        if self.running {
            self.waiting.push(WaitingInput {
                app: self.app_id.clone(),
                text: text.clone(),
                images: (!images.is_empty()).then(|| images.clone()),
            });
        }
        fx.push(Effect::Input {
            session: self.session.clone(),
            text,
            images,
        });
        // What was written in its place is out: the draft put aside comes back.
        if self.editor.is_empty() && self.stash.is_some() {
            self.swap_stash();
        }
    }

    /// Ctrl-S: the draft and its images go aside and the editor is free for another message, or
    /// the one put aside comes back (trading places with what is in the editor now).
    fn swap_stash(&mut self) {
        let text = self.editor.text().to_string();
        let attachments = std::mem::take(&mut self.attachments);
        let draft = (!text.trim().is_empty()).then_some((text, attachments));
        match std::mem::replace(&mut self.stash, draft) {
            Some((text, attachments)) => {
                self.editor.set_text(&text);
                self.attachments = attachments;
            }
            None => self.editor.clear(),
        }
        self.menu_sel = 0;
        self.histories.of(&self.session).reset();
        self.dirty = true;
    }

    // ---- drawing ----

    fn areas(&mut self, area: Rect) -> Areas {
        // The side column: the todos, plan and files as wide as the files need, up to a third of
        // the terminal; with the agent's commands under them, two fifths, as the commands want
        // room.
        let terminal = self.terminal_on
            && self.widgets.has_commands()
            && area.width >= CHAT_MIN_WIDTH + 1 + TERMINAL_MIN_WIDTH;
        let panels_base = self
            .widgets
            .sidebar_width(SIDE_MIN_WIDTH, (area.width / 3).clamp(32, 40));
        let (side_base, side_min) = if terminal {
            let base = (area.width * 2 / 5).clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
            (base.max(panels_base), TERMINAL_MIN_WIDTH)
        } else {
            (panels_base, SIDE_MIN_WIDTH)
        };
        let side_w = resized(
            side_base,
            self.resized.side,
            side_min,
            area.width.saturating_sub(1 + CHAT_MIN_WIDTH),
        );
        let side = (area.width >= side_w + 1 + 40
            && (terminal || self.widgets.wants_sidebar(area.width)))
        .then_some(side_w);
        if side.is_some() {
            self.resized.side = side_w as i16 - side_base as i16;
        }
        let right_w = side.map_or(0, |w| w + 1);
        // The sessions sidebar yields to the plan / files column, and to the chat's width.
        let left_base = sidebar_width(area.width);
        let left_w = resized(
            left_base,
            self.resized.sessions,
            SIDEBAR_WIDTH,
            area.width.saturating_sub(right_w + CHAT_MIN_WIDTH),
        );
        if left_w >= SIDEBAR_WIDTH {
            self.resized.sessions = left_w as i16 - left_base as i16;
        }
        let left = (self.sessions_on && !self.sessions.is_empty() && left_w >= SIDEBAR_WIDTH)
            .then_some(Rect {
                width: left_w,
                ..area
            });
        let left_w = left.map_or(0, |r| r.width);
        let main = Rect {
            x: area.x + left_w,
            width: area.width - left_w - right_w,
            ..area
        };
        let terminal = terminal && side.is_some();
        let side = side.map(|w| Rect {
            x: area.x + area.width - w,
            y: area.y,
            width: w,
            height: area.height,
        });
        // Past the prompt (or the input box's bar), and its right margin with fills.
        let margin = u16::from(self.styles.fills());
        self.editor
            .set_width(usize::from(main.width.saturating_sub(2 + margin)).max(1));
        // The subagents list sits under the status lines.
        let agents_h = self.agents_rows(main.height);
        let jobs_h = self.jobs_rows(main.height - agents_h);
        let h = main.height - agents_h - jobs_h;
        let status_h = h.saturating_sub(3).min(2);
        let rule_h = h.saturating_sub(status_h).min(1);
        let room = h - status_h - rule_h;
        // An engaged prompt takes the editor's place; a waiting one adds a hint row above it.
        let box_w = main.width.saturating_sub(3 * margin);
        let panel = if let Some(secret) = &self.secret {
            secret.lines(usize::from(box_w), self.styles.accent, self.styles.muted)
        } else if self.prompt.engaged {
            self.panel_lines(box_w, usize::from(room.saturating_sub(1).min(PANEL_ROWS)))
        } else {
            Vec::new()
        };
        let hint_h = u16::from(self.prompt_waits() && room >= 3);
        let pasted = !attach::referenced(self.editor.text(), &self.attachments).is_empty();
        let attached_h = if !panel.is_empty() || !pasted {
            0
        } else if self.images.is_some() && room >= PREVIEW_ROWS + 1 + 4 {
            PREVIEW_ROWS + 1
        } else {
            u16::from(room >= 3)
        };
        let waiting_h = if panel.is_empty() && room >= 6 {
            self.waiting.len().min(WAITING_ROWS) as u16
        } else {
            0
        };
        let wanted = if panel.is_empty() {
            self.editor.height().clamp(1, EDITOR_ROWS) as u16 + hint_h + attached_h + waiting_h
        } else {
            panel.len() as u16
        };
        let editor_h = wanted.min(room.saturating_sub(1)).max(room.min(1));
        let chat_h = room - editor_h;
        let at = |y: u16, height: u16| Rect {
            y: main.y + y,
            height,
            ..main
        };
        let hint_h = hint_h.min(editor_h.saturating_sub(1));
        let waiting_h = waiting_h.min(editor_h.saturating_sub(1 + hint_h));
        let attached_h = attached_h.min(editor_h.saturating_sub(1 + hint_h + waiting_h));
        // Too little room for a thumbnail: none, rather than half of one.
        let attached_h = if attached_h > 1 && attached_h < PREVIEW_ROWS + 1 {
            1
        } else {
            attached_h
        };
        let above = hint_h + waiting_h + attached_h;
        let top = chat_h + rule_h;
        // With fills, the rule's row, the editor and the first status line are one raised box
        // with the accent bar on its left, a row of the chat's above it; what is in the box
        // starts past the bar.
        let gap = u16::from(self.styles.fills() && chat_h > 1);
        let inset = |r: Rect| {
            if self.styles.fills() {
                Rect {
                    x: r.x + 2,
                    width: r.width.saturating_sub(3),
                    ..r
                }
            } else {
                r
            }
        };
        Areas {
            chat: at(0, chat_h - gap),
            rule: at(chat_h, rule_h),
            hint: inset(at(top, hint_h)),
            waiting: inset(at(top + hint_h, waiting_h)),
            attached: inset(at(top + hint_h + waiting_h, attached_h)),
            editor: at(chat_h + rule_h + above, editor_h - above),
            panel,
            input: at(chat_h, rule_h + editor_h + status_h.min(1)),
            status: at(chat_h + rule_h + editor_h, status_h),
            agents: at(h, agents_h),
            jobs: at(h + agents_h, jobs_h),
            side,
            terminal,
            left,
        }
    }

    /// Rows the subagents list takes of a pane `total` rows tall: none when the pane is small.
    fn agents_rows(&self, total: u16) -> u16 {
        if total < 12 {
            return 0;
        }
        let want = self
            .agents
            .height(self.now_ms, self.focus == Some(Focus::Agents));
        u16::try_from(want).unwrap_or(u16::MAX).min(total / 3)
    }

    /// Rows the background commands take of what is left of a pane `total` rows tall.
    fn jobs_rows(&self, total: u16) -> u16 {
        if total < 12 {
            return 0;
        }
        let rows = self.widgets.jobs(self.now_ms);
        let want = Jobs::height(&rows, self.focus == Some(Focus::Jobs));
        u16::try_from(want).unwrap_or(u16::MAX).min(total / 4)
    }

    fn draw_jobs(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let rows = self.widgets.jobs(self.now_ms);
        let lines = self.jobs.lines(
            &rows,
            usize::from(area.width),
            self.now_ms,
            self.focus == Some(Focus::Jobs),
            &self.styles,
        );
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_agents(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let main = MainRow {
            harness: &self.harness,
            running: self.running,
        };
        let lines = self.agents.lines(
            usize::from(area.width),
            self.now_ms,
            &main,
            self.focus == Some(Focus::Agents),
            &self.styles,
        );
        frame.render_widget(Paragraph::new(lines), area);
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.size = (area.width, area.height);
        let areas = self.areas(area);
        self.sidebar_shown = areas.side.is_some() && self.widgets.has_panels();
        self.terminal_area = areas
            .side
            .and_then(|side| self.widgets.side_terminal(side, areas.terminal));
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
            Some(Focus::Panel(Panel::Terminal)) if !self.terminal_area.is_some() => {
                self.focus = None
            }
            Some(Focus::Panel(Panel::Todos | Panel::Plan | Panel::Files))
                if !self.sidebar_shown =>
            {
                self.focus = None;
            }
            // The last todo went: so did its panel.
            Some(Focus::Panel(Panel::Todos)) if !self.widgets.has_todos() => self.focus = None,
            Some(Focus::Sessions) if !self.sessions_shown => self.focus = None,
            Some(Focus::Agents) if !self.agents.visible(self.now_ms) => self.focus = None,
            Some(Focus::Jobs) if self.widgets.jobs(self.now_ms).is_empty() => {
                self.widgets.show_job(None);
                self.focus = None;
            }
            Some(Focus::Chat) if !self.chat.has_files() => self.focus = None,
            _ => {}
        }
        self.sync_prompt();
        self.chat.set_highlight(self.focus == Some(Focus::Chat));

        if self.github.is_open() {
            self.github
                .draw(frame, areas.chat, self.now_ms, &self.styles);
        } else if self.widgets.overlay_open() {
            self.widgets.draw_overlay(frame, areas.chat);
        } else if self.transcript.is_some() {
            let opts = ChatOpts {
                expanded: self.expanded,
                show_thoughts: self.show_thoughts,
                palette: self.palette,
                active: None,
                in_panel: false,
                marker: self.styles.accent,
                ui: Default::default(),
                icons: self.icons,
                // A subagent's work has no turns to sum up.
                running: true,
                split_diffs: self.split_diffs,
            };
            if let Some(t) = self.transcript.as_mut() {
                t.draw(frame, areas.chat, &opts, &self.styles);
            }
        } else {
            match self.overlay {
                Overlay::None => self.draw_chat(frame, areas.chat),
                Overlay::Help => self.draw_help(frame, areas.chat),
                Overlay::Switcher => self.draw_switcher(frame, areas.chat),
            }
            self.draw_menu(frame, areas.chat);
            self.draw_mentions(frame, areas.chat);
        }
        if let Some(left) = areas.left {
            self.draw_sessions(frame, left);
        }
        if let Some(side) = areas.side {
            let panel = match self.focus {
                Some(Focus::Panel(panel)) => Some(panel),
                _ => None,
            };
            self.widgets
                .draw_sidebar(frame, side, panel, areas.terminal);
        }
        let fills = self.styles.fills();
        if fills {
            crate::style::fill(frame.buffer_mut(), areas.input, self.styles.raised);
        } else if areas.rule.height > 0 {
            let rule = "─".repeat(usize::from(areas.rule.width));
            frame.render_widget(
                Paragraph::new(Span::styled(rule, self.styles.border)),
                areas.rule,
            );
        }
        if areas.hint.height > 0 {
            let hint = if self.active_form().is_some() {
                "form open — it takes the keys when you stop typing"
            } else {
                "permission needed — waiting until you stop typing"
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    truncate(hint, usize::from(areas.hint.width)),
                    self.styles.accent,
                )),
                areas.hint,
            );
        }
        if areas.waiting.height > 0 {
            self.draw_waiting(frame, areas.waiting);
        }
        if areas.attached.height > 0 {
            self.draw_attached(frame, areas.attached);
        }
        if areas.panel.is_empty() {
            self.draw_editor(frame, areas.editor);
        } else if fills {
            let at = Rect {
                x: areas.editor.x + 2,
                width: areas.editor.width.saturating_sub(3),
                ..areas.editor
            };
            frame.render_widget(Paragraph::new(areas.panel), at);
        } else {
            frame.render_widget(Paragraph::new(areas.panel), areas.editor);
        }
        self.draw_status(frame, areas.status);
        if fills {
            // The bar down the input box's left edge.
            let bar = self.styles.accent.bg(self.styles.raised);
            for y in areas.input.top()..areas.input.bottom() {
                frame.buffer_mut()[(areas.input.x, y)]
                    .set_symbol("┃")
                    .set_style(bar);
            }
        }
        self.draw_agents(frame, areas.agents);
        self.draw_jobs(frame, areas.jobs);
        if !self.no_color && self.theme.paints() {
            crate::theme::paint(frame.buffer_mut(), &self.palette);
        }
    }

    /// The sessions on the left: the list, then a border and a gap before the chat.
    fn draw_sessions(&mut self, frame: &mut Frame, area: Rect) {
        let inner = area.width.saturating_sub(2);
        crate::style::fill(
            frame.buffer_mut(),
            Rect {
                width: inner + 1,
                ..area
            },
            self.styles.panel,
        );
        // The shown session's details at the foot, while the list keeps a dozen rows.
        let dir = shorten_home(&self.cwd, self.home.as_deref());
        let details = crate::details::lines(
            &crate::details::Details {
                dir: &dir,
                usage: self.widgets.usage(),
                connectors: &self.connectors,
            },
            usize::from(inner),
            &self.styles,
        );
        let details_h = u16::try_from(details.len() + 1).unwrap_or(u16::MAX);
        let details_h = if area.height >= details_h + 12 {
            details_h
        } else {
            0
        };
        // A row of the fill above the list.
        let top = u16::from(self.styles.fills() && area.height - details_h > 4);
        let list = Rect {
            y: area.y + top,
            width: inner,
            height: area.height - details_h - top,
            ..area
        };
        if details_h > 0 {
            let at = Rect {
                y: area.y + area.height - details_h + 1,
                height: details_h - 1,
                ..list
            };
            frame.render_widget(Paragraph::new(details), at);
        }
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
        crate::style::pane_edge(frame.buffer_mut(), area.x + inner, area, &self.palette);
    }

    fn chat_opts(&self) -> ChatOpts {
        ChatOpts {
            expanded: self.expanded,
            show_thoughts: self.show_thoughts,
            palette: self.palette,
            active: self
                .active_ref()
                .map(|r| (r.to_string(), self.prompt.button)),
            in_panel: self.prompt.engaged && self.active_form().is_none(),
            marker: self.styles.accent,
            ui: self.forms.all().clone(),
            icons: self.icons,
            running: self.running,
            split_diffs: self.split_diffs,
        }
    }

    /// A session nothing was said in yet: where it works, the latest sessions there when the
    /// sidebar doesn't list them, and the keys to start with.
    fn empty_chat(&mut self, w: usize) -> Vec<Line<'static>> {
        let muted = self.styles.muted;
        let project = self
            .cwd
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        let mut head = vec![Span::styled(project, Modifier::BOLD)];
        if let Some(branch) = self.widgets.branch() {
            head.push(Span::styled(format!(" · {branch}"), muted));
        }
        let mut lines = vec![fit_line(Line::from(head), w), Line::default()];
        if !self.sessions_shown {
            let recent = self
                .sessions
                .recent_at(&self.cwd, &self.session, EMPTY_RECENT);
            if !recent.is_empty() {
                lines.push(Line::from(Span::styled("recent here", muted)));
                for info in &recent {
                    let age = relative_time(info.last_activity, self.now_ms);
                    let room = w.saturating_sub(width(&age) + 4);
                    let title = truncate(&session_title(info), room);
                    let pad = w.saturating_sub(width(&title) + width(&age) + 2).max(1);
                    lines.push(fit_line(
                        Line::from(vec![
                            Span::raw(format!("  {title}")),
                            Span::raw(" ".repeat(pad)),
                            Span::styled(age, muted),
                        ]),
                        w,
                    ));
                }
                lines.push(Line::default());
            }
        }
        let key = |k: &'static str, what: &'static str| {
            vec![
                Span::styled(k, self.styles.accent),
                Span::styled(format!(" {what}"), muted),
            ]
        };
        let sep = || Span::styled("  ·  ", muted);
        let mut keys = key("enter", "send");
        for (k, what) in [("/", "commands"), ("ctrl-g", "sessions"), ("/help", "keys")] {
            keys.push(sep());
            keys.extend(key(k, what));
        }
        lines.push(fit_line(Line::from(keys), w));
        lines
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
        // With fills, in from the edge as far as the input box's text.
        let content = if self.styles.fills() && content.width >= 20 && content.height > 4 {
            Rect {
                x: content.x + 2,
                y: content.y + 1,
                width: content.width - 2,
                height: content.height - 1,
            }
        } else {
            content
        };
        let opts = self.chat_opts();
        if let Some(images) = self.images.as_mut() {
            images.request(self.chat.image_sources());
            self.chat.set_images(Some(images.chat_view()));
        }
        self.chat.layout(content.width, content.height, &opts);
        if self.chat.items().is_empty() {
            let lines = if self.chat.loaded() {
                self.empty_chat(usize::from(content.width))
            } else {
                vec![Line::from(Span::styled(
                    "connecting to the session…",
                    self.styles.muted,
                ))]
            };
            frame.render_widget(Paragraph::new(lines), content);
            return;
        }
        frame.render_widget(
            Paragraph::new(self.chat.visible_at(Some(self.now_ms))),
            content,
        );
        if let Some(images) = self.images.as_mut() {
            for at in self.chat.placements() {
                images.draw(frame.buffer_mut(), content, &at, self.styles.muted);
            }
        }
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

    /// The `@` file list over the bottom of the chat, like the slash menu: a rule, then the
    /// files with their icons, the selected one marked.
    fn draw_mentions(&mut self, frame: &mut Frame, chat: Rect) {
        let matches = self.mention_matches();
        if matches.is_empty() || chat.height == 0 {
            return;
        }
        self.mention_sel = self.mention_sel.min(matches.len() - 1);
        let rows = matches
            .len()
            .min(MENU_ROWS)
            .min(usize::from(chat.height).saturating_sub(1));
        if rows == 0 {
            return;
        }
        let start = (self.mention_sel + 1).saturating_sub(rows);
        let w = usize::from(chat.width);
        let lines = matches
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(i, c)| {
                let (marker, style) = if i == self.mention_sel {
                    ("› ", self.styles.accent.add_modifier(Modifier::BOLD))
                } else if c.dir {
                    ("  ", self.styles.muted)
                } else {
                    ("  ", Style::new())
                };
                let mut spans = vec![Span::styled(marker, self.styles.accent)];
                if self.icons {
                    spans.push(neo_renderer_ratatui::file_icon(&c.path, &self.palette));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(format!("@{}", c.path), style));
                fit_line(Line::from(spans), w)
            });
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
            .map(|c| width(&c.name) + 1 + width(&c.args) + usize::from(!c.args.is_empty()))
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
                        Span::styled(c.desc.clone(), self.styles.muted),
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

    /// The prompt as it fills the editor's place: the question with a compact preview, the
    /// buttons (one focused), "1 of 3" when more wait, and a line of keys (or the note being
    /// typed).
    fn panel_lines(&self, width: u16, max_rows: usize) -> Vec<Line<'static>> {
        let Some(r) = self.active_ref() else {
            return Vec::new();
        };
        let Some(item) = self.chat.item(r) else {
            return Vec::new();
        };
        if self.chat.has_targets(r) {
            return vec![self.form_hint(r, width)];
        }
        let buttons = self.chat.buttons_of(r);
        let filtering = choice::is_long(buttons);
        // A long choice shows only the options that match, numbered from its own first line.
        let shown = filtering.then(|| choice::matching(buttons, &self.prompt.filter));
        let filtered = shown
            .as_ref()
            .map(|s| neo_renderer_ratatui::keep_buttons(item, s));
        let (item, focused) = match (&filtered, &shown) {
            (Some(filtered), Some(shown)) => (
                filtered,
                shown
                    .iter()
                    .position(|&i| i == self.prompt.button)
                    .unwrap_or(usize::MAX),
            ),
            _ => (
                item,
                self.prompt.button.min(buttons.len().saturating_sub(1)),
            ),
        };
        let w = usize::from(width);
        let marker = self.styles.accent;
        let tail = match &self.note {
            Some(note) => chat::note_line(&note.text, w.saturating_sub(2), marker),
            None => Line::from(Span::styled(
                keys_hint(
                    buttons
                        .get(self.prompt.button.min(buttons.len().saturating_sub(1)))
                        .filter(|_| !shown.as_ref().is_some_and(Vec::is_empty)),
                    shown.as_ref().map_or(buttons.len(), Vec::len),
                    filtering.then_some(self.prompt.filter.as_str()),
                ),
                self.styles.muted,
            )),
        };
        let nothing = shown.as_ref().is_some_and(Vec::is_empty);
        // The question, then the filter field, then the options.
        let head = 1 + usize::from(filtering);
        let budget = max_rows.saturating_sub(1).max(1);
        let compose = |preview: usize| {
            let mut body = chat::panel_lines(
                item,
                width,
                focused,
                preview,
                filtering,
                self.palette,
                marker,
            );
            if filtering {
                let field = chat::filter_line(&self.prompt.filter, w.saturating_sub(2), marker);
                body.insert(1.min(body.len()), chat::barred(field, marker));
                if nothing {
                    body.push(chat::barred(
                        Line::from(Span::styled("no match", self.styles.muted)),
                        marker,
                    ));
                }
            }
            body
        };
        let mut body = Vec::new();
        for preview in [10, 6, 3, 1, 0] {
            body = compose(preview);
            if body.len() <= budget {
                break;
            }
        }
        if body.len() > budget {
            // Too small for the preview: keep the question (and the filter) and a window of the
            // buttons that follows the focused one, scrolling no further than it takes to show it.
            let rows = budget.saturating_sub(head);
            let mut rest = body.split_off(head.min(body.len()));
            // The focused button is on the first line that differs from a drawing without one.
            let unfocused =
                chat::panel_lines(item, width, usize::MAX, 0, filtering, self.palette, marker);
            let at = rest
                .iter()
                .zip(unfocused.iter().skip(1))
                .position(|(a, b)| a != b)
                .unwrap_or(rest.len().saturating_sub(1));
            let max_top = rest.len().saturating_sub(rows);
            let mut top = self.prompt.panel_top.get().unwrap_or(max_top);
            if at < top {
                top = at;
            } else if at >= top + rows {
                top = at + 1 - rows;
            }
            top = top.min(max_top);
            self.prompt.panel_top.set(Some(top));
            body.extend(rest.drain(top..top + rows));
        }
        let (at, of) = self.queue();
        if of > 1
            && let Some(first) = body.first_mut()
        {
            first
                .spans
                .push(Span::styled(format!("  ({at} of {of})"), self.styles.muted));
            *first = fit_line(first.clone(), w);
        }
        body.push(fit_line(chat::barred(tail, marker), w));
        body
    }

    fn draw_editor(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        // In the input box its bar takes the prompt's place.
        let prompt = if self.styles.fills() {
            Span::raw("  ")
        } else {
            Span::styled("> ", self.styles.accent)
        };
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
            // The markers of pasted images stand out: that is where they go.
            let marks: Vec<std::ops::Range<usize>> = attach::markers(self.editor.text())
                .into_iter()
                .filter(|(_, n)| self.attachments.iter().any(|(m, _)| m == n))
                .map(|(range, _)| range)
                .collect();
            let accent = self.styles.accent;
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
                    let all = self.editor.text();
                    let end = r.start + all[r.start..r.end].trim_end_matches('\n').len();
                    let mut spans = vec![lead];
                    let mut at = r.start;
                    for m in marks.iter().filter(|m| m.start < end && m.end > r.start) {
                        let (from, to) = (m.start.max(r.start), m.end.min(end));
                        if from > at {
                            spans.push(Span::raw(all[at..from].to_string()));
                        }
                        spans.push(Span::styled(all[from..to].to_string(), accent));
                        at = to;
                    }
                    if at < end {
                        spans.push(Span::raw(all[at..end].to_string()));
                    }
                    Line::from(spans)
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), area);
            if self.editor_has_cursor() {
                let x = (2 + col).min(usize::from(area.width) - 1) as u16;
                frame.set_cursor_position(Position::new(area.x + x, area.y + (row - start) as u16));
            }
            return;
        }
        if self.editor_has_cursor() {
            frame.set_cursor_position(Position::new(area.x + 2.min(area.width - 1), area.y));
        }
    }

    /// The inputs waiting behind the running turn, one muted italic `◷` row each (their first
    /// line; not the `›` of a message that was sent), the first with a `queued` tag at the right
    /// edge, the last row counting those that don't fit.
    fn draw_waiting(&self, frame: &mut Frame, area: Rect) {
        let rows = usize::from(area.height);
        let w = usize::from(area.width);
        let shown = if self.waiting.len() > rows {
            rows - 1
        } else {
            rows
        };
        let row = self.styles.muted.add_modifier(Modifier::ITALIC);
        let mut lines: Vec<Line> = self.waiting[..shown]
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let first = input
                    .text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("");
                let mut text = format!("◷ {}", first.trim());
                if input.text.trim().lines().nth(1).is_some() {
                    text.push_str(" …");
                }
                match input.images.as_ref().map_or(0, Vec::len) {
                    0 => {}
                    1 => text.push_str("  [image]"),
                    n => text.push_str(&format!("  [{n} images]")),
                }
                let tag = (i == 0).then(|| {
                    let mut spans = vec![Span::styled("queued", self.styles.accent)];
                    if self.waiting.iter().any(|w| w.app == self.app_id) {
                        spans.push(Span::styled(" · ↑ edits", self.styles.muted));
                    }
                    Line::from(spans)
                });
                let room = w.saturating_sub(tag.as_ref().map_or(0, |t| t.width() + 2));
                let left = Line::from(Span::styled(truncate(&text, room.max(1)), row));
                split_line(left, tag, w)
            })
            .collect();
        if shown < self.waiting.len() {
            let more = format!("  … +{} more queued", self.waiting.len() - shown);
            lines.push(Line::from(Span::styled(
                truncate(&more, w),
                self.styles.muted,
            )));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// The images pasted into the draft, above the editor: thumbnails with their marker under
    /// them where the terminal shows pictures, else one row of `[image 1] cat.png`.
    fn draw_attached(&mut self, frame: &mut Frame, area: Rect) {
        let attached: Vec<(usize, String)> =
            attach::referenced(self.editor.text(), &self.attachments)
                .into_iter()
                .map(|(n, path)| (n, path.to_string()))
                .collect();
        let (accent, muted) = (self.styles.accent, self.styles.muted);
        if area.height > 1
            && let Some(images) = self.images.as_mut()
        {
            images.request(attached.iter().map(|(_, path)| path.as_str()));
            let view = images.chat_view();
            let rows = area.height - 1;
            let mut x = 0;
            for (n, path) in &attached {
                let status = view.status.get(path);
                let cols = match status {
                    Some(ImageStatus::Ready { width, height }) => {
                        thumbnail_cols(*width, *height, rows, view.cell)
                    }
                    _ => PREVIEW_COLS / 2,
                };
                if x + cols > area.width {
                    break;
                }
                let label_at = Rect {
                    x: area.x + x,
                    y: area.y + rows,
                    width: cols,
                    height: 1,
                };
                match status {
                    Some(ImageStatus::Ready { .. }) => {
                        let at = chat::Placement {
                            src: path.clone(),
                            x,
                            y: 0,
                            cols,
                            rows,
                        };
                        images.draw(frame.buffer_mut(), area, &at, muted);
                    }
                    Some(ImageStatus::Failed(_)) => frame.render_widget(
                        Paragraph::new(Span::styled("can't show it", muted)),
                        Rect {
                            y: area.y,
                            ..label_at
                        },
                    ),
                    _ => frame.render_widget(
                        Paragraph::new(Span::styled("loading…", muted)),
                        Rect {
                            y: area.y,
                            ..label_at
                        },
                    ),
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        truncate(&attach::marker(*n), usize::from(cols)),
                        accent,
                    )),
                    label_at,
                );
                x += cols + 2;
            }
            return;
        }
        let mut spans = Vec::new();
        for (n, path) in &attached {
            let name = std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            spans.push(Span::styled(attach::marker(*n), accent));
            spans.push(Span::styled(format!(" {name}  "), muted));
        }
        let line = fit_line(Line::from(spans), usize::from(area.width));
        frame.render_widget(Paragraph::new(line), area);
    }

    /// The terminal cursor sits in the editor unless something else has the keys.
    fn editor_has_cursor(&self) -> bool {
        self.focus.is_none()
            && self.secret.is_none()
            && self.overlay != Overlay::Switcher
            && !self.github.is_open()
            && self.transcript.is_none()
    }

    fn status_lines(&self, width: usize) -> [Line<'static>; 2] {
        // The project's name: the terminal's own title or prompt has the whole path.
        let cwd = shorten_home(&self.cwd, self.home.as_deref());
        let cwd = cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(&cwd);
        let cwd = if cwd.is_empty() { "/" } else { cwd };
        let branch = self.widgets.branch();
        let agent_in = self
            .agent_app
            .as_deref()
            .filter(|a| *a != self.app_id.as_str());
        let notice = self.notice.as_ref().map(|n| (n.text.as_str(), n.error));
        Status {
            cwd,
            branch: branch.as_deref(),
            settings: &self.settings,
            harness: &self.harness,
            agent_in,
            usage: self.widgets.usage_line(),
            working_secs: self
                .running
                .then(|| u64::try_from((self.now_ms - self.turn_started_ms) / 1000).unwrap_or(0)),
            spinner: SPINNER[usize::try_from(self.now_ms / 125).unwrap_or(0) % SPINNER.len()],
            queued: self.waiting.len(),
            stashed: self.stash.is_some(),
            cache_cold: self.cache_cold,
            connected: self.connected,
            approvals: self.approvals,
            notice,
            ui_hint: self.active_ref().is_none()
                && self
                    .chat
                    .prompt_refs()
                    .iter()
                    .any(|r| self.chat.has_targets(r)),
            styles: &self.styles,
        }
        .lines(width)
    }

    /// The status lines; with fills both start past the input box's bar, the first inside the
    /// box and the second under it.
    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let area = if self.styles.fills() {
            Rect {
                x: area.x + 2,
                width: area.width.saturating_sub(3),
                ..area
            }
        } else {
            area
        };
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
            key("shift-tab", "approvals: ask / review / accept edits / auto"),
            key(
                "tab",
                "complete command, note on a prompt, next panel (the chat's files first)",
            ),
            key("ctrl-s", "put the draft aside / bring it back"),
            key(
                "ctrl-g",
                "sessions sidebar: focus (up/down, enter, n new chat, esc)",
            ),
            key("ctrl-r", "GitHub issues and pull requests (/github)"),
            key(
                "ctrl-x, down",
                "subagents under the prompt: enter opens, x stops (down: empty input)",
            ),
            key(
                "diff view",
                "n/p hunk · h/l/0 sideways · s split/unified · f whole file · q close",
            ),
            key(
                "1-9, arrows, enter",
                "answer a prompt or fill in a form: tab, type, space; esc: input",
            ),
            key(
                "ctrl-p / ctrl-n",
                "use the buttons and forms above, older first",
            ),
            Line::default(),
            head("commands"),
        ];
        for c in self.commands() {
            let label = if c.args.is_empty() {
                format!("/{}", c.name)
            } else {
                format!("/{} {}", c.name, c.args)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<36}"), self.styles.accent),
                Span::styled(c.desc.clone(), self.styles.muted),
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

/// Columns a thumbnail `rows` high takes for a picture of `width` × `height` pixels, with
/// cells of `cell` pixels.
fn thumbnail_cols(width: u32, height: u32, rows: u16, cell: (u16, u16)) -> u16 {
    let (cw, ch) = cell;
    if width == 0 || height == 0 || cw == 0 {
        return PREVIEW_COLS / 2;
    }
    let px = f64::from(rows) * f64::from(ch) * f64::from(width) / f64::from(height);
    ((px / f64::from(cw)).ceil() as u16).clamp(1, PREVIEW_COLS)
}

struct Areas {
    chat: Rect,
    rule: Rect,
    /// One row above the editor: a prompt is waiting for the user to stop typing.
    hint: Rect,
    /// Rows above the editor: the inputs waiting behind the running turn.
    waiting: Rect,
    /// One row above the editor: the images going with the next message.
    attached: Rect,
    /// The editor, or the prompt that has taken its place.
    editor: Rect,
    /// The prompt's lines when it has taken the editor's place.
    panel: Vec<Line<'static>>,
    /// The input box: the rule's row to the first status line.
    input: Rect,
    status: Rect,
    /// The subagents list under the status lines.
    agents: Rect,
    /// The background commands under the subagents.
    jobs: Rect,
    side: Option<Rect>,
    /// Whether the side column holds the terminal panel, under its other panels.
    terminal: bool,
    /// The sessions sidebar.
    left: Option<Rect>,
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agents_app_tests.rs"]
mod agents_app_tests;

#[cfg(test)]
#[path = "secret_app_tests.rs"]
mod secret_app_tests;

#[cfg(test)]
#[path = "openui_app_tests.rs"]
mod openui_app_tests;
