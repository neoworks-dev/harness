//! Drives the window against the real daemon (echo agent, no background services), headless.
//! Skipped when bun is missing. A stand-in "lsp" connector (a Rust tool server) answers the
//! diagnostics calls.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use suite_protocol::{AppPresence, ConnectorKind};
use suite_sdk::tools::{Tool, ToolResult, ToolServer};
use suite_sdk::{Client, Event, Options};

use crate::buffer::Pos;
use crate::editor::{DiagState, Severity};
use crate::gateway::{Gateway, ToolOutput};
use crate::pad::*;
use crate::tools;

struct Daemon {
    child: Child,
    socket: PathBuf,
    dir: PathBuf,
}

impl Daemon {
    fn start(label: &str) -> Option<Daemon> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("suite-pad-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let socket = dir.join("bus.sock");
        let child = Command::new("bun")
            .args(["run", "daemon/src/main.ts"])
            .current_dir(root)
            .env("SUITE_AGENT", "echo")
            .env("SUITE_SERVICES", "none")
            .env("SUITE_SOCKET", &socket)
            .env("SUITE_DATA_DIR", dir.join("data"))
            .env("SUITE_PORT", "0")
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

    fn client(&self, name: &str, presence: AppPresence) -> std::sync::Arc<Client> {
        let mut options = Options::new(name);
        options.socket = Some(self.socket.clone());
        options.presence = Some(presence);
        std::sync::Arc::new(Client::connect(options).expect("connect"))
    }

    fn gateway(&self) -> Gateway {
        Gateway::beside(&self.socket)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn app_presence(app: &str, home: bool) -> AppPresence {
    AppPresence {
        description: None,
        templates: None,
        components: None,
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

/// A stand-in for the lsp service: `diagnostics` reports one error on line 2 of any file.
fn fake_lsp(daemon: &Daemon) -> (ToolServer, std::sync::Arc<Client>) {
    let diagnostics = Tool::new(
        "diagnostics",
        "Errors and warnings",
        json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
        |args| {
            ToolResult::structured(
                "problems",
                json!({"path": args["path"], "errors": 1, "warnings": 0, "diagnostics": [
                    {"line": 2, "column": 7, "severity": "error", "message": "Cannot find name 'missing'.", "source": "fake"}
                ]}),
            )
        },
    );
    let server = ToolServer::start("lsp", None, vec![diagnostics]).expect("lsp tool server");
    let mut presence = app_presence("lsp", false);
    presence.kind = ConnectorKind::Service;
    presence.mcp = Some(server.endpoint());
    let client = daemon.client("lsp", presence);
    (server, client)
}

struct Rig {
    daemon: Daemon,
    /// The window's client.
    pad_client: std::sync::Arc<Client>,
    setup: Option<Setup>,
}

impl Rig {
    fn new(label: &str, open: Vec<(PathBuf, Option<usize>)>) -> Option<Rig> {
        let daemon = Daemon::start(label)?;
        let (server, requests) = tools::serve().expect("tool server");
        let endpoint = server.endpoint();
        let pad_client = daemon.client(APP, presence(true, Some(endpoint.clone()), None));
        let setup = Setup {
            socket: daemon.socket.clone(),
            requests,
            server: Some(server),
            open,
        };
        Some(Rig {
            daemon,
            pad_client,
            setup: Some(setup),
        })
    }

    fn window(&mut self, cx: &mut TestAppContext) -> (gpui::WindowHandle<Pad>, gpui::Entity<Pad>) {
        let client = self.pad_client.clone();
        let setup = self.setup.take().expect("one window per rig");
        let window = cx.add_window(|window, cx| Pad::new(client, setup, window, cx));
        let pad = window.root(cx).expect("root view");
        (window, pad)
    }
}

/// Runs the app until `done` holds. Events arrive from other threads, so the executor is
/// polled rather than waited on.
fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    cx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Calls a connector tool through the daemon's gateway, from a thread (the call blocks while the
/// window, on this thread, has to answer).
fn call_tool(
    gateway: &Gateway,
    name: &str,
    args: Value,
) -> mpsc::Receiver<Result<ToolOutput, String>> {
    let (send, receive) = mpsc::channel();
    let (gateway, name) = (gateway.clone(), name.to_string());
    std::thread::spawn(move || {
        let _ = send.send(gateway.call_tool(&name, args));
    });
    receive
}

fn answer(
    cx: &mut TestAppContext,
    what: &str,
    receive: mpsc::Receiver<Result<ToolOutput, String>>,
) -> ToolOutput {
    let mut got = None;
    wait_until(cx, what, |_| {
        got = receive.try_recv().ok();
        got.is_some()
    });
    got.expect("answered").expect("tool call")
}

fn draw(cx: &mut TestAppContext, window: gpui::WindowHandle<Pad>) {
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .expect("draw");
}

fn scratch(daemon: &Daemon) -> PathBuf {
    let dir = daemon.dir.join("site");
    std::fs::create_dir_all(&dir).expect("site dir");
    dir
}

#[gpui::test]
fn the_pane_follows_the_agent(cx: &mut TestAppContext) {
    let Some(mut rig) = Rig::new("follow", Vec::new()) else {
        return;
    };
    let home = rig.daemon.client("home", app_presence("test-home", true));
    let (window, pad) = rig.window(cx);
    let state = |cx: &mut TestAppContext| {
        pad.read_with(cx, |pad, cx| {
            (
                pad.agent_at.clone(),
                pad.follower.pane().map(String::from),
                // The chat as the layout model's text, one entry per line.
                pad.chat.read_with(cx, |chat, _| {
                    if chat.items().is_empty() {
                        Vec::new()
                    } else {
                        chat.dump().lines().map(String::from).collect::<Vec<_>>()
                    }
                }),
            )
        })
    };

    // Drawing exercises both layouts (not the real fonts, which the test platform lacks).
    wait_until(cx, "the agent at home", |cx| {
        state(cx).0.as_deref() == Some("test-home")
    });
    draw(cx, window);
    assert_eq!(state(cx).1, None, "collapsed while the agent is elsewhere");

    // Bring it here, and say something from the terminal (typing in another app would move the
    // agent there): the pane replays and follows.
    home.move_agent(Some(APP), None).expect("move here");
    wait_until(cx, "the pane to open", |cx| state(cx).1.is_some());
    let session = state(cx).1.expect("open");
    home.input(&session, "cli", "hello", &[]).expect("input");
    wait_until(cx, "the echo", |cx| {
        state(cx)
            .2
            .iter()
            .any(|line| line.contains("echo #1: hello"))
    });

    draw(cx, window);
    // The user's input is a bubble headed with where it came from.
    assert!(
        state(cx).2.iter().any(|line| line.contains("\"» cli\"")),
        "{:?}",
        state(cx).2
    );

    // Typing in the pane sends input tagged as the code editor's.
    pad.update(cx, |pad, cx| {
        pad.field.insert("from the editor");
        pad.send_input(cx);
    });
    wait_until(cx, "the second turn", |cx| {
        state(cx)
            .2
            .iter()
            .any(|line| line.contains("echo #2: from the editor"))
    });
    assert!(
        state(cx)
            .2
            .iter()
            .any(|line| line.contains("\"» code-editor\"")),
        "{:?}",
        state(cx).2
    );

    // Send it home: the pane collapses and forgets the chat.
    home.move_agent(None, None).expect("move home");
    wait_until(cx, "the pane to collapse", |cx| state(cx).1.is_none());
    assert!(state(cx).2.is_empty());

    // Back again: a fresh subscription replays everything.
    home.move_agent(Some(APP), None).expect("move here again");
    wait_until(cx, "the replay", |cx| {
        let lines = state(cx).2;
        lines.iter().any(|l| l.contains("echo #1: hello"))
            && lines.iter().any(|l| l.contains("echo #2: from the editor"))
    });
}

#[gpui::test]
fn the_agent_drives_the_editor_through_the_gateway(cx: &mut TestAppContext) {
    let Some(mut rig) = Rig::new("tools", Vec::new()) else {
        return;
    };
    let site = scratch(&rig.daemon);
    let header = site.join("Header.tsx");
    let other = site.join("notes.txt");
    std::fs::write(
        &header,
        "export const Header = () => (\n  <Logo missing />\n);\n",
    )
    .unwrap();
    std::fs::write(&other, "alpha\nbeta\ngamma\n").unwrap();

    let (_lsp_server, _lsp) = fake_lsp(&rig.daemon);
    let home = rig.daemon.client("home", app_presence("test-home", true));
    let home_events = home.events();
    let (window, pad) = rig.window(cx);
    let gateway = rig.daemon.gateway();
    let current = |cx: &mut TestAppContext| {
        pad.read_with(cx, |pad, _| {
            pad.editor().current().map(|doc| {
                (
                    doc.path().to_path_buf(),
                    doc.buffer.cursor(),
                    doc.diag_state.clone(),
                    doc.highlight,
                )
            })
        })
    };

    // The connector lists the four tools.
    wait_until(cx, "the connector", |_| {
        rig.pad_client
            .list_connectors()
            .expect("list")
            .iter()
            .any(|c| c.app == APP && c.tools.len() == 4)
    });
    let listed = rig.pad_client.list_connectors().expect("list");
    let me = listed.iter().find(|c| c.app == APP).expect("listed");
    let mut names: Vec<&str> = me.tools.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "code-editor.files",
            "code-editor.open",
            "code-editor.reveal",
            "code-editor.selection"
        ]
    );
    assert_eq!(me.error, None);

    // Nothing open yet.
    let out = answer(
        cx,
        "selection",
        call_tool(&gateway, "code-editor.selection", json!({})),
    );
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.structured.expect("data")["open"], false);

    // open: the file, its line count and language; diagnostics arrive from the lsp stand-in.
    let out = answer(
        cx,
        "open",
        call_tool(
            &gateway,
            "code-editor.open",
            json!({"path": header, "line": 2}),
        ),
    );
    assert!(!out.is_error, "{}", out.text);
    let data = out.structured.expect("data");
    assert_eq!(data["path"], header.display().to_string());
    assert_eq!(data["lines"], 3);
    assert_eq!(data["language"], "typescriptreact");
    assert_eq!(current(cx).expect("open").1, Pos::new(1, 0));
    wait_until(cx, "diagnostics", |cx| {
        current(cx).is_some_and(|c| c.2 == DiagState::Done)
    });
    pad.read_with(cx, |pad, _| {
        let doc = pad.editor().current().expect("doc");
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(doc.diagnostics[0].severity, Severity::Error);
        assert_eq!(doc.worst_on(1), Some(Severity::Error));
    });
    draw(cx, window);

    // The presence context follows the current file and line.
    wait_until(cx, "the context", |_| {
        rig.pad_client
            .list_connectors()
            .expect("list")
            .iter()
            .any(|c| {
                c.app == APP
                    && c.context.as_ref().is_some_and(|ctx| {
                        ctx.get("file") == Some(&header.display().to_string())
                            && ctx.get("line").map(String::as_str) == Some("2")
                    })
            })
    });

    // reveal: another file, cursor, highlight.
    let out = answer(
        cx,
        "reveal",
        call_tool(
            &gateway,
            "code-editor.reveal",
            json!({"path": other, "line": 3, "column": 2}),
        ),
    );
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.structured.expect("data")["text"], "gamma");
    let (path, cursor, _, highlight) = current(cx).expect("open");
    assert_eq!(
        (path, cursor, highlight),
        (other.clone(), Pos::new(2, 1), Some(2))
    );
    draw(cx, window);
    wait_until(cx, "the new line in the context", |_| {
        rig.pad_client
            .list_connectors()
            .expect("list")
            .iter()
            .any(|c| {
                c.app == APP
                    && c.context
                        .as_ref()
                        .is_some_and(|ctx| ctx.get("line").map(String::as_str) == Some("3"))
            })
    });

    // selection: what the user selected, so the agent can act on "this".
    pad.update(cx, |pad, cx| {
        let doc = pad.editor_mut_for_test().current_mut().expect("doc");
        doc.buffer.set_cursor(Pos::new(0, 2), false);
        doc.buffer.set_cursor(Pos::new(1, 2), true);
        cx.notify();
    });
    let out = answer(
        cx,
        "selection",
        call_tool(&gateway, "code-editor.selection", json!({})),
    );
    let data = out.structured.expect("data");
    assert_eq!(data["text"], "pha\nbe");
    assert_eq!(data["path"], other.display().to_string());
    assert!(out.text.contains("pha\nbe"), "{}", out.text);

    // files: both, the current one marked, dirty state.
    pad.update(cx, |pad, _| {
        let buffer = &mut pad.editor_mut_for_test().current_mut().expect("doc").buffer;
        buffer.set_cursor(Pos::new(0, 5), false);
        buffer.insert("!");
    });
    let out = answer(
        cx,
        "files",
        call_tool(&gateway, "code-editor.files", json!({})),
    );
    let files = out.structured.expect("data")["files"].clone();
    assert_eq!(files[0]["path"], header.display().to_string());
    assert_eq!(files[0]["dirty"], false);
    assert_eq!(files[1]["current"], true);
    assert_eq!(files[1]["dirty"], true);

    // Bad arguments are tool errors, not crashes.
    let out = answer(
        cx,
        "bad open",
        call_tool(
            &gateway,
            "code-editor.open",
            json!({"path": site.join("nope.txt")}),
        ),
    );
    assert!(out.is_error, "{}", out.text);

    // The agent edits a file that is open: the editor picks it up. The daemon sends file.changed
    // for the agent's writes; here another app announces it.
    std::fs::write(&header, "export const Header = () => (\n  <Logo />\n);\n").unwrap();
    home.file_changed(&header.display().to_string())
        .expect("file.changed");
    wait_until(cx, "the reload", |cx| {
        pad.read_with(cx, |pad, _| {
            pad.editor()
                .docs()
                .first()
                .is_some_and(|d| d.buffer.line(1) == "  <Logo />")
        })
    });
    // ... except where the user has unsaved edits: that is flagged.
    std::fs::write(&other, "changed elsewhere\n").unwrap();
    home.file_changed(&other.display().to_string())
        .expect("file.changed");
    wait_until(cx, "the conflict flag", |cx| {
        pad.read_with(cx, |pad, _| {
            pad.editor()
                .docs()
                .get(1)
                .is_some_and(|d| d.changed_on_disk)
        })
    });
    draw(cx, window);

    // Saving writes the file and announces it to the other apps.
    pad.update(cx, |pad, cx| {
        pad.save_for_test(cx);
    });
    assert_eq!(
        std::fs::read_to_string(&other).unwrap(),
        "alpha!\nbeta\ngamma\n"
    );
    let mut announced = false;
    wait_until(cx, "file.changed from the editor", |_| {
        while let Ok(event) = home_events.try_recv() {
            if let Event::FileChanged(changed) = event
                && changed.path == other.display().to_string()
            {
                announced = true;
            }
        }
        announced
    });
}

#[gpui::test]
fn keys_type_save_and_move_focus(cx: &mut TestAppContext) {
    let Some(mut rig) = Rig::new("keys", Vec::new()) else {
        return;
    };
    let site = scratch(&rig.daemon);
    let file = site.join("a.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let home = rig.daemon.client("home", app_presence("test-home", true));
    let home_events = home.events();
    let (window, pad) = rig.window(cx);
    cx.update(bind_keys);
    wait_until(cx, "the agent at home", |cx| {
        pad.read_with(cx, |pad, _| pad.agent_at.as_deref() == Some("test-home"))
    });
    // The chat pane only exists while the agent is here.
    home.move_agent(Some(APP), None).expect("move here");
    wait_until(cx, "the pane to open", |cx| {
        pad.read_with(cx, |pad, _| pad.follower.pane().is_some())
    });
    pad.update(cx, |pad, cx| pad.open_path(&file, None, cx));
    draw(cx, window);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    pad.update_in(&mut vcx, |pad, window, _| window.focus(&pad.editor_focus));
    draw_visual(&mut vcx);

    // No lsp service in this daemon: the problems panel says so instead of failing.
    wait_until_visual(&mut vcx, "the diagnostics answer", |vcx| {
        pad.read_with(vcx, |pad, _| {
            matches!(
                pad.editor().current().map(|d| &d.diag_state),
                Some(DiagState::Unavailable(why)) if why.contains("lsp")
            )
        })
    });
    draw_visual(&mut vcx);

    let target =
        |vcx: &mut VisualTestContext| pad.update_in(vcx, |pad, window, _| pad.target(window));
    assert_eq!(target(&mut vcx), Target::Editor);
    let text = |vcx: &mut VisualTestContext| {
        pad.read_with(vcx, |pad, _| {
            pad.editor().current().expect("doc").buffer.file_text()
        })
    };

    // Move, type, split a line, undo.
    vcx.simulate_keystrokes("ctrl-end");
    vcx.simulate_input("X");
    assert_eq!(text(&mut vcx), "one\ntwoX\n");
    vcx.simulate_keystrokes("enter");
    vcx.simulate_input("y");
    assert_eq!(text(&mut vcx), "one\ntwoX\ny\n");
    vcx.simulate_keystrokes("ctrl-z");
    assert_eq!(
        text(&mut vcx),
        "one\ntwoX\n\n",
        "typing after the newline is one step"
    );
    vcx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&mut vcx), "one\ntwoX\n");
    vcx.simulate_keystrokes("up home shift-down");
    pad.read_with(&vcx, |pad, _| {
        let selected = pad
            .editor()
            .current()
            .and_then(|d| d.buffer.selected_text());
        assert_eq!(selected.as_deref(), Some("one\n"));
    });
    vcx.simulate_keystrokes("backspace");
    assert_eq!(text(&mut vcx), "twoX\n");
    pad.read_with(&vcx, |pad, _| {
        assert!(pad.editor().current().expect("doc").dirty())
    });

