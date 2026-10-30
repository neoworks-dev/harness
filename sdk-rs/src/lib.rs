//! Rust client for neo bus, kept minimal for GPUI apps. See the README.
//!
//! One background thread owns the WebSocket (over the daemon's Unix socket). Requests are
//! blocking calls with a timeout, so run them on a background executor; everything the daemon
//! pushes arrives on [`Client::events`], an [`async_channel`] a GPUI task can `.recv().await`.
//! There is no async runtime in here.

mod conn;
mod socket;
pub mod tools;
mod view;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use std::{fmt, io, thread};

use conn::{Command, Effect, Request};
pub use neo_protocol as protocol;
use neo_protocol::{
    AgentFocus, AgentMove, AgentMoveMessage, AppContext, AppContextMessage, AppPresence,
    AppPresenceMessage, Asset, AssetFetchMessage, AssetGetMessage, AssetPut, AssetPutMessage,
    AssetRef, AssetUrl, ConnectorInfo, ConnectorsList, Envelope, ErrorInfo, FileChanged,
    FileChangedMessage, SecretAnswer, SecretAnswerMessage, SecretRequest, SessionCancel,
    SessionCreate, SessionCreateParams, SessionEventRecord, SessionInfo, SessionInput,
    SessionInputMessage, SessionList, SessionRef, SessionSubscribe, SessionUnqueue,
    SessionUnqueueParams, SessionUnsubscribe, SubscribeParams, SubscribedPayload, Theme, UiAction,
    UiActionMessage, UserSettings, ViewPatch, ViewRef, ViewSnapshot, ViewSubscribe,
    ViewUnsubscribe, WaitingInput,
};
pub use socket::default_socket_path;
pub use view::ViewState;

/// How long a request waits for its reply.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Protocol version this client speaks (`bus.hello`).
pub const PROTOCOL_VERSION: i64 = 1;

