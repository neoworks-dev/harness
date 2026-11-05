//! Everything between the app state and the daemon: choosing the session at start, a worker
//! thread that runs the app's blocking requests, and pumping the client's events into the app.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use neo_protocol::{
    AgentFocus, AppPresence, AudioFormat, ConnectorInfo, ConnectorKind, SessionCreateParams,
    SessionInfo, SpeechAudio,
};
use neo_sdk::{Client, Event};

use crate::app::{App, Config, Effect, Msg};
use crate::attach;
use crate::history::HistoryStore;
use crate::secret::SecretValue;
use crate::util::base64;
use crate::voice;

/// What to show at start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// `--session ID|prefix`
    pub session: Option<String>,
    /// `--new`
    pub new: bool,
    /// `DIR`: the directory to work in instead of the current one.
    pub dir: Option<String>,
}

pub const USAGE: &str = "usage: neo-term [--session ID|PREFIX] [--new] [DIR]

Neo's full-screen terminal app: the shared conversation, plan, files and usage.
Without flags it shows the agent's current session if it works in DIR (default: the current
directory), else the latest session there, else a new one.

  --session ID|PREFIX   show this session
  --new                 start a new session in DIR
  -h, --help            this text";

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Option<Args>, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--new" => out.new = true,
            "--session" => {
                out.session = Some(it.next().ok_or("--session needs an id")?);
            }
            other => match other.strip_prefix("--session=") {
                Some(id) => out.session = Some(id.to_string()),
                None if other.starts_with('-') => {
                    return Err(format!("unknown argument {other:?}"));
                }
                None if out.dir.is_some() => return Err("only one directory".into()),
                None => out.dir = Some(other.to_string()),
            },
        }
    }
    if out.new && out.session.is_some() {
        return Err("--new and --session don't go together".into());
    }
    Ok(Some(out))
}

/// Which session to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Existing(String),
    New,
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// `--session` first (an exact id or a unique prefix), `--new`, else the agent's current
/// session when it works in `cwd`, else the most recently active session in `cwd`, else new.
pub fn choose_session(
    sessions: &[SessionInfo],
    focus: Option<&AgentFocus>,
    cwd: &str,
    args: &Args,
) -> Result<Choice, String> {
    if let Some(want) = &args.session {
        if let Some(exact) = sessions.iter().find(|s| &s.id == want) {
            return Ok(Choice::Existing(exact.id.clone()));
        }
        let hits: Vec<_> = sessions.iter().filter(|s| s.id.starts_with(want)).collect();
        return match hits.as_slice() {
            [one] => Ok(Choice::Existing(one.id.clone())),
            [] => Err(format!("no session {want:?}")),
            _ => Err(format!(
                "{want:?} matches {} sessions: be more specific",
                hits.len()
            )),
        };
    }
    if args.new {
        return Ok(Choice::New);
    }
    if let Some(current) = focus.and_then(|f| sessions.iter().find(|s| s.id == f.session))
        && same_dir(&current.cwd, cwd)
    {
        return Ok(Choice::Existing(current.id.clone()));
    }
    Ok(sessions
        .iter()
        .filter(|s| same_dir(&s.cwd, cwd))
        .max_by_key(|s| s.last_activity)
        .map_or(Choice::New, |s| Choice::Existing(s.id.clone())))
}

/// `terminal`, or `terminal-2`… when another terminal is already connected under that name
/// (registering a taken name would replace the other one).
pub fn pick_app_id(connectors: &[ConnectorInfo]) -> String {
    let taken = |name: &str| connectors.iter().any(|c| c.app == name && c.connected);
    if !taken("terminal") {
        return "terminal".into();
    }
    (2..)
        .map(|n| format!("terminal-{n}"))
        .find(|name| !taken(name))
        .expect("unbounded range")
}

pub fn presence(app_id: &str, cwd: &str) -> AppPresence {
    AppPresence {
        app: app_id.into(),
        kind: ConnectorKind::App,
        title: Some("Terminal".into()),
        open: true,
        context: Some(BTreeMap::from([("cwd".to_string(), cwd.to_string())])),
        mcp: None,
        launch: None,
        home: None,
        pid: Some(i64::from(std::process::id())),
        templates: None,
        components: None,
        description: None,
    }
}

