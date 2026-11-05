//! The IO thread: owns the WebSocket, matches replies to requests, tracks what to restore after
//! a reconnect, and turns pushed frames into [`Event`]s.

use std::collections::HashMap;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use neo_protocol::{
    AppPresence, AppPresenceMessage, BusHello, Envelope, Hello, SessionSubscribe, SubscribeParams,
    ViewSubscribe, ViewSubscribeParams,
};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

use crate::{Error, Event, PROTOCOL_VERSION, Result};

pub(crate) type Socket = WebSocket<UnixStream>;

/// How long the thread waits for a frame before looking at the command queue again. This is
/// also the latency of an outgoing request.
const POLL: Duration = Duration::from_millis(20);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_BACKOFF: Duration = Duration::from_millis(50);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

pub(crate) enum Command {
    Request(Box<Request>),
    /// A frame without a reply (`file.changed`); dropped while disconnected.
    Send(Box<Envelope>),
    Close,
}

pub(crate) struct Request {
    pub envelope: Envelope,
    pub reply: mpsc::Sender<Result<Envelope>>,
    pub effect: Effect,
}

/// What a request changes in the state kept for reconnecting. One per request, so the size of
/// the presence variant doesn't matter.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Effect {
    None,
    Subscribe {
        session: String,
        from_seq: i64,
    },
    Unsubscribe(String),
    Presence(AppPresence),
    /// A new context for the registered presence (kept for reconnects).
    Context(std::collections::BTreeMap<String, String>),
    /// The view, and the tail its snapshot was asked with.
    ViewSubscribe(ViewKey, Option<i64>),
    ViewUnsubscribe(ViewKey),
}

/// A followed view: the session, and the agent for a per-agent view.
pub(crate) type ViewKey = (String, Option<String>);

/// How to take an effect back when the daemon refuses the request.
#[allow(clippy::large_enum_variant)]
enum Undo {
    None,
    Unsubscribe(String),
    Presence(Option<AppPresence>),
    ViewUnsubscribe(ViewKey),
}

struct Pending {
    /// Absent for requests the thread sent on its own (restoring state).
    reply: Option<mpsc::Sender<Result<Envelope>>>,
    undo: Undo,
}

pub(crate) struct Target {
    pub socket: PathBuf,
    pub client: String,
    pub reconnect: bool,
}

pub(crate) struct Io {
    target: Target,
    commands: mpsc::Receiver<Command>,
    events: async_channel::Sender<Event>,
    pending: HashMap<String, Pending>,
    /// Subscribed sessions and the last seq seen of each. Replays after a resubscription
    /// start at the next one; anything at or below it is dropped as a repeat.
    subs: HashMap<String, i64>,
    /// Sessions whose view is followed, with the tail asked for; resubscribed (fresh snapshot)
    /// after a reconnect.
    views: HashMap<ViewKey, Option<i64>>,
    presence: Option<AppPresence>,
    next_internal: u64,
}

enum Flow {
    Continue,
    /// The connection is gone.
    Lost,
    /// The client was closed (or dropped).
    Closed,
}

/// Opens the socket, upgrades to WebSocket at `/bus` and exchanges hello/welcome.
pub(crate) fn dial(path: &Path, client: &str) -> Result<Socket> {
    let stream = UnixStream::connect(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => Error::DaemonDown,
        _ => Error::Io(err),
    })?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    // The host is not used for routing; it only has to be well-formed.
    let request = "ws://neo/bus"
        .into_client_request()
        .map_err(|err| Error::Connect(err.to_string()))?;
    let (mut ws, _) =
        tungstenite::client(request, stream).map_err(|err| Error::Connect(err.to_string()))?;

    let hello = Envelope::BusHello(BusHello {
        payload: Hello {
            protocol: PROTOCOL_VERSION,
            client: client.into(),
        },
    });
    send(&mut ws, &hello).map_err(|_| Error::Disconnected)?;
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => match serde_json::from_str(text.as_str()) {
                Ok(Envelope::BusWelcome(_)) => break,
                Ok(Envelope::BusError(error)) => return Err(Error::Daemon(error.payload)),
                _ => {}
            },
            Ok(_) => {}
            Err(err) if is_timeout(&err) => return Err(Error::Timeout),
            Err(_) => return Err(Error::Disconnected),
        }
    }
    ws.get_ref().set_read_timeout(Some(POLL))?;
    Ok(ws)
}