#[derive(Debug)]
pub enum Error {
    /// Nothing is listening on the socket.
    DaemonDown,
    /// The connection ended (or the client is closed) before the reply arrived.
    Disconnected,
    /// No reply within the request timeout.
    Timeout,
    /// The daemon understood the request and refused it (`bus.error`).
    Daemon(ErrorInfo),
    /// The daemon answered with something other than the expected reply.
    Unexpected,
    /// Connecting or the WebSocket handshake failed.
    Connect(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DaemonDown => f.write_str("daemon not running"),
            Error::Disconnected => f.write_str("connection to daemon lost"),
            Error::Timeout => f.write_str("daemon did not answer in time"),
            Error::Daemon(info) => f.write_str(&info.message),
            Error::Unexpected => f.write_str("unexpected reply from daemon"),
            Error::Connect(why) => write!(f, "cannot connect to daemon: {why}"),
            Error::Io(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// What [`Client::connect`] needs. `Options::new("name")` fills in the rest.
#[derive(Debug, Clone)]
pub struct Options {
    /// The daemon's socket; `None` means [`default_socket_path`].
    pub socket: Option<PathBuf>,
    /// Client name for `bus.hello`, e.g. `code-editor`.
    pub client: String,
    /// Registered right after connecting, and again after every reconnect.
    pub presence: Option<AppPresence>,
    /// Redial with backoff when the connection drops, instead of ending the event stream.
    pub reconnect: bool,
}

impl Options {
    pub fn new(client: impl Into<String>) -> Self {
        Options {
            socket: None,
            client: client.into(),
            presence: None,
            reconnect: false,
        }
    }
}

/// Everything the daemon pushes, in wire order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `agent.focus`: where the agent lives now. Sent after every hello and on each change.
    Focus(AgentFocus),
    /// `connectors.changed`.
    Connectors(Vec<ConnectorInfo>),
    /// One event of a subscribed session, replayed or live. Per session they arrive in seq
    /// order with no gaps or repeats, also across reconnects.
    Session(SessionEventRecord),
    /// The replay of a (re)subscription is complete; what follows is live.
    Subscribed {
        session: SessionInfo,
        replayed_through: i64,
    },
    /// The connection came up (`true`, also once right after `connect`) or went down.
    Connection(bool),
    /// `theme`: sent after every hello and when it changes.
    Theme(Theme),
    /// `user.settings`: the user's /settings toggles, after every hello and on each change.
    UserSettings(UserSettings),
    /// The full view of a followed session: after [`Client::subscribe_view`] and again after
    /// each reconnect. Replace what you have.
    ViewSnapshot(ViewSnapshot),
    /// Changes to a followed session's view, to apply in order (see [`ViewState`]).
    ViewPatch(ViewPatch),
    /// `file.changed`: the agent or another app changed a file.
    FileChanged(FileChanged),
    /// `session.changed`: a session's state, title, preview or lines changed (for overviews).
    SessionChanged(SessionInfo),
    /// `session.removed`: the daemon deleted a session nobody wrote in once nothing followed
    /// it; drop it from overviews. Carries its id.
    SessionRemoved(String),
    /// `secret.request`: the daemon asks this client alone for a secret (an API key, a login
    /// code). Answer it with [`Client::answer_secret`]; never log or store the value.
    Secret(SecretRequest),
}

/// A connection to the daemon. Cheap to share behind an `Arc`; dropping it disconnects.
pub struct Client {
    commands: mpsc::Sender<Command>,
    events: async_channel::Receiver<Event>,
    next_id: AtomicU64,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Client {
    /// Connects, says hello and starts the IO thread. Fails with [`Error::DaemonDown`] if the
    /// daemon isn't running; only a lost connection is retried (with `reconnect`). The
    /// presence in `options` is registered in the background; use [`Client::set_presence`] to
    /// wait for the daemon to accept it.
    pub fn connect(options: Options) -> Result<Client> {
        let socket = options.socket.unwrap_or_else(default_socket_path);
        let ws = conn::dial(&socket, &options.client)?;
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let io = conn::Io::new(
            conn::Target {
                socket,
                client: options.client,
                reconnect: options.reconnect,
            },
            command_rx,
            event_tx,
            options.presence.filter(|presence| presence.open),
        );
        let thread = thread::Builder::new()
            .name("neo-sdk".into())
            .spawn(move || io.run(ws))?;
        Ok(Client {
            commands,
            events,
            next_id: AtomicU64::new(1),
            thread: Mutex::new(Some(thread)),
        })
    }

    /// The stream of pushed events. Every receiver shares one queue (each event goes to one of
    /// them), and events queue up without bound until read, so read them or don't ask.
    pub fn events(&self) -> async_channel::Receiver<Event> {
        self.events.clone()
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        match self.call(Effect::None, |id| Envelope::SessionList(SessionList { id }))? {
            Envelope::SessionListResult(reply) => Ok(reply.payload.sessions),
            _ => Err(Error::Unexpected),
        }
    }

    pub fn create_session(&self, params: SessionCreateParams) -> Result<SessionInfo> {
        let create = |id| {
            Envelope::SessionCreate(SessionCreate {
                id,
                payload: params,
            })
        };
        match self.call(Effect::None, create)? {
            Envelope::SessionInfoMessage(reply) => Ok(reply.payload),
            _ => Err(Error::Unexpected),
        }
    }

    /// Replays the session's events from `from_seq` (inclusive; below 1 means the start), then
    /// follows it live. The events arrive on [`Client::events`], followed by
    /// [`Event::Subscribed`]; this call returns at the same point. Subscribing again restarts
    /// the replay at the new `from_seq`.
    pub fn subscribe(&self, session: &str, from_seq: i64) -> Result<SubscribedPayload> {
        let from_seq = from_seq.max(1);
        let effect = Effect::Subscribe {
            session: session.into(),
            from_seq,
        };
        let subscribe = |id| {
            Envelope::SessionSubscribe(SessionSubscribe {
                id,
                payload: SubscribeParams {
                    session: session.into(),
                    from_seq: (from_seq > 1).then_some(from_seq),
                },
            })
        };
        match self.call(effect, subscribe)? {
            Envelope::SessionSubscribed(reply) => Ok(reply.payload),
            _ => Err(Error::Unexpected),
        }
    }

    /// Stops the session's events. Ones already queued on [`Client::events`] may still be read.
    pub fn unsubscribe(&self, session: &str) -> Result<()> {
        let effect = Effect::Unsubscribe(session.into());
        self.ack(effect, |id| {
            Envelope::SessionUnsubscribe(SessionUnsubscribe {
                id,
                payload: session_ref(session),
            })
        })
    }

    /// Submits input; returns once the daemon has queued it. `images` are absolute paths.
    pub fn input(&self, session: &str, app: &str, text: &str, images: &[String]) -> Result<()> {
        self.send_input(session, app, text, images, false)
    }

    /// [`Client::input`] into the running turn at once, where its harness can take it
    /// (steering); else it waits for the next turn like any input.
    pub fn steer(&self, session: &str, app: &str, text: &str, images: &[String]) -> Result<()> {
        self.send_input(session, app, text, images, true)
    }

    fn send_input(
        &self,
        session: &str,
        app: &str,
        text: &str,
        images: &[String],
        steer: bool,
    ) -> Result<()> {
        self.ack(Effect::None, |id| {
            Envelope::SessionInputMessage(SessionInputMessage {
                id: Some(id),
                payload: SessionInput {
                    session: session.into(),
                    app: app.into(),
                    text: text.into(),
                    images: (!images.is_empty()).then(|| images.to_vec()),
                    steer: steer.then_some(true),
                },
            })
        })
    }

    /// Takes back the inputs `app` queued behind the session's running turn, oldest first, to
    /// edit them. Those a running neo turn already took in are not among them.
    pub fn unqueue(&self, session: &str, app: &str) -> Result<Vec<WaitingInput>> {
        let unqueue = |id| {
            Envelope::SessionUnqueue(SessionUnqueue {
                id,
                payload: SessionUnqueueParams {
                    session: session.into(),
                    app: app.into(),
                },
            })
        };
        match self.call(Effect::None, unqueue)? {
            Envelope::SessionUnqueued(reply) => Ok(reply.payload.inputs),
            _ => Err(Error::Unexpected),
        }
    }

    /// Cancels the session's running turn; a no-op when idle.
    pub fn cancel(&self, session: &str) -> Result<()> {
        self.ack(Effect::None, |id| {
            Envelope::SessionCancel(SessionCancel {
                id,
                payload: session_ref(session),
            })
        })
    }

    /// Moves the agent to `app` (no app: back to the home app); `session` also makes that
    /// session the current one. The result arrives as [`Event::Focus`].
    pub fn move_agent(&self, app: Option<&str>, session: Option<&str>) -> Result<()> {
        self.ack(Effect::None, |id| {
            Envelope::AgentMoveMessage(AgentMoveMessage {
                id,
                payload: AgentMove {
                    app: app.map(Into::into),
                    session: session.map(Into::into),
                },
            })
        })
    }

    /// Registers (`open: true`) or unregisters this client as a connector, or updates its
    /// registration. It lasts as long as the connection; with `reconnect` the latest one is
    /// sent again after each reconnect.
    pub fn set_presence(&self, presence: AppPresence) -> Result<()> {
        let effect = Effect::Presence(presence.clone());
        self.ack(effect, |id| {
            Envelope::AppPresenceMessage(AppPresenceMessage {
                id: Some(id),
                payload: presence,
            })
        })
    }

    /// Updates what the registered app `app` shows now (its presence context) without
    /// re-sending the whole presence; kept for reconnects. Cheap enough for every cursor move.
    pub fn set_context(
        &self,
        app: &str,
        context: std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        let effect = Effect::Context(context.clone());
        self.ack(effect, |id| {
            Envelope::AppContextMessage(AppContextMessage {
                id: Some(id),
                payload: AppContext {
                    app: app.into(),
                    context,
                },
            })
        })
    }

    /// Every connector the daemon knows, open or not.
    pub fn list_connectors(&self) -> Result<Vec<ConnectorInfo>> {
        match self.call(Effect::None, |id| {
            Envelope::ConnectorsList(ConnectorsList { id })
        })? {
            Envelope::ConnectorsListResult(reply) => Ok(reply.payload.connectors),
            _ => Err(Error::Unexpected),
        }
    }

    /// Follows the session's resolved view: returns the snapshot (also sent as
    /// [`Event::ViewSnapshot`]), then [`Event::ViewPatch`]es arrive on [`Client::events`].
    pub fn subscribe_view(&self, session: &str) -> Result<ViewSnapshot> {
        self.subscribe_view_of(session, None)
    }

    /// Follows the view of one agent's work in the session (a subagent, or a guest the user
    /// addressed): its messages, thoughts and tool calls, none of the main conversation. Its
    /// snapshot and patches carry `agent`, and are told apart from the session's own that way.
    pub fn subscribe_agent_view(&self, session: &str, agent: &str) -> Result<ViewSnapshot> {
        self.subscribe_view_of(session, Some(agent))
    }

    fn subscribe_view_of(&self, session: &str, agent: Option<&str>) -> Result<ViewSnapshot> {
        let effect = Effect::ViewSubscribe((session.into(), agent.map(Into::into)));
        let subscribe = |id| {
            Envelope::ViewSubscribe(ViewSubscribe {
                id,
                payload: view_ref(session, agent),
            })
        };
        match self.call(effect, subscribe)? {
            Envelope::ViewSnapshotMessage(reply) => Ok(reply.payload),
            _ => Err(Error::Unexpected),
        }
    }

    pub fn unsubscribe_view(&self, session: &str) -> Result<()> {
        self.unsubscribe_view_of(session, None)
    }

    pub fn unsubscribe_agent_view(&self, session: &str, agent: &str) -> Result<()> {
        self.unsubscribe_view_of(session, Some(agent))
    }

    fn unsubscribe_view_of(&self, session: &str, agent: Option<&str>) -> Result<()> {
        let effect = Effect::ViewUnsubscribe((session.into(), agent.map(Into::into)));
        self.ack(effect, |id| {
            Envelope::ViewUnsubscribe(ViewUnsubscribe {
                id,
                payload: view_ref(session, agent),
            })
        })
    }

    /// A button in item `item_ref` was pressed. `copy:` actions are the caller's to handle.
    pub fn action(
        &self,
        session: &str,
        item_ref: &str,
        action: &str,
        args: Option<serde_json::Value>,
    ) -> Result<()> {
        self.ack(Effect::None, |id| {
            Envelope::UiActionMessage(UiActionMessage {
                id,
                payload: UiAction {
                    session: session.into(),
                    r#ref: item_ref.into(),
                    action: action.into(),
                    args,
                },
            })
        })
    }

    /// Stores a file (`path`) or bytes (`data`, base64) in the asset store. Identical content
    /// gets the same handle.
    pub fn put_asset(&self, asset: AssetPut) -> Result<Asset> {
        let put = |id| Envelope::AssetPutMessage(AssetPutMessage { id, payload: asset });
        match self.call(Effect::None, put)? {
            Envelope::AssetInfoMessage(reply) => Ok(reply.payload),
            _ => Err(Error::Unexpected),
        }
    }

    /// Looks up an `asset://…` handle: its stored file's path, name and type.
    pub fn get_asset(&self, handle: &str) -> Result<Asset> {
        let get = |id| {
            Envelope::AssetGetMessage(AssetGetMessage {
                id,
                payload: AssetRef {
                    handle: handle.into(),
                },
            })
        };
        match self.call(Effect::None, get)? {
            Envelope::AssetInfoMessage(reply) => Ok(reply.payload),
            _ => Err(Error::Unexpected),
        }
    }

    /// Has the daemon download the image at an `http(s)://` URL into the asset store (once per
    /// URL); the asset, or an error when it isn't a public image. Blocks while it downloads.
    pub fn fetch_asset(&self, url: &str) -> Result<Asset> {
        let fetch = |id| {
            Envelope::AssetFetchMessage(AssetFetchMessage {
                id,
                payload: AssetUrl { url: url.into() },
            })
        };
        match self.call(Effect::None, fetch)? {
            Envelope::AssetInfoMessage(reply) => Ok(reply.payload),
            _ => Err(Error::Unexpected),
        }
    }

    /// Tells the other clients this app changed a file (after saving it). No reply; lost if
    /// the connection is down.
    pub fn file_changed(&self, path: &str) -> Result<()> {
        let envelope = Envelope::FileChangedMessage(FileChangedMessage {
            payload: FileChanged {
                path: path.into(),
                app: None,
                session: None,
            },
        });
        self.commands
            .send(Command::Send(Box::new(envelope)))
            .map_err(|_| Error::Disconnected)
    }

    /// Answers a `secret.request`: the typed value, or `None` when the user cancelled. No reply;
    /// lost if the connection is down (the daemon drops the question then anyway).
    pub fn answer_secret(&self, id: &str, value: Option<String>) -> Result<()> {
        let envelope = Envelope::SecretAnswerMessage(SecretAnswerMessage {
            payload: SecretAnswer {
                id: id.into(),
                value,
            },
        });
        self.commands
            .send(Command::Send(Box::new(envelope)))
            .map_err(|_| Error::Disconnected)
    }

    fn ack(&self, effect: Effect, build: impl FnOnce(String) -> Envelope) -> Result<()> {
        match self.call(effect, build)? {
            Envelope::BusAck(_) => Ok(()),
            _ => Err(Error::Unexpected),
        }
    }

    /// Sends one request and waits for the frame carrying its id. `bus.error` becomes
    /// [`Error::Daemon`].
    fn call(&self, effect: Effect, build: impl FnOnce(String) -> Envelope) -> Result<Envelope> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (reply, answer) = mpsc::channel();
        let request = Request {
            envelope: build(id),
            reply,
            effect,
        };
        self.commands
            .send(Command::Request(Box::new(request)))
            .map_err(|_| Error::Disconnected)?;
        match answer.recv_timeout(REQUEST_TIMEOUT) {
            Ok(reply) => reply,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Disconnected),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
        if let Some(thread) = self.thread.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = thread.join();
        }
    }
}

fn view_ref(session: &str, agent: Option<&str>) -> ViewRef {
    ViewRef {
        session: session.into(),
        agent: agent.map(Into::into),
    }
}

fn session_ref(session: &str) -> SessionRef {
    SessionRef {
        session: session.into(),
    }
}