    // ctrl-s saves, and the other apps hear of it.
    vcx.simulate_keystrokes("ctrl-s");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "twoX\n");
    pad.read_with(&vcx, |pad, _| {
        assert!(!pad.editor().current().expect("doc").dirty())
    });
    let mut announced = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !announced && Instant::now() < deadline {
        vcx.run_until_parked();
        while let Ok(event) = home_events.try_recv() {
            if matches!(event, Event::FileChanged(c) if c.path == file.display().to_string()) {
                announced = true;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(announced, "file.changed after the save");

    // Focus: editor, chat pane, input, back; F6 cycles.
    vcx.simulate_keystrokes("ctrl-3");
    assert_eq!(target(&mut vcx), Target::Input);
    vcx.simulate_input("hi");
    pad.read_with(&vcx, |pad, _| assert_eq!(pad.field.text(), "hi"));
    vcx.simulate_keystrokes("ctrl-2");
    assert_eq!(target(&mut vcx), Target::Chat);
    vcx.simulate_keystrokes("ctrl-1");
    assert_eq!(target(&mut vcx), Target::Editor);
    vcx.simulate_keystrokes("f6");
    assert_eq!(target(&mut vcx), Target::Chat);
    vcx.simulate_keystrokes("f6");
    assert_eq!(target(&mut vcx), Target::Input);
    vcx.simulate_keystrokes("f6");
    assert_eq!(target(&mut vcx), Target::Editor);

    // ctrl-o opens the prompt, enter opens what was typed (relative to nothing: absolute here).
    let second = site.join("b.txt");
    std::fs::write(&second, "bee\n").unwrap();
    vcx.simulate_keystrokes("ctrl-o");
    draw_visual(&mut vcx);
    assert_eq!(target(&mut vcx), Target::Prompt);
    // The prompt starts in the current file's directory.
    vcx.simulate_input("b.txt:1");
    vcx.simulate_keystrokes("enter");
    assert_eq!(target(&mut vcx), Target::Editor);
    pad.read_with(&vcx, |pad, _| {
        assert_eq!(pad.editor().docs().len(), 2);
        assert_eq!(pad.editor().current().expect("doc").path(), second);
    });
    // escape closes the prompt without opening anything.
    vcx.simulate_keystrokes("ctrl-o");
    draw_visual(&mut vcx);
    vcx.simulate_keystrokes("escape");
    assert_eq!(target(&mut vcx), Target::Editor);
    pad.read_with(&vcx, |pad, _| assert_eq!(pad.editor().docs().len(), 2));

    // ctrl-w: unsaved edits survive the first try.
    vcx.simulate_input("!");
    vcx.simulate_keystrokes("ctrl-w");
    pad.read_with(&vcx, |pad, _| {
        assert_eq!(pad.editor().docs().len(), 2);
        assert!(pad.notice.as_deref().unwrap_or("").contains("Unsaved"));
    });
    vcx.simulate_keystrokes("ctrl-w");
    pad.read_with(&vcx, |pad, _| assert_eq!(pad.editor().docs().len(), 1));
}

fn wait_until_visual(
    vcx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    vcx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        vcx.run_until_parked();
        if done(vcx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn draw_visual(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn asset_images_in_the_chat_resolve_through_the_daemon(cx: &mut TestAppContext) {
    let Some(mut rig) = Rig::new("assets", Vec::new()) else {
        return;
    };
    let site = scratch(&rig.daemon);
    let logo = site.join("logo.png");
    std::fs::write(&logo, b"not really a png").unwrap();
    let home = rig.daemon.client("home", app_presence("test-home", true));
    let stored = home
        .put_asset(suite_protocol::AssetPut {
            path: Some(logo.display().to_string()),
            data: None,
            name: None,
            mime: None,
        })
        .expect("put asset");
    assert!(stored.handle.starts_with("asset://"));

    let (window, pad) = rig.window(cx);
    let chat = pad.read_with(cx, |pad, _| pad.chat.clone());
    let snapshot: suite_protocol::ViewSnapshot = serde_json::from_value(json!({
        "session": "s", "seq": 1, "items": [{"ref": "i", "kind": "tool", "label": "x", "state": "done",
            "component": {"type": "card", "children": [
                {"type": "image", "src": stored.handle, "width": 40, "height": 20}]}}]
    }))
    .expect("snapshot");
    chat.update(cx, |chat, cx| chat.apply_snapshot(snapshot, cx));
    draw(cx, window);
    wait_until(cx, "the asset to resolve", |cx| {
        chat.read_with(cx, |chat, _| chat.asset_path(&stored.handle).is_some())
    });
    let path = chat.read_with(cx, |chat, _| {
        chat.asset_path(&stored.handle).map(PathBuf::from)
    });
    assert_eq!(path.as_deref(), Some(Path::new(&stored.path)));
    draw(cx, window);
}
