//! The app state: chat, editor, overlays and keys, with no terminal or network in it.
//!
//! [`App::handle_key`], [`App::handle_event`] and [`App::handle_msg`] change the state and return
//! [`Effect`]s for the runtime to carry out against the daemon; [`App::draw`] paints it.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use neo_protocol::{
    AgentFocus, SecretRequest, SessionEvent, SessionEventRecord, SessionInfo, SessionState,
    SettingName, SlashCommand, WaitingInput,
};
use neo_renderer_ratatui::{
    ButtonRef, DevRef, ImageStatus, Options, Palette, UiKind, UiTarget, neoui_targets,
};
use neo_sdk::Event;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::agents::{Agents, AgentsAction, MainRow, Transcript};
use crate::attach;
use crate::chat::{self, Chat, ChatOpts, Pick};
use crate::choice;
use crate::editor::Editor;
use crate::forms::{Forms, Outcome, Pending};
use crate::github::model::Kind;
use crate::github::{Github, Outgoing};
use crate::history::{Histories, HistoryStore};
use crate::images::Images;
use crate::jobs::{Jobs, JobsAction};
use crate::mention::{self, Candidate, Mentions};
use crate::menu::{self, Entry, Item, Switcher, session_title};
use crate::reader::{Reader, ReaderAction};
use crate::secret::{self, Secret, SecretValue};
use crate::sessions::{Look, SIDEBAR_WIDTH, Sessions, SessionsAction, sidebar_width};
use crate::status::{Approvals, HarnessSettings, Status, record_setting};
use crate::style::{Styles, plain_palette};
use crate::theme::{Prefs, Theme};
use crate::toast::{self, Toasts};
use crate::util::{fit_line, relative_time, shorten_home, split_line, truncate, width};
use crate::voice::{SpaceKey, SpaceRun, Voice};
use crate::widgets::{Panel, Select, WidgetAction, Widgets};

// The rest of `impl App`, by what it does.
mod commands;
mod draw;
mod draw_input;
mod draw_status;
mod events;
mod setup;
mod focus;
mod keys;
mod layout;
mod prompts;
mod typing;

/// How long a second Ctrl-C quits.
const QUIT_WINDOW_MS: i64 = 1500;
/// How long a transient message stays.
const NOTICE_MS: i64 = 4000;
/// How often a running turn's session info is refreshed (queue length, stuck turns).
const POLL_MS: i64 = 2000;
/// Idle this long, a session's prompt cache is gone with Anthropic (5 minutes, 1 hour with long
/// retention) and OpenAI (5 minutes to an hour); DeepSeek's disk cache may outlast it.
const CACHE_COLD_MS: i64 = 60 * 60 * 1000;
/// How long a command runs before Ctrl-B offers to send it to the background.
const BACKGROUND_AFTER_MS: i64 = 3000;
/// An event older than this when it arrives is history (a replay), not news for a card.
const TOAST_FRESH_MS: i64 = 10_000;
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
/// A gap going round a full braille cell: it fills the cell, where a geometric shape sits small
/// in most fonts. A frame each [`SPIN_MS`], the rate the screen repaints while one shows.
const SPINNER: [char; 8] = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
const SPIN_MS: i64 = 80;
/// What the work row says while a turn runs, in place of a plain "working"; a length prime to
/// 5, the step between them.
const WORK_WORDS: [&str; 24] = [
    "noodling…",
    "herding bytes…",
    "asking the rubber duck…",
    "untangling spaghetti…",
    "reticulating splines…",
    "bribing the compiler…",
    "counting semicolons…",
    "chasing pointers…",
    "feeding the hamsters…",
    "shaving a yak…",
    "consulting the oracle…",
    "juggling tokens…",
    "polishing pixels…",
    "warming up the tubes…",
    "grepping the void…",
    "summoning daemons…",
    "sharpening pencils…",
    "rearranging electrons…",
    "reading the manual…",
    "brewing coffee…",
    "connecting the dots…",
    "poking at it…",
    "thinking very hard…",
    "making it make sense…",
];
/// How long each of them stays.
const WORK_WORD_MS: i64 = 12_000;
/// A chat opens with its last this many items (`view.subscribe` with a tail), and pages in this
/// many older ones whenever the view nears the oldest it has.
pub const TAIL_ITEMS: i64 = 40;
const PAGE_ITEMS: i64 = 40;
/// The chat needs at least this much beside the sessions sidebar, else the sidebar is hidden.
const CHAT_MIN_WIDTH: u16 = 52;
/// The narrowest the plan / files column gets.
const SIDE_MIN_WIDTH: u16 = 24;
/// Sessions an empty chat lists from the same repository.
const EMPTY_RECENT: usize = 5;

