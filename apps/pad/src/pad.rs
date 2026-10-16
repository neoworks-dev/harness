//! The Pad window: the code editor stand-in. Open files on the left, the agent's chat pane on
//! the right (collapsed while the agent lives elsewhere), a path prompt on ctrl-o.
//!
//! This file is the state and what happens to it (keys, bus events, tool calls); `view.rs` draws it.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle,
    Focusable, KeyBinding, Pixels, ScrollStrategy, UTF16Selection, UniformListScrollHandle, Window,
    actions,
};
use suite_protocol::{AppPresence, ConnectorKind, LaunchSpec, McpEndpoint};
use suite_renderer_gpui::{ChatEvent, ChatFonts, ChatView};
use suite_sdk::tools::ToolServer;
use suite_sdk::{Client, Event};

use crate::buffer::{Buffer, Pos};
use crate::editor::{Editor, Effect, resolve_path};
use crate::field::Field;
use crate::focus::{Change, Follower};
use crate::gateway::Gateway;
use crate::tools::{self, Request};

/// This app's name on the bus: its presence, and the `app` of the input it sends.
pub const APP: &str = "code-editor";
pub const TITLE: &str = "Code editor (Pad)";

/// Embedded in `main` because GPUI on Linux only knows fonts by exact family name and its
/// default one (IBM Plex Sans) is not generally installed.
pub const FONT: &str = "Geist";

pub const EDITOR_CONTEXT: &str = "PadEditor";
pub const INPUT_CONTEXT: &str = "PadInput";
pub const PROMPT_CONTEXT: &str = "PadPrompt";

