//! The client against a scripted daemon: framing, reply routing, and what a reconnect restores.

mod common;

use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

use common::{TempDir, wait_for};
use serde_json::{Value, json};
use suite_protocol::{AppPresence, ConnectorKind};
use suite_sdk::{Client, Error, Event, Options};
use tungstenite::{Message, WebSocket};

/// The daemon's side of one connection.
struct Peer(WebSocket<UnixStream>);

impl Peer {
    fn accept(listener: &UnixListener) -> Peer {
        let (stream, _) = listener.accept().expect("accept");
        Peer(tungstenite::accept(stream).expect("websocket handshake"))
    }

    fn recv(&mut self) -> Value {
        loop {
            if let Message::Text(text) = self.0.read().expect("read frame") {
                return serde_json::from_str(text.as_str()).expect("json frame");
            }
        }
    }

    fn expect(&mut self, topic: &str) -> Value {
        let frame = self.recv();
        assert_eq!(frame["topic"], topic, "frame: {frame}");
        frame
    }

    fn send(&mut self, frame: Value) {
        self.0
            .send(Message::text(frame.to_string()))
            .expect("send frame");
    }

    /// Accepts the hello and welcomes the client.
    fn greet(&mut self) {
        let hello = self.expect("bus.hello");
        assert_eq!(hello["payload"]["protocol"], 1);
        self.send(json!({"topic": "bus.welcome", "payload":
            {"protocol": 1, "daemonVersion": "test", "pid": 1, "startedAt": 0}}));
    }

    fn event(&mut self, session: &str, seq: i64, text: &str) {
        self.send(json!({"topic": "session.events", "payload": {
            "session": session, "seq": seq, "ts": 0,
            "event": {"type": "agent_message_chunk", "messageId": "m", "text": text}}}));
    }

    fn subscribed(&mut self, id: &Value, session: &str, last_seq: i64) {
        self.send(json!({"topic": "session.subscribed", "id": id, "payload": {
            "session": session_info(session, last_seq), "replayedThrough": last_seq}}));
    }
}

fn session_info(id: &str, last_seq: i64) -> Value {
    json!({"id": id, "cwd": "/", "createdAt": 0, "lastActivity": 0, "lastSeq": last_seq,
           "state": "idle", "queued": 0})
}