/// The terminal panel's width: two fifths of what the chat would have, within these.
const TERMINAL_MIN_WIDTH: u16 = 40;
const TERMINAL_MAX_WIDTH: u16 = 100;
/// Columns Alt-. / Alt-, widen or narrow a pane by.
const RESIZE_STEP: i16 = 4;
/// Most rows a prompt takes where the editor is (its question, preview, buttons and hint).
const PANEL_ROWS: u16 = 16;
/// Fewest rows the file view of an edit a prompt asks about takes; with less room, none.
const REVIEW_MIN_ROWS: u16 = 6;
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
        /// Into the running turn at once (Alt-Enter), not after it.
        steer: bool,
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
    /// Start recording from the microphone (see voice.rs; `Msg::VoiceFailed` if it can't).
    VoiceStart,
    /// Stop recording and have the daemon write it out (`Msg::Transcribed`).
    VoiceStop,
    /// Take this app's inputs back out of the session's queue (`Msg::Unqueued`).
    Unqueue {
        session: String,
    },
    /// The `count` items of the session's view ahead of item `before` (`Msg::Page`).
    Page {
        session: String,
        before: String,
        count: i64,
    },
    /// Open a link of a neoui button in the browser (xdg-open).
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
    /// Recording couldn't start: why.
    VoiceFailed(String),
    /// What was said into the microphone, written out (empty: nothing was), or why not.
    Transcribed(Result<String, String>),
    /// The inputs taken back out of the queue of `session`, oldest first.
    Unqueued {
        session: String,
        inputs: Vec<WaitingInput>,
    },
    /// Older items of `session`'s view, or why they didn't come.
    Page {
        session: String,
        page: Result<neo_protocol::ViewPage, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Help,
    Switcher,
    /// The marked sessions (Alt-').
    Marks,
    /// The first start's question: what the user does with neo.
    Setup,
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

/// A pane Alt-. / Alt-, resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Sessions,
    Side,
}

/// Columns each pane was widened (or, negative, narrowed) by with Alt-. / Alt-,.
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