actions!(
    pad,
    [
        Enter,
        Escape,
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        PageUp,
        PageDown,
        DocStart,
        DocEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Tab,
        Undo,
        Redo,
        Save,
        Reload,
        OpenPrompt,
        CloseFile,
        NextTab,
        PrevTab,
        FocusEditor,
        FocusChatPane,
        FocusInput,
        CycleFocus,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        // Anywhere.
        KeyBinding::new("ctrl-1", FocusEditor, None),
        KeyBinding::new("ctrl-2", FocusChatPane, None),
        KeyBinding::new("ctrl-3", FocusInput, None),
        KeyBinding::new("f6", CycleFocus, None),
        KeyBinding::new("ctrl-o", OpenPrompt, None),
        KeyBinding::new("ctrl-s", Save, None),
        KeyBinding::new("ctrl-r", Reload, None),
        KeyBinding::new("ctrl-w", CloseFile, None),
        KeyBinding::new("ctrl-pagedown", NextTab, None),
        KeyBinding::new("ctrl-pageup", PrevTab, None),
        // The editor.
        KeyBinding::new("enter", Enter, Some(EDITOR_CONTEXT)),
        KeyBinding::new("escape", Escape, Some(EDITOR_CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(EDITOR_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(EDITOR_CONTEXT)),
        KeyBinding::new("left", Left, Some(EDITOR_CONTEXT)),
        KeyBinding::new("right", Right, Some(EDITOR_CONTEXT)),
        KeyBinding::new("up", Up, Some(EDITOR_CONTEXT)),
        KeyBinding::new("down", Down, Some(EDITOR_CONTEXT)),
        KeyBinding::new("home", Home, Some(EDITOR_CONTEXT)),
        KeyBinding::new("end", End, Some(EDITOR_CONTEXT)),
        KeyBinding::new("pageup", PageUp, Some(EDITOR_CONTEXT)),
        KeyBinding::new("pagedown", PageDown, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-home", DocStart, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-end", DocEnd, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-c", Copy, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-x", Cut, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(EDITOR_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(EDITOR_CONTEXT)),
        KeyBinding::new("tab", Tab, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-z", Undo, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-z", Redo, Some(EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-y", Redo, Some(EDITOR_CONTEXT)),
        // The chat input.
        KeyBinding::new("enter", Enter, Some(INPUT_CONTEXT)),
        KeyBinding::new("escape", Escape, Some(INPUT_CONTEXT)),
        // Up from the input moves into the chat's items with buttons.
        KeyBinding::new("up", Up, Some(INPUT_CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(INPUT_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(INPUT_CONTEXT)),
        KeyBinding::new("left", Left, Some(INPUT_CONTEXT)),
        KeyBinding::new("right", Right, Some(INPUT_CONTEXT)),
        KeyBinding::new("home", Home, Some(INPUT_CONTEXT)),
        KeyBinding::new("ctrl-a", Home, Some(INPUT_CONTEXT)),
        KeyBinding::new("end", End, Some(INPUT_CONTEXT)),
        KeyBinding::new("ctrl-e", End, Some(INPUT_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(INPUT_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(INPUT_CONTEXT)),
        // The path prompt.
        KeyBinding::new("enter", Enter, Some(PROMPT_CONTEXT)),
        KeyBinding::new("escape", Escape, Some(PROMPT_CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(PROMPT_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(PROMPT_CONTEXT)),
        KeyBinding::new("left", Left, Some(PROMPT_CONTEXT)),
        KeyBinding::new("right", Right, Some(PROMPT_CONTEXT)),
        KeyBinding::new("home", Home, Some(PROMPT_CONTEXT)),
        KeyBinding::new("ctrl-a", Home, Some(PROMPT_CONTEXT)),
        KeyBinding::new("end", End, Some(PROMPT_CONTEXT)),
        KeyBinding::new("ctrl-e", End, Some(PROMPT_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(PROMPT_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(PROMPT_CONTEXT)),
    ]);
}

/// The presence: what the daemon knows about this app. `context` is the current file and line.
pub fn presence(
    open: bool,
    endpoint: Option<McpEndpoint>,
    context: Option<BTreeMap<String, String>>,
) -> AppPresence {
    AppPresence {
        templates: open.then(tools::templates),
        app: APP.into(),
        kind: ConnectorKind::App,
        title: Some(TITLE.into()),
        open,
        context: context.filter(|_| open),
        mcp: endpoint.filter(|_| open),
        // Lets the agent start Pad with apps_launch while it is closed.
        launch: std::env::current_exe().ok().map(|exe| LaunchSpec {
            command: vec![exe.to_string_lossy().into_owned()],
            cwd: None,
        }),
        home: None,
        pid: Some(i64::from(std::process::id())),
    }
}

/// What the window needs besides the bus client.
pub struct Setup {
    /// The bus socket (the lsp gateway's address is found next to it).
    pub socket: PathBuf,
    /// Tool calls from the tool server's threads.
    pub requests: async_channel::Receiver<Request>,
    /// Where the tool server listens, for the presence updates.
    pub endpoint: Option<McpEndpoint>,
    /// Keeps the tool server alive as long as the window.
    pub server: Option<ToolServer>,
    /// Files to open at startup.
    pub open: Vec<(PathBuf, Option<usize>)>,
}

/// A bus call the view wants made. They run one after another on a worker (blocking calls
/// belong on a background thread), so a quick open, close, open can't reorder.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    SubscribeView(String),
    UnsubscribeView(String),
    Input {
        session: String,
        text: String,
    },
    /// A button in the chat: `ui.action`.
    Action {
        session: String,
        item: String,
        action: String,
        args: Option<serde_json::Value>,
    },
    Cancel(String),
    Move {
        app: Option<&'static str>,
    },
    /// The current file or line changed.
    Presence(Box<AppPresence>),
    /// A file was saved.
    FileChanged(String),
}

impl Op {
    fn run(&self, client: &Client) -> suite_sdk::Result<()> {
        match self {
            // The snapshot also arrives as an event, which is what the chat is fed from.
            Op::SubscribeView(session) => client.subscribe_view(session).map(drop),
            Op::UnsubscribeView(session) => client.unsubscribe_view(session),
            Op::Input { session, text } => client.input(session, APP, text, &[]),
            Op::Action {
                session,
                item,
                action,
                args,
            } => client.action(session, item, action, args.clone()),
            Op::Cancel(session) => client.cancel(session),
            Op::Move { app } => client.move_agent(*app, None),
            Op::Presence(presence) => client.set_presence((**presence).clone()),
            Op::FileChanged(path) => client.file_changed(path),
        }
    }
}

/// Where the keyboard is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Editor,
    Chat,
    Input,
    Prompt,
}

pub struct Pad {
    ops: async_channel::Sender<Op>,
    pub(crate) follower: Follower,
    /// The app the agent is in right now (`None`: no app can host it, or not known yet).
    pub(crate) agent_at: Option<String>,
    pub(crate) chat: Entity<ChatView>,
    /// The chat input.
    pub(crate) field: Field,
    pub(crate) input_focus: FocusHandle,
    pub(crate) editor_focus: FocusHandle,
    pub(crate) prompt_focus: FocusHandle,
    pub(crate) connected: bool,
    /// The last failed call or a short report, shown in the status line until the next action.
    pub(crate) notice: Option<String>,
    pub(crate) editor: Editor,
    /// The open-file prompt (ctrl-o), while it is up.
    pub(crate) prompt: Option<Field>,
    pub(crate) scroll: UniformListScrollHandle,
    /// Where the editor body was drawn, for mapping clicks to text positions.
    pub(crate) body: Rc<Cell<Bounds<Pixels>>>,
    gateway: Gateway,
    endpoint: Option<McpEndpoint>,
    _server: Option<ToolServer>,
    last_context: Option<BTreeMap<String, String>>,
    /// The file ctrl-w refused to close because of its edits; a second ctrl-w discards them.
    pending_close: Option<PathBuf>,
}

impl Pad {
    pub fn new(
        client: Arc<Client>,
        setup: Setup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pad {
        let (ops, queue) = async_channel::unbounded::<Op>();

        // Pushed events: focus, session events, connection state, file.changed.
        let events = client.events();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update(cx, |pad, cx| pad.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Tool calls from the tool server's threads.
        let requests = setup.requests;
        cx.spawn(async move |this, cx| {
            while let Ok(request) = requests.recv().await {
                if this
                    .update(cx, |pad, cx| pad.run_tool(request, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // Asset handles in the chat (`asset://img-…`) are files in the daemon's store.
        let resolver_client = client.clone();
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(cx);
            chat.set_fonts(
                ChatFonts {
                    sans: FONT.into(),
                    ..ChatFonts::default()
                },
                cx,
            );
            chat.set_image_resolver(
                move |handle| {
                    resolver_client
                        .get_asset(handle)
                        .ok()
                        .map(|asset| PathBuf::from(asset.path))
                },
                cx,
            );
            chat
        });
        cx.subscribe_in(&chat, window, Pad::on_chat).detach();

        // Bus calls, one at a time.
        cx.spawn(async move |this, cx| {
            while let Ok(op) = queue.recv().await {
                let client = client.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { op.run(&client) })
                    .await;
                if let Err(err) = result {
                    let shown = this.update(cx, |pad, cx| {
                        pad.notice = Some(err.to_string());
                        cx.notify();
                    });
                    if shown.is_err() {
                        break;
                    }
                }
            }
        })
        .detach();

        let mut pad = Pad {
            ops,
            follower: Follower::new(APP),
            agent_at: None,
            chat,
            field: Field::default(),
            input_focus: cx.focus_handle(),
            editor_focus: cx.focus_handle(),
            prompt_focus: cx.focus_handle(),
            connected: true,
            notice: None,
            editor: Editor::default(),
            prompt: None,
            scroll: UniformListScrollHandle::new(),
            body: Rc::default(),
            gateway: Gateway::beside(&setup.socket),
            endpoint: setup.endpoint,
            _server: setup.server,
            last_context: None,
            pending_close: None,
        };
        for (path, line) in setup.open {
            pad.open_path(&path, line, cx);
        }
        pad
    }

    #[cfg(test)]
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    #[cfg(test)]
    pub fn editor_mut_for_test(&mut self) -> &mut Editor {
        &mut self.editor
    }

    #[cfg(test)]
    pub fn save_for_test(&mut self, cx: &mut Context<Self>) {
        self.save_current(cx);
    }

    fn do_op(&self, op: Op) {
        // Unbounded and the worker outlives the window, so this only fails during shutdown.
        let _ = self.ops.try_send(op);
    }

    // --- bus events ---

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Focus(focus) => {
                self.agent_at = focus.app.clone();
                self.notice = None;
                for change in self.follower.on_focus(&focus) {
                    match change {
                        Change::Open(session) => self.do_op(Op::SubscribeView(session)),
                        Change::Close(session) => {
                            self.chat.update(cx, |chat, cx| chat.clear(cx));
                            self.do_op(Op::UnsubscribeView(session));
                        }
                    }
                }
            }
            // Views of a session we just left can still be queued.
            Event::ViewSnapshot(snapshot) => {
                if self.follower.pane() == Some(snapshot.session.as_str()) {
                    self.chat
                        .update(cx, |chat, cx| chat.apply_snapshot(snapshot, cx));
                }
            }
            Event::ViewPatch(patch) => {
                if self.follower.pane() == Some(patch.session.as_str()) {
                    self.chat.update(cx, |chat, cx| chat.apply_patch(patch, cx));
                }
            }
            Event::Theme(theme) => self.chat.update(cx, |chat, cx| chat.set_theme(&theme, cx)),
            Event::Connection(up) => self.connected = up,
            Event::FileChanged(changed) => {
                // Our own saves aren't echoed, but be safe.
                if changed.app.as_deref() != Some(APP) {
                    let effects = self.editor.on_file_changed(Path::new(&changed.path));
                    self.run_effects(effects, cx);
                    self.touched(cx, false);
                }
            }
            Event::Session(_) | Event::Subscribed { .. } | Event::Connectors(_) => {}
        }
        cx.notify();
    }

    fn on_chat(
        &mut self,
        _: &Entity<ChatView>,
        event: &ChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChatEvent::Action { item, action, args } => {
                if let Some(session) = self.follower.pane() {
                    self.do_op(Op::Action {
                        session: session.into(),
                        item: item.clone(),
                        action: action.clone(),
                        args: args.clone(),
                    });
                }
            }
            ChatEvent::Copied(_) => {}
            ChatEvent::Leave => window.focus(&self.input_focus),
        }
        cx.notify();
    }

    // --- effects of editor changes ---

    pub(crate) fn run_effects(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::FileChanged(path) => {
                    self.do_op(Op::FileChanged(path.to_string_lossy().into_owned()));
                }
                Effect::Diagnose(path) => self.diagnose(path, cx),
            }
        }
    }

    /// Asks the lsp service about a file, off the UI thread.
    fn diagnose(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(revision) = self.editor.begin_diagnose(&path) else {
            return;
        };
        let gateway = self.gateway.clone();
        cx.spawn(async move |this, cx| {
            let asked = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { gateway.diagnostics(&asked) })
                .await;
            this.update(cx, |pad, cx| {
                pad.editor.finish_diagnose(&path, revision, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// After anything that moved the cursor or changed the text: keep the cursor in view and
    /// tell the daemon the new file/line when they changed.
    pub(crate) fn touched(&mut self, cx: &mut Context<Self>, scroll: bool) {
        if scroll && let Some(doc) = self.editor.current() {
            self.scroll
                .scroll_to_item(doc.buffer.cursor().line, ScrollStrategy::Top);
        }
        self.sync_context();
        cx.notify();
    }

    fn sync_context(&mut self) {
        let context = self.editor.context();
        if context == self.last_context {
            return;
        }
        self.last_context = context.clone();
        let presence = presence(true, self.endpoint.clone(), context);
        self.do_op(Op::Presence(Box::new(presence)));
    }

    /// Opens a file for the user (the prompt, the command line).
    pub(crate) fn open_path(&mut self, path: &Path, line: Option<usize>, cx: &mut Context<Self>) {
        match self.editor.open(path, line) {
            Ok((_, effects)) => {
                self.notice = None;
                self.run_effects(effects, cx);
                self.show_cursor_line();
            }
            Err(why) => self.notice = Some(why),
        }
        self.touched(cx, false);
    }

    /// Scrolls so the cursor line is in the middle of the editor.
    fn show_cursor_line(&self) {
        if let Some(doc) = self.editor.current() {
            self.scroll
                .scroll_to_item_strict(doc.buffer.cursor().line, ScrollStrategy::Center);
        }
    }

    // --- tools ---

    fn run_tool(&mut self, request: Request, cx: &mut Context<Self>) {
        let (result, outcome) = tools::apply(&mut self.editor, request.call);
        self.run_effects(outcome.effects, cx);
        if let Some(line) = outcome.reveal {
            self.scroll
                .scroll_to_item_strict(line, ScrollStrategy::Center);
        }
        self.touched(cx, false);
        let _ = request.reply.send(result);
    }

    // --- focus ---

    /// Which part has the keyboard.
    pub fn target(&self, window: &Window) -> Target {
        if self.prompt.is_some() && self.prompt_focus.is_focused(window) {
            Target::Prompt
        } else if self.input_focus.is_focused(window) {
            Target::Input
        } else if self.editor_focus.is_focused(window) {
            Target::Editor
        } else {
            Target::Chat
        }
    }

    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        window.focus(&self.editor_focus);
        cx.notify();
    }

    fn focus_chat_pane(&mut self, _: &FocusChatPane, window: &mut Window, cx: &mut Context<Self>) {
        if !self.agent_here(cx) {
            return;
        }
        self.prompt = None;
        let handle = self.chat.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    fn focus_input(&mut self, _: &FocusInput, window: &mut Window, cx: &mut Context<Self>) {
        if !self.agent_here(cx) {
            return;
        }
        self.prompt = None;
        window.focus(&self.input_focus);
        cx.notify();
    }

    /// The chat pane only exists while the agent is here; says so when it doesn't.
    fn agent_here(&mut self, cx: &mut Context<Self>) -> bool {
        let here = self.follower.pane().is_some();
        if !here {
            self.notice = Some("The agent isn't here: use \"Bring the agent here\".".into());
            cx.notify();
        }
        here
    }

    fn cycle_focus(&mut self, _: &CycleFocus, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        if self.follower.pane().is_none() {
            window.focus(&self.editor_focus);
            cx.notify();
            return;
        }
        match self.target(window) {
            Target::Editor => {
                let handle = self.chat.read(cx).focus_handle(cx);
                window.focus(&handle);
            }
            Target::Chat => window.focus(&self.input_focus),
            Target::Input | Target::Prompt => window.focus(&self.editor_focus),
        }
        cx.notify();
    }

    // --- chat input ---

    pub(crate) fn send_input(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.follower.pane().map(String::from) else {
            return;
        };
        let text = self.field.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.field.take();
        self.do_op(Op::Input { session, text });
        cx.notify();
    }

    pub(crate) fn send_home(&self) {
        self.do_op(Op::Move { app: None });
    }

    pub(crate) fn bring_here(&self) {
        self.do_op(Op::Move { app: Some(APP) });
    }

    // --- actions, by where the keyboard is ---

    /// The single-line field the keyboard is in, if it is in one.
    fn field_mut(&mut self, window: &Window) -> Option<&mut Field> {
        match self.target(window) {
            Target::Input => Some(&mut self.field),
            Target::Prompt => self.prompt.as_mut(),
            _ => None,
        }
    }

    /// A cursor move in the editor.
    fn nav(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Buffer)) {
        self.notice = None;
        if let Some(doc) = self.editor.current_mut() {
            change(&mut doc.buffer);
            doc.highlight = None;
        }
        self.touched(cx, true);
    }

    /// A text edit in the editor.
    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Buffer)) {
        self.nav(cx, change);
        self.editor.note_edit();
    }

    fn page_rows(&self) -> isize {
        let rows = self.scroll.0.borrow().last_item_size.map_or(20.0, |size| {
            f32::from(size.item.height) / crate::view::LINE_HEIGHT
        });
        (rows as isize - 1).max(1)
    }

    fn on_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.change(cx, Buffer::newline),
            Target::Input => self.send_input(cx),
            Target::Prompt => self.submit_prompt(window, cx),
            Target::Chat => {}
        }
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.nav(cx, Buffer::clear_selection),
            Target::Input => {
                // Idle sessions ack and ignore it, so there is no need to check the turn state.
                if let Some(session) = self.follower.pane() {
                    self.do_op(Op::Cancel(session.into()));
                }
            }
            Target::Prompt => {
                self.prompt = None;
                window.focus(&self.editor_focus);
                cx.notify();
            }
            Target::Chat => {}
        }
    }

    fn on_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.change(cx, Buffer::backspace),
            _ => self.edit_field(window, cx, Field::backspace),
        }
    }

    fn on_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.change(cx, Buffer::delete),
            _ => self.edit_field(window, cx, Field::delete),
        }
    }

    fn on_left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.nav(cx, |b| b.move_left(false)),
            _ => self.edit_field(window, cx, Field::left),
        }
    }

    fn on_right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.nav(cx, |b| b.move_right(false)),
            _ => self.edit_field(window, cx, Field::right),
        }
    }

    fn on_up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.nav(cx, |b| b.move_lines(-1, false)),
            Target::Input => {
                self.chat
                    .update(cx, |chat, cx| chat.focus_last_actionable(window, cx));
            }
            _ => {}
        }
    }

    fn on_down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        if self.target(window) == Target::Editor {
            self.nav(cx, |b| b.move_lines(1, false));
        }
    }

    fn on_home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.nav(cx, |b| b.move_home(false)),
            _ => self.edit_field(window, cx, Field::home),
        }
    }

    fn on_end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => self.nav(cx, |b| b.move_end(false)),
            _ => self.edit_field(window, cx, Field::end),
        }
    }

    fn edit_field(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Field),
    ) {
        if let Some(field) = self.field_mut(window) {
            change(field);
            cx.notify();
        }
    }

    fn on_page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows();
        self.nav(cx, |b| b.move_lines(-rows, false));
    }

    fn on_page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows();
        self.nav(cx, |b| b.move_lines(rows, false));
    }

    fn on_select(&mut self, cx: &mut Context<Self>, how: impl FnOnce(&mut Buffer)) {
        self.nav(cx, how);
    }

    fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        match self.target(window) {
            Target::Editor => self.change(cx, |b| b.insert(&text)),
            _ => self.edit_field(window, cx, |field| field.insert(&text)),
        }
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self
            .editor
            .current()
            .and_then(|doc| doc.buffer.selected_text())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let mut cut = None;
        self.change(cx, |b| cut = b.cut());
        if let Some(text) = cut {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_current(cx);
    }

    fn save_current(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.editor.current_index() else {
            return;
        };
        match self.editor.save(index) {
            Ok(effects) => {
                self.notice = self
                    .editor
                    .current()
                    .map(|doc| format!("Saved {}", doc.path().display()));
                self.run_effects(effects, cx);
            }
            Err(why) => self.notice = Some(why),
        }
        self.touched(cx, false);
    }

    fn on_reload(&mut self, _: &Reload, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.editor.current_index() else {
            return;
        };
        match self.editor.reload(index) {
            Ok(effects) => {
                self.notice = Some("Reloaded from disk".into());
                self.run_effects(effects, cx);
            }
            Err(why) => self.notice = Some(why),
        }
        self.touched(cx, true);
    }

    fn on_close_file(&mut self, _: &CloseFile, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.editor.current_index() else {
            return;
        };
        let path = self.editor.docs()[index].path().to_path_buf();
        let force = self.pending_close.as_ref() == Some(&path);
        if self.editor.close(index, force) {
            self.pending_close = None;
            self.notice = None;
            self.show_cursor_line();
        } else {
            self.notice = Some("Unsaved changes: ctrl-s saves, ctrl-w again discards them.".into());
            self.pending_close = Some(path);
        }
        self.touched(cx, false);
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.cycle(1);
        self.show_cursor_line();
        self.touched(cx, false);
    }

    fn on_prev_tab(&mut self, _: &PrevTab, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.cycle(-1);
        self.show_cursor_line();
        self.touched(cx, false);
    }

    pub(crate) fn switch_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.editor.switch(index);
        self.show_cursor_line();
        self.touched(cx, false);
    }

    fn open_prompt(&mut self, _: &OpenPrompt, window: &mut Window, cx: &mut Context<Self>) {
        let mut field = Field::default();
        // Start in the current file's directory.
        if let Some(dir) = self.editor.current().and_then(|doc| doc.path().parent()) {
            field.insert(&format!("{}/", dir.display()));
        }
        self.prompt = Some(field);
        window.focus(&self.prompt_focus);
        cx.notify();
    }

    fn submit_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut field) = self.prompt.take() else {
            return;
        };
        window.focus(&self.editor_focus);
        let text = field.take();
        let text = text.trim();
        if text.is_empty() {
            cx.notify();
            return;
        }
        // `path:line` opens at a line.
        let (name, line) = match text.rsplit_once(':') {
            Some((name, line)) if line.chars().all(|c| c.is_ascii_digit()) && !line.is_empty() => {
                (name, line.parse::<usize>().ok())
            }
            _ => (text, None),
        };
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let path = resolve_path(name, &base);
        self.open_path(&path, line, cx);
    }

    // --- mouse ---

    /// A click or drag in line `line` at window x `x`.
    pub(crate) fn point_at(
        &mut self,
        line: usize,
        x: Pixels,
        extend: bool,
        cx: &mut Context<Self>,
        char_width: Pixels,
    ) {
        let origin = self.body.get().origin.x + crate::view::text_left();
        let vcol = (f32::from(x - origin) / f32::from(char_width))
            .max(0.0)
            .round() as usize;
        self.nav(cx, |b| {
            let line = line.min(b.line_count() - 1);
            let col = crate::buffer::col_at_visual(b.line(line), vcol);
            b.set_cursor(Pos::new(line, col), extend);
        });
    }

    pub(crate) fn go_to(&mut self, line: usize, column: usize, cx: &mut Context<Self>) {
        self.nav(cx, |b| b.set_cursor(Pos::new(line, column), false));
    }
}