enum Job {
    Effect(Effect),
}

/// Runs the app's effects: requests on a worker thread (they block up to 10 s), events pumped
/// back into the app.
pub struct Driver {
    client: Arc<Client>,
    jobs: Option<mpsc::Sender<Job>>,
    msgs: mpsc::Receiver<Msg>,
    worker: Option<thread::JoinHandle<()>>,
}

/// What the runtime must do itself.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub copy: Vec<String>,
    pub quit: bool,
    /// A file to open in the editor, and the line.
    pub edit: Option<(std::path::PathBuf, Option<usize>)>,
}

impl Driver {
    pub fn new(client: Arc<Client>, app_id: &str) -> Driver {
        let (jobs, job_rx) = mpsc::channel();
        let (msg_tx, msgs) = mpsc::channel();
        let worker = {
            let client = client.clone();
            let app = app_id.to_string();
            thread::Builder::new()
                .name("neo-term-worker".into())
                .spawn(move || work(&client, &app, &job_rx, &msg_tx))
                .expect("spawn worker")
        };
        Driver {
            client,
            jobs: Some(jobs),
            msgs,
            worker: Some(worker),
        }
    }

    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    /// Carries out what the app asked for. Clipboard writes and quitting are the caller's.
    pub fn run(&self, effects: Vec<Effect>) -> Outcome {
        let mut out = Outcome::default();
        for effect in effects {
            match effect {
                Effect::Copy(text) => out.copy.push(text),
                Effect::Quit => out.quit = true,
                Effect::Edit { path, line } => out.edit = Some((path, line)),
                other => {
                    if let Some(jobs) = &self.jobs {
                        let _ = jobs.send(Job::Effect(other));
                    }
                }
            }
        }
        out
    }

    /// Moves everything waiting (worker answers, then daemon events) into the app and runs the
    /// effects that causes. Returns what the caller must do, and whether anything came in.
    pub fn pump(&self, app: &mut App) -> (Outcome, bool) {
        let mut out = Outcome::default();
        let mut any = false;
        while let Ok(msg) = self.msgs.try_recv() {
            any = true;
            merge(&mut out, self.run(app.handle_msg(msg)));
        }
        let events = self.client.events();
        while let Ok(event) = events.try_recv() {
            any = true;
            merge(&mut out, self.run(app.handle_event(event)));
        }
        (out, any)
    }
}