fn send(ws: &mut Socket, envelope: &Envelope) -> std::result::Result<(), tungstenite::Error> {
    let text = serde_json::to_string(envelope).expect("envelopes serialize");
    ws.send(Message::text(text))
}

fn is_timeout(err: &tungstenite::Error) -> bool {
    matches!(err, tungstenite::Error::Io(io)
        if matches!(io.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
}

/// The request id a daemon reply answers.
fn reply_id(envelope: &Envelope) -> Option<&str> {
    match envelope {
        Envelope::BusAck(m) => Some(&m.id),
        Envelope::BusError(m) => m.id.as_deref(),
        Envelope::SessionInfoMessage(m) => Some(&m.id),
        Envelope::SessionListResult(m) => Some(&m.id),
        Envelope::SessionSubscribed(m) => Some(&m.id),
        Envelope::SessionUnqueued(m) => Some(&m.id),
        Envelope::ConnectorsListResult(m) => Some(&m.id),
        Envelope::ViewSnapshotMessage(m) => Some(&m.id),
        Envelope::ViewPageResult(m) => Some(&m.id),
        Envelope::AssetInfoMessage(m) => Some(&m.id),
        Envelope::SpeechTextMessage(m) => Some(&m.id),
        _ => None,
    }
}

impl Io {
    pub fn new(
        target: Target,
        commands: mpsc::Receiver<Command>,
        events: async_channel::Sender<Event>,
        presence: Option<AppPresence>,
    ) -> Io {
        Io {
            target,
            commands,
            events,
            pending: HashMap::new(),
            subs: HashMap::new(),
            views: HashMap::new(),
            presence,
            next_internal: 0,
        }
    }

    /// Serves `socket` until the client is closed or, without `reconnect`, the connection ends.
    pub fn run(mut self, mut socket: Socket) {
        loop {
            self.emit(Event::Connection(true));
            self.restore(&mut socket);
            let flow = self.serve(&mut socket);
            if matches!(flow, Flow::Closed) {
                // Best effort: the daemon drops this client's presence either way.
                let _ = socket.close(None);
                let _ = socket.flush();
                return;
            }
            self.fail_pending();
            self.emit(Event::Connection(false));
            if !self.target.reconnect {
                return;
            }
            match self.redial() {
                Some(next) => socket = next,
                None => return,
            }
        }
    }

    fn emit(&self, event: Event) {
        // Unbounded, and the Client keeps a receiver alive, so this cannot fail or block.
        let _ = self.events.try_send(event);
    }

    fn serve(&mut self, socket: &mut Socket) -> Flow {
        loop {
            loop {
                match self.commands.try_recv() {
                    Ok(Command::Request(request)) => {
                        if let Flow::Lost = self.send_request(socket, request) {
                            return Flow::Lost;
                        }
                    }
                    Ok(Command::Send(envelope)) => {
                        if send(socket, &envelope).is_err() {
                            return Flow::Lost;
                        }
                    }
                    Ok(Command::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                        return Flow::Closed;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            match socket.read() {
                Ok(Message::Text(text)) => self.on_frame(text.as_str()),
                Ok(_) => {} // pings are answered by tungstenite itself
                Err(err) if is_timeout(&err) => {}
                Err(_) => return Flow::Lost,
            }
        }
    }

    fn send_request(&mut self, socket: &mut Socket, request: Box<Request>) -> Flow {
        let Request {
            envelope,
            reply,
            effect,
        } = *request;
        let undo = self.apply(effect);
        let Some(id) = request_id(&envelope) else {
            return Flow::Continue; // every request the client builds has one
        };
        if send(socket, &envelope).is_err() {
            let _ = reply.send(Err(Error::Disconnected));
            return Flow::Lost;
        }
        self.pending.insert(
            id,
            Pending {
                reply: Some(reply),
                undo,
            },
        );
        Flow::Continue
    }

    /// Records what a request changes for reconnecting, before it is sent: its replayed events
    /// can arrive ahead of its reply.
    fn apply(&mut self, effect: Effect) -> Undo {
        match effect {
            Effect::None => Undo::None,
            Effect::Subscribe { session, from_seq } => {
                self.subs.insert(session.clone(), from_seq - 1);
                Undo::Unsubscribe(session)
            }
            Effect::Unsubscribe(session) => {
                self.subs.remove(&session);
                Undo::None
            }
            Effect::Presence(presence) => {
                let previous = self.presence.take();
                self.presence = presence.open.then_some(presence);
                Undo::Presence(previous)
            }
            Effect::Context(context) => {
                if let Some(presence) = &mut self.presence {
                    presence.context = Some(context);
                }
                Undo::None
            }
            Effect::ViewSubscribe(key, tail) => {
                self.views.insert(key.clone(), tail);
                Undo::ViewUnsubscribe(key)
            }
            Effect::ViewUnsubscribe(key) => {
                self.views.remove(&key);
                Undo::None
            }
        }
    }

    fn undo(&mut self, undo: Undo) {
        match undo {
            Undo::None => {}
            Undo::Unsubscribe(session) => {
                self.subs.remove(&session);
            }
            Undo::Presence(previous) => self.presence = previous,
            Undo::ViewUnsubscribe(key) => {
                self.views.remove(&key);
            }
        }
    }

    fn on_frame(&mut self, text: &str) {
        // Frames this version can't read are skipped; so are bus.errors without an id.
        let Ok(envelope) = serde_json::from_str::<Envelope>(text) else {
            return;
        };
        match envelope {
            Envelope::SessionEvents(message) => {
                let record = message.payload;
                if let Some(last) = self.subs.get_mut(&record.session)
                    && record.seq > *last
                {
                    *last = record.seq;
                    self.emit(Event::Session(record));
                }
            }
            Envelope::AgentFocusMessage(message) => self.emit(Event::Focus(message.payload)),
            Envelope::ConnectorsChanged(message) => {
                self.emit(Event::Connectors(message.payload.connectors));
            }
            Envelope::ThemeMessage(message) => self.emit(Event::Theme(message.payload)),
            Envelope::UserSettingsMessage(message) => {
                self.emit(Event::UserSettings(message.payload));
            }
            Envelope::FileChangedMessage(message) => self.emit(Event::FileChanged(message.payload)),
            Envelope::SessionChangedMessage(message) => {
                self.emit(Event::SessionChanged(message.payload));
            }
            Envelope::SessionRemovedMessage(message) => {
                self.emit(Event::SessionRemoved(message.payload.id));
            }
            Envelope::SecretRequestMessage(message) => self.emit(Event::Secret(message.payload)),
            Envelope::ViewPatchMessage(message) => {
                let key = (
                    message.payload.session.clone(),
                    message.payload.agent.clone(),
                );
                if self.views.contains_key(&key) {
                    self.emit(Event::ViewPatch(message.payload));
                }
            }
            reply => self.on_reply(reply),
        }
    }

    fn on_reply(&mut self, envelope: Envelope) {
        if let Envelope::SessionSubscribed(message) = &envelope {
            // Before the waiter wakes, so a caller never sees its reply ahead of the event.
            self.emit(Event::Subscribed {
                session: message.payload.session.clone(),
                replayed_through: message.payload.replayed_through,
            });
        }
        if let Envelope::ViewSnapshotMessage(message) = &envelope {
            // Also on resubscription after a reconnect, where nobody waits for the reply.
            self.emit(Event::ViewSnapshot(message.payload.clone()));
        }
        let Some(id) = reply_id(&envelope) else {
            return;
        };
        let Some(pending) = self.pending.remove(id) else {
            return;
        };
        let result = match envelope {
            Envelope::BusError(error) => {
                self.undo(pending.undo);
                Err(Error::Daemon(error.payload))
            }
            reply => Ok(reply),
        };
        if let Some(reply) = pending.reply {
            let _ = reply.send(result);
        }
    }

    /// The connection is gone: nobody will get the replies they wait for.
    fn fail_pending(&mut self) {
        for (_, pending) in self.pending.drain() {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Err(Error::Disconnected));
            }
        }
    }

    /// Registers presence and resumes each subscription after the last event seen, on a fresh
    /// connection (and at the start of the first). Replies are handled by the read loop.
    fn restore(&mut self, socket: &mut Socket) {
        if let Some(presence) = self.presence.clone() {
            let id = self.internal_id();
            let message = Envelope::AppPresenceMessage(AppPresenceMessage {
                id: Some(id.clone()),
                payload: presence,
            });
            // A registration the daemon refuses must not be sent again on every reconnect.
            self.send_internal(socket, id, message, Undo::Presence(None));
        }
        let sessions: Vec<(String, i64)> = self
            .subs
            .iter()
            .map(|(session, last)| (session.clone(), *last))
            .collect();
        for (session, last) in sessions {
            let id = self.internal_id();
            let message = Envelope::SessionSubscribe(SessionSubscribe {
                id: id.clone(),
                payload: SubscribeParams {
                    session: session.clone(),
                    from_seq: Some(last + 1),
                },
            });
            self.send_internal(socket, id, message, Undo::Unsubscribe(session));
        }
        let views: Vec<(ViewKey, Option<i64>)> =
            self.views.iter().map(|(k, t)| (k.clone(), *t)).collect();
        for (key, tail) in views {
            let id = self.internal_id();
            let message = Envelope::ViewSubscribe(ViewSubscribe {
                id: id.clone(),
                payload: ViewSubscribeParams {
                    session: key.0.clone(),
                    agent: key.1.clone(),
                    tail,
                },
            });
            self.send_internal(socket, id, message, Undo::ViewUnsubscribe(key));
        }
    }

    fn internal_id(&mut self) -> String {
        self.next_internal += 1;
        format!("r{}", self.next_internal)
    }

    fn send_internal(&mut self, socket: &mut Socket, id: String, message: Envelope, undo: Undo) {
        // A failed write means the connection is dead; the read loop notices on its own.
        if send(socket, &message).is_ok() {
            self.pending.insert(id, Pending { reply: None, undo });
        }
    }

    /// Redials with backoff until it works or the client is closed. Requests made meanwhile
    /// fail at once rather than queueing up behind an absent daemon.
    fn redial(&mut self) -> Option<Socket> {
        let mut delay = MIN_BACKOFF;
        loop {
            let deadline = Instant::now() + delay;
            while Instant::now() < deadline {
                if !self.reject_requests() {
                    return None;
                }
                thread::sleep(POLL.min(delay));
            }
            if let Ok(socket) = dial(&self.target.socket, &self.target.client) {
                return Some(socket);
            }
            delay = (delay * 2).min(MAX_BACKOFF);
        }
    }

    /// Answers queued requests with an error; false once the client is closed.
    fn reject_requests(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Request(request)) => {
                    let _ = request.reply.send(Err(Error::Disconnected));
                }
                Ok(Command::Send(_)) => {}
                Ok(Command::Close) | Err(mpsc::TryRecvError::Disconnected) => return false,
                Err(mpsc::TryRecvError::Empty) => return true,
            }
        }
    }
}