fn setup(label: &str) -> (TempDir, UnixListener, Options) {
    let dir = TempDir::new(label);
    let socket = dir.join("bus.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let mut options = Options::new("test");
    options.socket = Some(socket);
    (dir, listener, options)
}

fn seqs(events: &async_channel::Receiver<Event>, count: usize) -> Vec<i64> {
    let mut seen = Vec::new();
    while seen.len() < count {
        let seq = wait_for(events, "session events", |event| match event {
            Event::Session(record) => Some(record.seq),
            _ => None,
        });
        seen.push(seq);
    }
    seen
}

#[test]
fn routes_replies_and_pushed_events() {
    let (_dir, listener, options) = setup("routes");
    let daemon = thread::spawn(move || {
        let mut peer = Peer::accept(&listener);
        peer.greet();
        peer.send(json!({"topic": "agent.focus", "payload": {"session": "s1", "app": "code-editor", "reason": "initial"}}));

        let list = peer.expect("session.list");
        peer.send(json!({"topic": "session.list_result", "id": list["id"],
            "payload": {"sessions": [session_info("s1", 0)]}}));

        let subscribe = peer.expect("session.subscribe");
        assert_eq!(
            subscribe["payload"],
            json!({"session": "s1"}),
            "fromSeq 1 is the default"
        );
        peer.event("s1", 1, "he");
        peer.event("s1", 2, "llo");
        peer.subscribed(&subscribe["id"], "s1", 2);
        peer.event("s1", 3, "!");
        // Not subscribed: dropped.
        peer.event("other", 1, "x");
        peer.send(json!({"topic": "connectors.changed", "payload": {"connectors": []}}));

        let input = peer.expect("session.input");
        assert_eq!(
            input["payload"],
            json!({"session": "s1", "app": "code-editor", "text": "hi"}),
        );
        peer.send(json!({"topic": "bus.ack", "id": input["id"]}));

        let moved = peer.expect("agent.move");
        assert_eq!(moved["payload"], json!({"app": "code-editor"}));
        peer.send(json!({"topic": "bus.error", "id": moved["id"],
            "payload": {"code": "not_found", "message": "code-editor isn't open"}}));

        peer.expect("session.unsubscribe");
    });

    let client = Client::connect(options).expect("connect");
    let events = client.events();
    assert_eq!(
        wait_for(&events, "connection", |e| matches!(
            e,
            Event::Connection(true)
        )
        .then_some(())),
        ()
    );
    let focus = wait_for(&events, "focus", |event| match event {
        Event::Focus(focus) => Some(focus.clone()),
        _ => None,
    });
    assert_eq!(
        (focus.session.as_str(), focus.app.as_deref()),
        ("s1", Some("code-editor"))
    );

    let sessions = client.list_sessions().expect("list");
    assert_eq!(sessions[0].id, "s1");

    let subscribed = client.subscribe("s1", 0).expect("subscribe");
    assert_eq!(subscribed.replayed_through, 2);
    // Replay, then the marker, then live.
    assert_eq!(seqs(&events, 2), [1, 2]);
    wait_for(&events, "subscribed", |event| {
        matches!(
            event,
            Event::Subscribed {
                replayed_through: 2,
                ..
            }
        )
        .then_some(())
    });
    assert_eq!(seqs(&events, 1), [3]);
    wait_for(&events, "connectors", |e| {
        matches!(e, Event::Connectors(c) if c.is_empty()).then_some(())
    });

    client.input("s1", "code-editor", "hi", &[]).expect("input");
    match client.move_agent(Some("code-editor"), None) {
        Err(Error::Daemon(info)) => assert_eq!(info.message, "code-editor isn't open"),
        other => panic!("expected the daemon's error, got {other:?}"),
    }
    client
        .unsubscribe("s1")
        .expect_err("daemon never acks: connection ends");
    daemon.join().expect("daemon script");
}

#[test]
fn without_reconnect_a_dropped_connection_ends_the_client() {
    let (_dir, listener, options) = setup("dropped");
    let daemon = thread::spawn(move || {
        let mut peer = Peer::accept(&listener);
        peer.greet();
        peer.expect("session.list");
        // Dropped without a reply.
    });
    let client = Client::connect(options).expect("connect");
    let events = client.events();
    assert!(matches!(client.list_sessions(), Err(Error::Disconnected)));
    wait_for(&events, "connection down", |e| {
        matches!(e, Event::Connection(false)).then_some(())
    });
    daemon.join().unwrap();
    assert!(matches!(client.list_sessions(), Err(Error::Disconnected)));
    // The queue ends once the thread has.
    assert!(wait_closed(&events));
}

fn wait_closed(events: &async_channel::Receiver<Event>) -> bool {
    let deadline = std::time::Instant::now() + common::WAIT;
    while std::time::Instant::now() < deadline {
        match events.try_recv() {
            Err(async_channel::TryRecvError::Closed) => return true,
            _ => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    false
}

#[test]
fn daemon_down_is_reported() {
    let dir = TempDir::new("down");
    let mut options = Options::new("test");
    options.socket = Some(dir.join("missing.sock"));
    assert!(matches!(Client::connect(options), Err(Error::DaemonDown)));
}

#[test]
fn reconnect_restores_presence_and_resumes_without_gaps_or_repeats() {
    let (_dir, listener, mut options) = setup("reconnect");
    options.reconnect = true;
    options.presence = Some(AppPresence {
        description: None,
        templates: None,
        app: "code-editor".into(),
        kind: ConnectorKind::App,
        title: Some("Pad".into()),
        open: true,
        context: None,
        mcp: None,
        launch: None,
        home: None,
        pid: Some(42),
    });
    let daemon = thread::spawn(move || {
        let mut peer = Peer::accept(&listener);
        peer.greet();
        let presence = peer.expect("app.presence");
        assert_eq!(presence["payload"]["pid"], 42);
        peer.send(json!({"topic": "bus.ack", "id": presence["id"]}));
        let subscribe = peer.expect("session.subscribe");
        peer.event("s1", 1, "a");
        peer.event("s1", 2, "b");
        peer.subscribed(&subscribe["id"], "s1", 2);
        peer.event("s1", 3, "c");
        // Wait until the client has the three events, then cut it off.
        let sync = peer.expect("session.list");
        peer.send(
            json!({"topic": "session.list_result", "id": sync["id"], "payload": {"sessions": []}}),
        );
        drop(peer);

        let mut peer = Peer::accept(&listener);
        peer.greet();
        peer.send(
            json!({"topic": "agent.focus", "payload": {"session": "s1", "reason": "initial"}}),
        );
        let presence = peer.expect("app.presence");
        assert_eq!(presence["payload"]["app"], "code-editor");
        let resume = peer.expect("session.subscribe");
        assert_eq!(resume["payload"], json!({"session": "s1", "fromSeq": 4}));
        // A daemon that replays too much must not cause repeats.
        peer.event("s1", 3, "c");
        peer.event("s1", 4, "d");
        peer.subscribed(&resume["id"], "s1", 4);
        peer.event("s1", 5, "e");
        let sync = peer.expect("session.list");
        peer.send(
            json!({"topic": "session.list_result", "id": sync["id"], "payload": {"sessions": []}}),
        );
    });

    let client = Client::connect(options).expect("connect");
    let events = client.events();
    client.subscribe("s1", 1).expect("subscribe");
    assert_eq!(seqs(&events, 3), [1, 2, 3]);
    client.list_sessions().expect("sync");
    wait_for(&events, "connection down", |e| {
        matches!(e, Event::Connection(false)).then_some(())
    });
    wait_for(&events, "connection back", |e| {
        matches!(e, Event::Connection(true)).then_some(())
    });
    wait_for(&events, "focus after hello", |e| {
        matches!(e, Event::Focus(_)).then_some(())
    });
    assert_eq!(seqs(&events, 2), [4, 5]);
    // The client is usable again after the reconnect.
    client.list_sessions().expect("list after reconnect");
    daemon.join().expect("daemon script");
}