fn merge(into: &mut Outcome, from: Outcome) {
    into.copy.extend(from.copy);
    into.quit |= from.quit;
    if from.edit.is_some() {
        into.edit = from.edit;
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn work(client: &Arc<Client>, app: &str, jobs: &mpsc::Receiver<Job>, msgs: &mpsc::Sender<Msg>) {
    // The microphone while Space is held (see voice.rs).
    let mut recording: Option<voice::Recording> = None;
    let fail = |what: &str, err: neo_sdk::Error| {
        let _ = msgs.send(Msg::Error(format!("{what}: {err}")));
    };
    // What is followed besides the shown session: the views of single agents (transcripts).
    // Dropped when the session changes.
    let mut agent_views: HashSet<(String, String)> = HashSet::new();
    while let Ok(Job::Effect(effect)) = jobs.recv() {
        match effect {
            Effect::Input {
                session,
                text,
                images,
                steer,
            } => {
                let sent = if steer {
                    client.steer(&session, app, &text, &images)
                } else {
                    client.input(&session, app, &text, &images)
                };
                if let Err(err) = sent {
                    fail("send failed", err);
                }
            }
            Effect::Action {
                session,
                item,
                action,
                args,
            } => {
                if let Err(err) = client.action(&session, &item, &action, args) {
                    fail("action failed", err);
                }
            }
            Effect::Cancel { session } => {
                if let Err(err) = client.cancel(&session) {
                    fail("cancel failed", err);
                }
            }
            Effect::AgentView { session, agent, on } => {
                let key = (session.clone(), agent.clone());
                if on {
                    agent_views.insert(key);
                    if let Err(err) = client.subscribe_agent_view(&session, &agent) {
                        fail("opening the transcript failed", err);
                    }
                } else if agent_views.remove(&key) {
                    let _ = client.unsubscribe_agent_view(&session, &agent);
                }
            }
            Effect::Switch {
                from,
                to,
                cwd,
                move_agent,
            } => {
                for (session, agent) in agent_views.drain() {
                    let _ = client.unsubscribe_agent_view(&session, &agent);
                }
                if let Err(err) = switch(client, app, from.as_deref(), &to, &cwd, move_agent) {
                    fail("switching session failed", err);
                }
            }
            Effect::ListSessions | Effect::Refresh => {
                let result = client.list_sessions().map_err(|e| e.to_string());
                let _ = msgs.send(Msg::Sessions(result));
            }
            Effect::CreateSession { cwd } => {
                let result = client
                    .create_session(SessionCreateParams {
                        cwd: Some(cwd),
                        title: None,
                    })
                    .map(Box::new)
                    .map_err(|e| e.to_string());
                let _ = msgs.send(Msg::Created(result));
            }
            Effect::Secret { id, value } => {
                // The value is moved onto the wire and not kept; an error never carries it.
                if let Err(err) = client.answer_secret(&id, value.map(SecretValue::take)) {
                    fail("answering the question failed", err);
                }
            }
            Effect::Page {
                session,
                before,
                count,
            } => {
                let page = client
                    .view_page(&session, &before, count)
                    .map_err(|e| e.to_string());
                let _ = msgs.send(Msg::Page { session, page });
            }
            Effect::OpenUrl(url) => open_url(&url, msgs),
            Effect::Unqueue { session } => match client.unqueue(&session, app) {
                Ok(inputs) => {
                    let _ = msgs.send(Msg::Unqueued { session, inputs });
                }
                Err(err) => fail("taking the queued messages back failed", err),
            },
            Effect::VoiceStart => match voice::Recording::start() {
                Ok(started) => recording = Some(started),
                Err(why) => {
                    let _ = msgs.send(Msg::VoiceFailed(why));
                }
            },
            Effect::VoiceStop => {
                let Some(stopped) = recording.take() else {
                    continue;
                };
                // Written out on a thread of its own: it takes seconds, the worker goes on.
                let client = client.clone();
                let msgs = msgs.clone();
                thread::spawn(move || {
                    let result = match stopped.stop() {
                        Ok(None) => Ok(String::new()),
                        Ok(Some(wav)) => client
                            .transcribe(SpeechAudio {
                                data: base64(&wav),
                                format: AudioFormat::Wav,
                            })
                            .map_err(|e| format!("speech to text failed: {e}")),
                        Err(why) => Err(why),
                    };
                    let _ = msgs.send(Msg::Transcribed(result));
                });
            }
            Effect::PasteImage => {
                let path = attach::paste_image().map(|p| p.to_string_lossy().into_owned());
                let _ = msgs.send(Msg::Pasted(path));
            }
            Effect::Copy(_) | Effect::Quit | Effect::Edit { .. } => {}
        }
    }
}

/// Hands a link to the desktop; says so when there is nothing to hand it to.
fn open_url(url: &str, msgs: &mpsc::Sender<Msg>) {
    let child = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        // Reaped on a thread of its own: some openers stay until the browser is up.
        Ok(mut child) => {
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(_) => {
            let _ = msgs.send(Msg::Notice(format!("no xdg-open to open {url}")));
        }
    }
}

fn switch(
    client: &Client,
    app: &str,
    from: Option<&str>,
    to: &str,
    cwd: &str,
    move_agent: bool,
) -> neo_sdk::Result<()> {
    if let Some(old) = from.filter(|old| *old != to) {
        let _ = client.unsubscribe_view(old);
        let _ = client.unsubscribe(old);
    }
    // The latest items only: a long chat shows at once, older ones are paged in.
    client.subscribe_view_tail(to, crate::app::TAIL_ITEMS)?;
    client.subscribe(to, 1)?;
    if move_agent {
        client.move_agent(Some(app), Some(to))?;
    }
    client.set_context(app, BTreeMap::from([("cwd".to_string(), cwd.to_string())]))?;
    Ok(())
}

/// Why the app can't start.
#[derive(Debug)]
pub enum StartError {
    DaemonDown,
    Other(String),
}

pub struct Started {
    pub app: App,
    pub driver: Driver,
}

pub struct StartOptions {
    /// The daemon socket; None: the default.
    pub socket: Option<std::path::PathBuf>,
    pub cwd: String,
    pub home: Option<String>,
    pub no_color: bool,
    pub history: HistoryStore,
    pub prefs: crate::theme::Prefs,
    pub args: Args,
}

/// Connects, registers as a terminal app, picks the session, and starts following it with the
/// agent moved here. Nothing is drawn yet.
pub fn start(options: StartOptions) -> Result<Started, StartError> {
    let mut client_options = neo_sdk::Options::new("terminal");
    client_options.socket = options.socket;
    client_options.reconnect = true;
    let client = Client::connect(client_options).map_err(|err| match err {
        neo_sdk::Error::DaemonDown => StartError::DaemonDown,
        other => StartError::Other(other.to_string()),
    })?;
    let other = |what: &str, err: neo_sdk::Error| StartError::Other(format!("{what}: {err}"));
    let events = client.events();

    // The daemon sends the agent's location right after hello; wait a moment for it.
    let mut early = Vec::new();
    let mut focus: Option<AgentFocus> = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while focus.is_none() && Instant::now() < deadline {
        match events.try_recv() {
            Ok(Event::Focus(f)) => {
                focus = Some(f.clone());
                early.push(Event::Focus(f));
            }
            Ok(event) => early.push(event),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }

    let connectors = client
        .list_connectors()
        .map_err(|e| other("listing apps", e))?;
    let app_id = pick_app_id(&connectors);
    client
        .set_presence(presence(&app_id, &options.cwd))
        .map_err(|e| other("registering", e))?;

    let sessions = client
        .list_sessions()
        .map_err(|e| other("listing sessions", e))?;
    let session = match choose_session(&sessions, focus.as_ref(), &options.cwd, &options.args)
        .map_err(StartError::Other)?
    {
        Choice::Existing(id) => sessions
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .expect("chosen from the list"),
        Choice::New => client
            .create_session(SessionCreateParams {
                cwd: Some(options.cwd.clone()),
                title: None,
            })
            .map_err(|e| other("creating a session", e))?,
    };

    let mut app = App::new(Config {
        app_id: app_id.clone(),
        session: session.id.clone(),
        cwd: session.cwd.clone(),
        home: options.home,
        no_color: options.no_color,
        history: options.history,
        prefs: options.prefs,
    });
    let driver = Driver::new(Arc::new(client), &app_id);
    // The sidebar's overview starts from this list; session.changed events keep it current.
    let mut known = sessions;
    if !known.iter().any(|s| s.id == session.id) {
        known.push(session.clone());
    }
    app.set_sessions(known);
    // The sidebar's servers start from this list; connectors.changed keeps it current.
    app.handle_event(Event::Connectors(connectors));
    for event in early {
        let fx = app.handle_event(event);
        driver.run(fx);
    }
    driver.run(vec![Effect::Switch {
        from: None,
        to: session.id,
        cwd: session.cwd,
        move_agent: true,
    }]);
    Ok(Started { app, driver })
}

/// The process's working directory as the daemon would see it (symlinks resolved).
pub fn current_dir() -> Result<String, String> {
    let dir = std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;
    Ok(Path::new(&dir).to_string_lossy().into_owned())
}

/// `dir` made absolute, which must be a directory.
pub fn work_dir(dir: &str) -> Result<String, String> {
    let path = std::fs::canonicalize(dir).map_err(|e| format!("{dir}: {e}"))?;
    if !path.is_dir() {
        return Err(format!("{dir}: not a directory"));
    }
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_protocol::{FocusReason, SessionState};

    fn info(id: &str, cwd: &str, last: i64) -> SessionInfo {
        SessionInfo {
            harness: None,
            parent: None,
            id: id.into(),
            title: None,
            cwd: cwd.into(),
            created_at: 0,
            last_activity: last,
            last_seq: 0,
            state: SessionState::Idle,
            queued: 0,
            waiting: None,
            asking: None,
            unseen: None,
            preview: None,
            added: None,
            removed: None,
        }
    }

    fn focus(session: &str) -> AgentFocus {
        AgentFocus {
            session: session.into(),
            app: None,
            from: None,
            reason: FocusReason::Initial,
        }
    }

    fn args(session: Option<&str>, new: bool) -> Args {
        Args {
            session: session.map(Into::into),
            new,
            dir: None,
        }
    }

    #[test]
    fn parses_flags() {
        let p = |a: &[&str]| parse_args(a.iter().map(|s| s.to_string()));
        assert_eq!(p(&[]), Ok(Some(Args::default())));
        assert_eq!(p(&["--new"]), Ok(Some(args(None, true))));
        assert_eq!(p(&["--session", "ab"]), Ok(Some(args(Some("ab"), false))));
        assert_eq!(p(&["--session=ab"]), Ok(Some(args(Some("ab"), false))));
        assert_eq!(p(&["-h"]), Ok(None));
        assert!(p(&["--session"]).is_err());
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["--new", "--session", "x"]).is_err());
        let in_dir = Args {
            dir: Some("../site".into()),
            ..args(None, true)
        };
        assert_eq!(p(&["--new", "../site"]), Ok(Some(in_dir)));
        assert!(p(&["a", "b"]).is_err());
    }

    #[test]
    fn work_dir_is_absolute_and_a_directory() {
        let here = std::env::current_dir().unwrap();
        let parent = here.parent().unwrap().canonicalize().unwrap();
        assert_eq!(work_dir(".."), Ok(parent.to_string_lossy().into_owned()));
        assert!(work_dir("Cargo.toml").is_err());
        assert!(work_dir("no/such/dir").is_err());
    }

    #[test]
    fn session_choice() {
        let list = [
            info("aa11", "/w/site", 10),
            info("aa22", "/w/site", 30),
            info("bb33", "/w/other", 50),
        ];
        let here = "/w/site";
        let c = |f: Option<&AgentFocus>, a: &Args| choose_session(&list, f, here, a);
        // The agent's session wins when it is in this directory, even if not the latest.
        assert_eq!(
            c(Some(&focus("aa11")), &args(None, false)),
            Ok(Choice::Existing("aa11".into()))
        );
        // In another directory: the latest session here.
        assert_eq!(
            c(Some(&focus("bb33")), &args(None, false)),
            Ok(Choice::Existing("aa22".into()))
        );
        assert_eq!(
            c(None, &args(None, false)),
            Ok(Choice::Existing("aa22".into()))
        );
        assert_eq!(c(None, &args(None, true)), Ok(Choice::New));
        assert_eq!(
            c(None, &args(Some("bb"), false)),
            Ok(Choice::Existing("bb33".into()))
        );
        assert_eq!(
            c(None, &args(Some("aa11"), false)),
            Ok(Choice::Existing("aa11".into()))
        );
        assert!(
            c(None, &args(Some("aa"), false))
                .unwrap_err()
                .contains("matches 2")
        );
        assert!(c(None, &args(Some("zz"), false)).is_err());
        assert_eq!(
            choose_session(&list, None, "/w/none", &Args::default()),
            Ok(Choice::New)
        );
        assert_eq!(
            choose_session(&list, None, "/w/site/", &Args::default()),
            Ok(Choice::Existing("aa22".into())),
            "a trailing slash is the same directory"
        );
    }

    fn connector(app: &str, connected: bool) -> ConnectorInfo {
        ConnectorInfo {
            app: app.into(),
            kind: ConnectorKind::App,
            title: None,
            connected,
            v: 0,
            tools: vec![],
            error: None,
            home: None,
            pid: None,
            context: None,
        }
    }

    #[test]
    fn a_second_terminal_gets_its_own_name() {
        assert_eq!(pick_app_id(&[]), "terminal");
        assert_eq!(pick_app_id(&[connector("terminal", false)]), "terminal");
        assert_eq!(pick_app_id(&[connector("terminal", true)]), "terminal-2");
        assert_eq!(
            pick_app_id(&[connector("terminal", true), connector("terminal-2", true)]),
            "terminal-3"
        );
    }

    #[test]
    fn presence_is_an_open_non_home_app_with_the_cwd() {
        let p = presence("terminal", "/w/site");
        assert_eq!(p.kind, ConnectorKind::App);
        assert!(p.open);
        assert_eq!(p.home, None);
        assert_eq!(p.title.as_deref(), Some("Terminal"));
        assert_eq!(p.context.unwrap()["cwd"], "/w/site");
    }
}
