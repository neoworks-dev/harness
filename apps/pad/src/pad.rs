//! The Pad window: collapsed while the agent lives elsewhere, a chat pane while it lives here.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, ClickEvent, Context, Entity, EntityInputHandler, FocusHandle, Focusable,
    HighlightStyle, InteractiveElement, IntoElement, KeyBinding, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Render, ScrollHandle, StatefulInteractiveElement, Styled, StyledText,
    UTF16Selection, Window, actions, canvas, div, hsla, px, rgb,
};
use suite_sdk::{Client, Event};

use crate::field::Field;
use crate::focus::{Change, Follower};
use crate::transcript::{Kind, Transcript};

/// This app's name on the bus: its presence, and the `app` of the input it sends.
pub const APP: &str = "pad";

/// Embedded in `main` because GPUI on Linux only knows fonts by exact family name and its
/// default one (IBM Plex Sans) is not generally installed.
pub const FONT: &str = "Geist";

const KEY_CONTEXT: &str = "PadInput";

actions!(
    pad,
    [
        Submit, CancelTurn, Backspace, Delete, Left, Right, Home, End, Paste
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CancelTurn, Some(KEY_CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
        KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
        KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
        KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-a", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("end", End, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-e", End, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(KEY_CONTEXT)),
    ]);
}

/// A bus call the view wants made. They run one after another on a worker (blocking calls
/// belong on a background thread), so a quick open, close, open can't reorder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Subscribe(String),
    Unsubscribe(String),
    Input { session: String, text: String },
    Cancel(String),
    Move { app: Option<&'static str> },
}

impl Op {
    fn run(&self, client: &Client) -> suite_sdk::Result<()> {
        match self {
            Op::Subscribe(session) => client.subscribe(session, 1).map(drop),
            Op::Unsubscribe(session) => client.unsubscribe(session),
            Op::Input { session, text } => client.input(session, APP, text, &[]),
            Op::Cancel(session) => client.cancel(session),
            Op::Move { app } => client.move_agent(*app, None),
        }
    }
}

pub struct Pad {
    ops: async_channel::Sender<Op>,
    follower: Follower,
    /// The app the agent is in right now (`None`: no app can host it, or not known yet).
    agent_at: Option<String>,
    transcript: Transcript,
    field: Field,
    focus: FocusHandle,
    scroll: ScrollHandle,
    connected: bool,
    /// The last failed call, shown until the next focus change.
    notice: Option<String>,
}

impl Pad {
    pub fn new(client: Arc<Client>, cx: &mut Context<Self>) -> Pad {
        let (ops, queue) = async_channel::unbounded::<Op>();

        // Pushed events: focus, session events, connection state.
        let events = client.events();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update(cx, |pad, cx| pad.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

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

        Pad {
            ops,
            follower: Follower::new(APP),
            agent_at: None,
            transcript: Transcript::default(),
            field: Field::default(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            connected: true,
            notice: None,
        }
    }

    fn do_op(&self, op: Op) {
        // Unbounded and the worker outlives the window, so this only fails during shutdown.
        let _ = self.ops.try_send(op);
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Focus(focus) => {
                self.agent_at = focus.app.clone();
                self.notice = None;
                for change in self.follower.on_focus(&focus) {
                    // Both start from an empty transcript: opening replays from seq 1.
                    self.transcript.clear();
                    match change {
                        Change::Open(session) => self.do_op(Op::Subscribe(session)),
                        Change::Close(session) => self.do_op(Op::Unsubscribe(session)),
                    }
                }
            }
            Event::Session(record) => {
                // Events of a session we just left can still be queued.
                if self.follower.pane() == Some(record.session.as_str()) {
                    self.transcript.push(&record.event);
                    self.scroll.scroll_to_bottom();
                }
            }
            Event::Connection(up) => self.connected = up,
            // Pad still draws from raw session events; the GPUI renderer takes over the view.
            Event::Subscribed { .. }
            | Event::Connectors(_)
            | Event::Theme(_)
            | Event::ViewSnapshot(_)
            | Event::ViewPatch(_) => {}
        }
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        self.send_input(cx);
    }

    fn send_input(&mut self, cx: &mut Context<Self>) {
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

    fn cancel_turn(&mut self, _: &CancelTurn, _: &mut Window, _: &mut Context<Self>) {
        // Idle sessions ack and ignore it, so there is no need to check the turn state.
        if let Some(session) = self.follower.pane() {
            self.do_op(Op::Cancel(session.into()));
        }
    }

    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Field)) {
        change(&mut self.field);
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(cx, |field| field.insert(&text));
        }
    }
}