impl Focusable for Pad {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.editor_focus.clone()
    }
}

impl Pad {
    /// Binds the action handlers for the root element.
    pub(crate) fn listeners(cx: &mut Context<Self>, root: gpui::Div) -> gpui::Div {
        use gpui::InteractiveElement as _;
        root.on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(
                cx.listener(|pad, _: &DocStart, _, cx| pad.nav(cx, |b| b.move_doc_start(false))),
            )
            .on_action(cx.listener(|pad, _: &DocEnd, _, cx| pad.nav(cx, |b| b.move_doc_end(false))))
            .on_action(
                cx.listener(|pad, _: &SelectLeft, _, cx| pad.on_select(cx, |b| b.move_left(true))),
            )
            .on_action(
                cx.listener(|pad, _: &SelectRight, _, cx| {
                    pad.on_select(cx, |b| b.move_right(true))
                }),
            )
            .on_action(
                cx.listener(|pad, _: &SelectUp, _, cx| {
                    pad.on_select(cx, |b| b.move_lines(-1, true))
                }),
            )
            .on_action(cx.listener(|pad, _: &SelectDown, _, cx| {
                pad.on_select(cx, |b| b.move_lines(1, true))
            }))
            .on_action(
                cx.listener(|pad, _: &SelectHome, _, cx| pad.on_select(cx, |b| b.move_home(true))),
            )
            .on_action(
                cx.listener(|pad, _: &SelectEnd, _, cx| pad.on_select(cx, |b| b.move_end(true))),
            )
            .on_action(
                cx.listener(|pad, _: &SelectAll, _, cx| pad.on_select(cx, Buffer::select_all)),
            )
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(|pad, _: &Tab, _, cx| pad.change(cx, Buffer::indent)))
            .on_action(cx.listener(|pad, _: &Undo, _, cx| {
                pad.change(cx, |b| {
                    b.undo();
                })
            }))
            .on_action(cx.listener(|pad, _: &Redo, _, cx| {
                pad.change(cx, |b| {
                    b.redo();
                })
            }))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_reload))
            .on_action(cx.listener(Self::open_prompt))
            .on_action(cx.listener(Self::on_close_file))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::focus_editor))
            .on_action(cx.listener(Self::focus_chat_pane))
            .on_action(cx.listener(Self::focus_input))
            .on_action(cx.listener(Self::cycle_focus))
    }
}

