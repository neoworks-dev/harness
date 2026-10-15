//! The client against the real daemon (`bun run daemon/src/main.ts` with the echo agent).
//! Skipped with a message when bun isn't installed.

mod common;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::{TempDir, wait_for};
use suite_protocol::{
    AgentFocus, AppPresence, ConnectorKind, SessionCreateParams, SessionEvent, StopReason,
};
use suite_sdk::{Client, Event, Options};

struct Daemon {
    child: Child,
    socket: std::path::PathBuf,
    _dir: TempDir,
}

impl Daemon {
    fn start() -> Option<Daemon> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let dir = TempDir::new("daemon");
        let socket = dir.join("bus.sock");
        let child = match Command::new("bun")
            .args(["run", "daemon/src/main.ts"])
            .current_dir(root)
            .env("SUITE_AGENT", "echo")
            .env("SUITE_SOCKET", &socket)
            .env("SUITE_DATA_DIR", dir.join("data"))
            .env("SUITE_PORT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                eprintln!("skipping: cannot run bun ({err})");
                return None;
            }
        };
        let daemon = Daemon {
            child,
            socket,
            _dir: dir,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
                return Some(daemon);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "daemon did not start listening on {}",
            daemon.socket.display()
        );
    }

    fn connect(&self, name: &str, presence: Option<AppPresence>) -> Client {
        let mut options = Options::new(name);
        options.socket = Some(self.socket.clone());
        options.presence = presence;
        Client::connect(options).expect("connect to daemon")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn app(name: &str, home: bool) -> AppPresence {
    AppPresence {
        app: name.into(),
        kind: ConnectorKind::App,
        title: Some(name.into()),
        open: true,
        context: None,
        mcp: None,
        launch: None,
        home: home.then_some(true),
        pid: Some(i64::from(std::process::id())),
    }
}

fn focus_on(events: &async_channel::Receiver<Event>, what: &str, app: Option<&str>) -> AgentFocus {
    wait_for(events, what, |event| match event {
        Event::Focus(focus) if focus.app.as_deref() == app => Some(focus.clone()),
        _ => None,
    })
}

#[test]
fn sessions_echo_presence_and_connectors() {
    let Some(daemon) = Daemon::start() else {
        return;
    };
    let client = daemon.connect("sdk-test", None);
    let events = client.events();
    wait_for(&events, "connection", |e| {
        matches!(e, Event::Connection(true)).then_some(())
    });

    let session = client
        .create_session(SessionCreateParams {
            cwd: Some("/tmp".into()),
            title: Some("sdk test".into()),
        })
        .expect("create session");
    assert!(
        client
            .list_sessions()
            .expect("list")
            .iter()
            .any(|s| s.id == session.id)
    );

    let subscribed = client.subscribe(&session.id, 1).expect("subscribe");
    assert_eq!(subscribed.replayed_through, 0);
    wait_for(&events, "subscribed", |e| {
        matches!(e, Event::Subscribed { .. }).then_some(())
    });

    client
        .input(&session.id, "sdk-test", "hello", &[])
        .expect("input");
    let mut reply = String::new();
    let mut last_seq = 0;
    wait_for(&events, "the echo turn", |event| {
        let Event::Session(record) = event else {
            return None;
        };
        assert_eq!(record.session, session.id);
        assert_eq!(record.seq, last_seq + 1, "gap-free seqs");
        last_seq = record.seq;
        match &record.event {
            SessionEvent::AgentMessageChunk(chunk) => reply.push_str(&chunk.text),
            SessionEvent::TurnEnded(ended) => {
                assert_eq!(ended.stop_reason, StopReason::EndTurn);
                return Some(());
            }
            _ => {}
        }
        None
    });
    assert!(reply.contains("echo #1: hello"), "reply was {reply:?}");

    // Presence shows up in the connector list, and as a change event for others.
    let other = daemon.connect("sdk-watcher", None);
    let watching = other.events();
    client
        .set_presence(app("sdk-test", false))
        .expect("presence");
    let connectors = client.list_connectors().expect("connectors");
    let me = connectors
        .iter()
        .find(|c| c.app == "sdk-test")
        .expect("registered");
    assert!(me.connected);
    wait_for(&watching, "connectors.changed", |event| match event {
        Event::Connectors(list) => list
            .iter()
            .any(|c| c.app == "sdk-test" && c.connected)
            .then_some(()),
        _ => None,
    });

    // Closing the connection unregisters.
    drop(client);
    wait_for(&watching, "connector gone", |event| match event {
        Event::Connectors(list) => list
            .iter()
            .any(|c| c.app == "sdk-test" && !c.connected)
            .then_some(()),
        _ => None,
    });
}

#[test]
fn the_agent_moves_between_apps() {
    let Some(daemon) = Daemon::start() else {
        return;
    };
    let home = daemon.connect("home", Some(app("sdk-home", true)));
    let home_events = home.events();
    let initial = focus_on(&home_events, "focus at the home app", Some("sdk-home"));

    let pad = daemon.connect("pad", Some(app("sdk-pad", false)));
    let pad_events = pad.events();
    // The daemon sends the current focus after every hello.
    focus_on(
        &pad_events,
        "initial focus for a new client",
        Some("sdk-home"),
    );
    wait_for(&pad_events, "presence", |event| match event {
        Event::Connectors(list) => list
            .iter()
            .any(|c| c.app == "sdk-pad" && c.connected)
            .then_some(()),
        _ => None,
    });

    pad.move_agent(Some("sdk-pad"), None).expect("move to pad");
    let moved = focus_on(&pad_events, "focus on pad", Some("sdk-pad"));
    assert_eq!(moved.session, initial.session);
    assert_eq!(moved.from.as_deref(), Some("sdk-home"));
    focus_on(&home_events, "home sees the move", Some("sdk-pad"));

    // A session switch rides along.
    let other = pad
        .create_session(SessionCreateParams {
            cwd: None,
            title: None,
        })
        .expect("create");
    pad.move_agent(None, Some(&other.id))
        .expect("move home with a session");
    let back = focus_on(&home_events, "back at home", Some("sdk-home"));
    assert_eq!(back.session, other.id);

    let error = pad
        .move_agent(Some("nowhere"), None)
        .expect_err("unknown app");
    assert!(matches!(error, suite_sdk::Error::Daemon(_)), "{error:?}");
}