impl Focusable for Pad {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Pad {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let (text, covered) = self.field.slice_utf16(range);
        *adjusted = Some(covered);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let cursor = self.field.cursor();
        Some(UTF16Selection {
            range: self.field.range_to_utf16(&(cursor..cursor)),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.field
            .marked()
            .map(|range| self.field.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.field.unmark();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |field| field.replace_utf16(range, text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |field| {
            field.replace_and_mark_utf16(range, text, selected)
        });
    }

    /// Where the input method puts its candidate window: just under the input line.
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

impl Render for Pad {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = if self.connected {
            String::new()
        } else {
            "daemon connection lost, retrying".to_string()
        };
        let body = if self.follower.pane().is_some() {
            self.pane(window, cx).into_any_element()
        } else {
            self.collapsed(cx).into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xd4d4d4))
            .font_family(FONT)
            .text_size(px(14.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x3c3c3c))
                    .child("Pad")
                    .child(div().text_color(rgb(0xe5a000)).child(status)),
            )
            .child(body)
    }
}

impl Pad {
    /// Shown while the agent is in another app.
    fn collapsed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let place = match &self.agent_at {
            Some(app) => format!("The agent is in {app}."),
            None => "The agent isn't in an app.".to_string(),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(place)
            .child(button("bring-here", "Bring the agent here").on_click(
                cx.listener(|pad, _: &ClickEvent, _, _| pad.do_op(Op::Move { app: Some(APP) })),
            ))
    }

    /// The chat: transcript on top, the input line below.
    fn pane(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notice = self.notice.clone().unwrap_or_default();
        let working = if self.transcript.running() {
            "working…"
        } else {
            ""
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_1()
                    .child(div().text_color(rgb(0x808080)).child(working))
                    .child(button("send-home", "Send home").on_click(
                        cx.listener(|pad, _: &ClickEvent, _, _| pad.do_op(Op::Move { app: None })),
                    )),
            )
            .child(
                div()
                    .id("transcript")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(self.transcript.lines().iter().map(|line| {
                        let colour = match line.kind {
                            Kind::User => rgb(0x9cdcfe),
                            Kind::Agent => rgb(0xd4d4d4),
                            Kind::Tool => rgb(0xce9178),
                            Kind::Note => rgb(0x808080),
                        };
                        div().text_color(colour).child(line.text.clone())
                    })),
            )
            .child(div().px_3().text_color(rgb(0xf48771)).child(notice))
            .child(self.input_line(window, cx))
    }

    fn input_line(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity: Entity<Pad> = cx.entity();
        let focus = self.focus.clone();
        let focused = self.focus.is_focused(window);
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel_turn))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(|pad, _: &Backspace, _, cx| pad.edit(cx, Field::backspace)))
            .on_action(cx.listener(|pad, _: &Delete, _, cx| pad.edit(cx, Field::delete)))
            .on_action(cx.listener(|pad, _: &Left, _, cx| pad.edit(cx, Field::left)))
            .on_action(cx.listener(|pad, _: &Right, _, cx| pad.edit(cx, Field::right)))
            .on_action(cx.listener(|pad, _: &Home, _, cx| pad.edit(cx, Field::home)))
            .on_action(cx.listener(|pad, _: &End, _, cx| pad.edit(cx, Field::end)))
            // Clicking the line focuses it, in case focus was elsewhere when the pane opened.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|pad, _: &MouseDownEvent, window, _| window.focus(&pad.focus)),
            )
            .relative()
            .m_3()
            .px_3()
            .py_2()
            .border_1()
            .border_color(if focused {
                rgb(0x569cd6)
            } else {
                rgb(0x3c3c3c)
            })
            .rounded_md()
            .child(self.input_text(focused))
            .child(
                // Registers this view as the window's input handler while it has focus, so
                // typed and composed text reaches `EntityInputHandler`.
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
    }

    /// The typed text with a block caret on the character under it (a space at the end).
    fn input_text(&self, focused: bool) -> impl IntoElement {
        let caret = HighlightStyle {
            background_color: Some(hsla(0.58, 0.6, 0.6, 0.6)),
            ..Default::default()
        };
        if self.field.text().is_empty() {
            return div()
                .flex()
                .child(StyledText::new(" ").with_highlights(focused.then_some((0..1, caret))))
                .child(div().text_color(rgb(0x808080)).child("Message the agent"))
                .into_any_element();
        }
        let text = self.field.text();
        let at = self.field.cursor();
        let width = text[at..].chars().next().map_or(1, char::len_utf8);
        let shown = if at == text.len() {
            format!("{text} ")
        } else {
            text.to_string()
        };
        StyledText::new(shown)
            .with_highlights(focused.then_some((at..at + width, caret)))
            .into_any_element()
    }
}