/// The keys line under a prompt in the input area; `review`: the prompt is an edit, with this
/// many comments written on it.
fn keys_hint(
    focused: Option<&ButtonRef>,
    buttons: usize,
    filter: Option<&str>,
    review: Option<usize>,
) -> String {
    // Each key as it is pressed, then what it does.
    let mut parts = Vec::new();
    if let Some(button) = focused {
        // A setting: Enter (or Space) changes it and the card stays, until Esc.
        if button.stays {
            parts.push("Enter Change".to_string());
        } else {
            parts.push(format!("Enter {}", button.label));
        }
        if takes_note(button) {
            parts.push("Tab Add a note".to_string());
        }
    }
    if buttons > 1 {
        parts.insert(1.min(parts.len()), "←/→ Choose".to_string());
    }
    match review {
        Some(0) => parts.push("n/p Next change · v Select · Space c Comment".to_string()),
        Some(1) => parts.push("n/p Next change · Space c Comment (1 goes along)".to_string()),
        Some(n) => parts.push(format!("n/p Next change · Space c Comment ({n} go along)")),
        None => {}
    }
    parts.push(
        match filter {
            Some(f) if !f.is_empty() => "Esc Clear",
            _ if focused.is_some_and(|b| b.stays) => "Esc Close",
            _ => "Esc Dismiss",
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

/// A setting that is text (`/settings`' system prompt words) being written in the editor: the
/// card and the button it answers, and the draft that waits meanwhile.
#[derive(Debug)]
struct SettingInput {
    r#ref: String,
    button: usize,
    multiline: bool,
    name: String,
    draft: (String, Vec<(usize, String)>),
}

/// Buttons whose answer the agent hears can carry a note for it: a permission, a review of its
/// changes; not a setting (/settings, /model). The daemon marks them.
fn takes_note(button: &ButtonRef) -> bool {
    button.note
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
    /// A reply opened as markdown (Enter on a picked reply), in the chat's place.
    reader: Option<Reader>,
    /// Last whole second the agents list was painted for (its clocks run).
    agents_tick: i64,
    chat: Chat,
    /// Pictures in the chat; None shows the `[image …]` placeholder.
    images: Option<Images>,
    /// File type icons in the chat and the files panel (`NEO_TERM_ICONS=off`: none).
    icons: bool,
    /// `/settings split-diffs`: diffs side by side, in the chat and the file view.
    split_diffs: bool,
    /// `/settings chart-images`: charts as pictures where the terminal draws them.
    chart_images: bool,
    /// `/settings hide-edits`: only the newest edit shows its changes.
    hide_edits: bool,
    /// The user's words in every agent's system prompt (`/system-prompt`), for editing them.
    prompt_addition: Option<String>,
    /// What Alt-. / Alt-, did to the panes' widths.
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
    /// What was in the editor of each session shown before, with its images: it is there again
    /// when the session is shown again.
    drafts: HashMap<String, (String, Vec<(usize, String)>)>,
    histories: Histories,
    overlay: Overlay,
    switcher: Switcher,
    focus: Option<Focus>,
    sessions: Sessions,
    /// The marked sessions, slot 1 first: Ctrl-G 1…9 (or Alt-1…9) show them, Alt-m marks the shown one,
    /// Alt-' lists them (kept in `term.json`).
    marks: Vec<String>,
    /// The row the marks list is on.
    marks_sel: usize,
    /// The setup walkthrough, while it is open.
    onboarding: Option<crate::onboarding::Onboarding>,
    /// It opened on its own, on the first start (not `/setup`).
    setup_first: bool,
    /// The first start's walkthrough was opened: not again while this runs.
    setup_asked: bool,
    /// The agents the daemon can run (harnesses.list), for the walkthrough's steps about them.
    harnesses: Vec<neo_protocol::HarnessInfo>,
    /// The last key was Ctrl-G: a digit now shows a marked chat.
    after_ctrl_g: bool,
    /// The sessions sidebar is switched on (it still needs a wide enough terminal).
    sessions_on: bool,
    sessions_shown: bool,
    /// The session's details (context, cost) are drawn at the foot of the sessions sidebar:
    /// the status line leaves them out.
    details_shown: bool,
    /// The working line is drawn over the input box: the status line leaves it out.
    work_shown: bool,
    /// The file view of the edit a prompt asks about is drawn: it has the keys that don't
    /// answer the prompt.
    review_shown: bool,
    /// The sidebar lists all the apps, not just the first few.
    apps_open: bool,
    /// Where the sidebar's row that shows or hides the rest of the apps is, if it is drawn.
    apps_toggle_at: Option<Rect>,
    /// The sidebar's keys are on the apps, on this row (counted from the first app).
    apps_sel: Option<usize>,
    /// Where the apps were drawn last.
    app_rows: crate::details::AppRows,
    /// Event cards in the top right of the chat.
    toasts: Toasts,
    /// Background commands whose end got a card already.
    toasted_jobs: HashSet<String>,
    /// Ctrl-G while the sidebar was hidden: focus it once it is drawn.
    sessions_wanted: bool,
    prompt: PromptState,
    /// The visual state of the neoui docs in the chat and the values on their way out.
    forms: Forms,
    /// The note typed for the focused button's answer (Tab on a card).
    note: Option<Note>,
    /// A setting's text is being written in the editor.
    setting_input: Option<SettingInput>,
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
    /// Speech to text (hold Space): recording, or being written out.
    voice: Voice,
    /// The Spaces just typed in the editor, to tell a held Space (see space).
    spaces: SpaceRun,
    /// The context was cleared (/clear) since the last turn: there is no cache to go cold.
    context_cleared: bool,
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
    /// The terminal panel was scrolled last, not the chat: the chat follows it to the command
    /// on its bottom row (the chat's top then, to tell when the chat is scrolled itself).
    terminal_leads: Option<usize>,
    /// Where the chat's lines were drawn last (the mouse selects text there).
    chat_area: Option<Rect>,
    dirty: bool,
    clear: bool,
}

impl App {
    pub fn new(config: Config) -> App {
        let theme = config.prefs.theme();
        let marks = config.prefs.marks();
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
            reader: None,
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
            chart_images: false,
            hide_edits: false,
            prompt_addition: None,
            resized: Resized::default(),
            connectors: Vec::new(),
            editor: Editor::default(),
            attachments: Vec::new(),
            stash: None,
            drafts: HashMap::new(),
            histories: Histories::new(config.history),
            overlay: Overlay::None,
            switcher: Switcher::new(&session),
            focus: None,
            sessions: Sessions::default(),
            marks,
            marks_sel: 0,
            onboarding: None,
            setup_first: false,
            setup_asked: false,
            harnesses: Vec::new(),
            after_ctrl_g: false,
            sessions_on: true,
            sessions_shown: false,
            details_shown: false,
            work_shown: false,
            review_shown: false,
            apps_open: false,
            apps_toggle_at: None,
            apps_sel: None,
            app_rows: Default::default(),
            toasts: Toasts::default(),
            toasted_jobs: HashSet::new(),
            sessions_wanted: false,
            prompt: PromptState::default(),
            forms: Forms::default(),
            note: None,
            setting_input: None,
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
            voice: Voice::Off,
            spaces: SpaceRun::default(),
            context_cleared: false,
            quit_armed_until: 0,
            now_ms: now,
            last_poll_ms: now,
            last_spin: 0,
            size: (80, 24),
            sidebar_shown: false,
            terminal_on: true,
            terminal_area: None,
            terminal_leads: None,
            chat_area: None,
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
        let rows = self.widgets.jobs(now_ms);
        self.toast_jobs(&rows, now_ms);
        let jobs = rows.len();
        if self.toasts.tick(now_ms) {
            self.dirty = true;
        }
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
        // The chat opened with its latest items: older ones come as they are needed.
        if let Some(before) = self.chat.wants_page().map(str::to_string) {
            self.chat.paging(true);
            fx.push(Effect::Page {
                session: self.session.clone(),
                before,
                count: PAGE_ITEMS,
            });
        }
        if self.running || self.sessions_shown && self.sessions.any_running() {
            let frame = now_ms / SPIN_MS;
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
            && !self.context_cleared
            && !self.chat.items().is_empty()
            && self
                .sessions
                .get(&self.session)
                .is_some_and(|s| now_ms - s.last_activity > CACHE_COLD_MS);
        if cold != self.cache_cold {
            self.cache_cold = cold;
            self.dirty = true;
        }
        // A held Space whose repeats stopped was let go; the seconds recorded count up.
        if let Voice::Recording(_) = self.voice {
            if self.spaces.released(now_ms) {
                self.stop_voice(&mut fx);
            } else if tick_s {
                self.dirty = true;
            }
        }
        if self.voice == Voice::Transcribing && now_ms / SPIN_MS != self.last_spin {
            self.last_spin = now_ms / SPIN_MS;
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

    /// Alt-. / Alt-,: the focused pane (else the side column while it holds the terminal panel,
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
    /// Over the input, under the chat: the file view of the edit the prompt asks about.
    review: Rect,
    /// One row under the chat while a turn runs: the working line.
    work: Rect,
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
mod tests;

#[cfg(test)]
#[path = "agents_app_tests.rs"]
mod agents_app_tests;

#[cfg(test)]
#[path = "secret_app_tests.rs"]
mod secret_app_tests;

#[cfg(test)]
#[path = "neoui_app_tests.rs"]
mod neoui_app_tests;