fn request_id(envelope: &Envelope) -> Option<String> {
    match envelope {
        Envelope::SessionCreate(m) => Some(m.id.clone()),
        Envelope::SessionList(m) => Some(m.id.clone()),
        Envelope::SessionSubscribe(m) => Some(m.id.clone()),
        Envelope::SessionUnsubscribe(m) => Some(m.id.clone()),
        Envelope::SessionCancel(m) => Some(m.id.clone()),
        Envelope::SessionUnqueue(m) => Some(m.id.clone()),
        Envelope::SessionInputMessage(m) => m.id.clone(),
        Envelope::AppPresenceMessage(m) => m.id.clone(),
        Envelope::ConnectorsList(m) => Some(m.id.clone()),
        Envelope::AgentMoveMessage(m) => Some(m.id.clone()),
        Envelope::ViewSubscribe(m) => Some(m.id.clone()),
        Envelope::ViewUnsubscribe(m) => Some(m.id.clone()),
        Envelope::ViewPageRequest(m) => Some(m.id.clone()),
        Envelope::UiActionMessage(m) => Some(m.id.clone()),
        Envelope::AssetPutMessage(m) => Some(m.id.clone()),
        Envelope::AppContextMessage(m) => m.id.clone(),
        Envelope::AssetGetMessage(m) => Some(m.id.clone()),
        Envelope::AssetFetchMessage(m) => Some(m.id.clone()),
        Envelope::SpeechTranscribeMessage(m) => Some(m.id.clone()),
        _ => None,
    }
}