fn button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .border_1()
        .border_color(rgb(0x3c3c3c))
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x2a2d2e)))
        .child(label)
}

/// Drives the view against the real daemon (echo agent), headless. Skipped when bun is missing.
#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use gpui::{AppContext as _, TestAppContext};
    use suite_protocol::{AppPresence, ConnectorKind};
    use suite_sdk::Options;

    use super::*;

    struct Daemon {
        child: Child,
        socket: std::path::PathBuf,
        dir: std::path::PathBuf,
    }

    impl Daemon {
        fn start() -> Option<Daemon> {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let dir = std::env::temp_dir().join(format!("suite-pad-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok()?;
            let socket = dir.join("bus.sock");
            let child = Command::new("bun")
                .args(["run", "daemon/src/main.ts"])
                .current_dir(root)
                .env("SUITE_AGENT", "echo")
                .env("SUITE_SOCKET", &socket)
                .env("SUITE_DATA_DIR", dir.join("data"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|err| eprintln!("skipping: cannot run bun ({err})"))
                .ok()?;
            let daemon = Daemon { child, socket, dir };
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
                    return Some(daemon);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("daemon did not start");
        }

        fn client(&self, name: &str, presence: AppPresence) -> Arc<Client> {
            let mut options = Options::new(name);
            options.socket = Some(self.socket.clone());
            options.presence = Some(presence);
            Arc::new(Client::connect(options).expect("connect"))
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn presence(app: &str, home: bool) -> AppPresence {
        AppPresence {
            templates: None,
            app: app.into(),
            kind: ConnectorKind::App,
            title: None,
            open: true,
            context: None,
            mcp: None,
            launch: None,
            home: home.then_some(true),
            pid: None,
        }
    }

    /// Runs the app until `done` holds. Events arrive from the SDK's thread, so the executor
    /// is polled rather than waited on.
    fn wait_until(
        cx: &mut TestAppContext,
        what: &str,
        mut done: impl FnMut(&mut TestAppContext) -> bool,
    ) {
        cx.executor().allow_parking();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[gpui::test]
    fn the_pane_follows_the_agent(cx: &mut TestAppContext) {
        let Some(daemon) = Daemon::start() else {
            return;
        };
        let home = daemon.client("home", presence("test-home", true));
        let client = daemon.client("pad", presence(APP, false));
        let window = cx.add_window(|_, cx| Pad::new(client, cx));
        let pad = window.root(cx).expect("root view");
        let state = |cx: &mut TestAppContext| {
            pad.read_with(cx, |pad, _| {
                (
                    pad.agent_at.clone(),
                    pad.follower.pane().map(String::from),
                    pad.transcript
                        .lines()
                        .iter()
                        .map(|l| l.text.clone())
                        .collect::<Vec<_>>(),
                )
            })
        };

        // Drawing exercises both layouts (not the real fonts, which the test platform lacks).
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                .expect("draw");
        };
        wait_until(cx, "the agent at home", |cx| {
            state(cx).0.as_deref() == Some("test-home")
        });
        draw(cx);
        assert_eq!(state(cx).1, None, "collapsed while the agent is elsewhere");

        // Bring it here, and let the home app say something to it: the pane replays and follows.
        home.move_agent(Some(APP), None).expect("move to pad");
        wait_until(cx, "the pane to open", |cx| state(cx).1.is_some());
        let session = state(cx).1.expect("open");
        home.input(&session, "test-home", "hello", &[])
            .expect("input");
        wait_until(cx, "the echo", |cx| {
            state(cx)
                .2
                .iter()
                .any(|line| line.contains("echo #1: hello"))
        });

        draw(cx);

        // Typing in the pane sends input tagged as pad's.
        pad.update(cx, |pad, cx| {
            pad.field.insert("from pad");
            pad.send_input(cx);
        });
        wait_until(cx, "the second turn", |cx| {
            state(cx)
                .2
                .iter()
                .any(|line| line.contains("echo #2: from pad"))
        });

        // Send it home: the pane collapses and forgets the transcript.
        home.move_agent(None, None).expect("move home");
        wait_until(cx, "the pane to collapse", |cx| state(cx).1.is_none());
        assert!(state(cx).2.is_empty());

        // Back again: a fresh subscription replays everything.
        home.move_agent(Some(APP), None).expect("move to pad again");
        wait_until(cx, "the replay", |cx| {
            let lines = state(cx).2;
            lines.iter().any(|l| l.contains("echo #1: hello"))
                && lines.iter().any(|l| l.contains("echo #2: from pad"))
        });
    }
}