// --- the platform's input methods: typed and composed text ---

impl EntityInputHandler for Pad {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if self.target(window) == Target::Editor {
            let buffer = &self.editor.current()?.buffer;
            let (from, to) = (
                buffer.pos_of_utf16(range.start),
                buffer.pos_of_utf16(range.end),
            );
            *adjusted = Some(buffer.utf16_of(from)..buffer.utf16_of(to));
            return Some(buffer.text_between(from, to));
        }
        let field = self.field_mut(window)?;
        let (text, covered) = field.slice_utf16(range);
        *adjusted = Some(covered);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.target(window) == Target::Editor {
            let buffer = &self.editor.current()?.buffer;
            let (from, to) = buffer
                .selection()
                .unwrap_or((buffer.cursor(), buffer.cursor()));
            return Some(UTF16Selection {
                range: buffer.utf16_of(from)..buffer.utf16_of(to),
                reversed: false,
            });
        }
        let field = self.field_mut(window)?;
        let cursor = field.cursor();
        Some(UTF16Selection {
            range: field.range_to_utf16(&(cursor..cursor)),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        match self.target(window) {
            Target::Editor => {
                let buffer = &self.editor.current()?.buffer;
                let (from, to) = buffer.marked()?;
                Some(buffer.utf16_of(from)..buffer.utf16_of(to))
            }
            Target::Input => self.field.marked().map(|r| self.field.range_to_utf16(&r)),
            Target::Prompt => {
                let field = self.prompt.as_ref()?;
                field.marked().map(|r| field.range_to_utf16(&r))
            }
            Target::Chat => None,
        }
    }

    fn unmark_text(&mut self, window: &mut Window, _: &mut Context<Self>) {
        match self.target(window) {
            Target::Editor => {
                if let Some(doc) = self.editor.current_mut() {
                    doc.buffer.unmark();
                }
            }
            _ => {
                if let Some(field) = self.field_mut(window) {
                    field.unmark();
                }
            }
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.target(window) {
            Target::Editor => {
                let text = text.to_string();
                self.change(cx, |buffer| match range {
                    Some(range) => {
                        let (from, to) = (
                            buffer.pos_of_utf16(range.start),
                            buffer.pos_of_utf16(range.end),
                        );
                        buffer.replace(from, to, &text);
                    }
                    None => buffer.insert(&text),
                });
            }
            Target::Chat => {}
            _ => self.edit_field(window, cx, |field| field.replace_utf16(range, text)),
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.target(window) {
            Target::Editor => {
                let text = text.to_string();
                self.change(cx, |buffer| {
                    let range =
                        range.map(|r| (buffer.pos_of_utf16(r.start), buffer.pos_of_utf16(r.end)));
                    buffer.compose(range, &text);
                });
            }
            Target::Chat => {}
            _ => self.edit_field(window, cx, |field| {
                field.replace_and_mark_utf16(range, text, selected)
            }),
        }
    }

    /// Where the input method puts its candidate window: just under the focused part.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
