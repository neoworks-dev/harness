//! Everything between the app state and the daemon: choosing the session at start, a worker
//! thread that runs the app's blocking requests, and pumping the client's events into the app.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use suite_protocol::{
    AgentFocus, AppPresence, ConnectorInfo, ConnectorKind, SessionCreateParams, SessionInfo,
};
use suite_sdk::{Client, Event};

use crate::app::{App, Config, Effect, Msg};
use crate::history::HistoryStore;

/// What to show at start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// `--session ID|prefix`
    pub session: Option<String>,
    /// `--new`
    pub new: bool,
}

pub const USAGE: &str = "usage: suite-term [--session ID|PREFIX] [--new]

The suite's full-screen terminal app: the shared conversation, plan, files and usage.
Without flags it shows the agent's current session if it works in this directory, else the
latest session here, else a new one.

  --session ID|PREFIX   show this session
  --new                 start a new session in the current directory
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
                None => return Err(format!("unknown argument {other:?}")),
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
}

impl Driver {
    pub fn new(client: Arc<Client>, app_id: &str) -> Driver {
        let (jobs, job_rx) = mpsc::channel();
        let (msg_tx, msgs) = mpsc::channel();
        let worker = {
            let client = client.clone();
            let app = app_id.to_string();
            thread::Builder::new()
                .name("suite-term-worker".into())
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
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn work(client: &Client, app: &str, jobs: &mpsc::Receiver<Job>, msgs: &mpsc::Sender<Msg>) {
    let fail = |what: &str, err: suite_sdk::Error| {
        let _ = msgs.send(Msg::Error(format!("{what}: {err}")));
    };
    while let Ok(Job::Effect(effect)) = jobs.recv() {
        match effect {
            Effect::Input { session, text } => {
                if let Err(err) = client.input(&session, app, &text, &[]) {
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
            Effect::Switch {
                from,
                to,
                cwd,
                move_agent,
            } => {
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
                    .map_err(|e| e.to_string());
                let _ = msgs.send(Msg::Created(result));
            }
            Effect::Copy(_) | Effect::Quit => {}
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
) -> suite_sdk::Result<()> {
    if let Some(old) = from.filter(|old| *old != to) {
        let _ = client.unsubscribe_view(old);
        let _ = client.unsubscribe(old);
    }
    client.subscribe_view(to)?;
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
    pub args: Args,
}

/// Connects, registers as a terminal app, picks the session, and starts following it with the
/// agent moved here. Nothing is drawn yet.
pub fn start(options: StartOptions) -> Result<Started, StartError> {
    let mut client_options = suite_sdk::Options::new("terminal");
    client_options.socket = options.socket;
    client_options.reconnect = true;
    let client = Client::connect(client_options).map_err(|err| match err {
        suite_sdk::Error::DaemonDown => StartError::DaemonDown,
        other => StartError::Other(other.to_string()),
    })?;
    let other = |what: &str, err: suite_sdk::Error| StartError::Other(format!("{what}: {err}"));
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
            .into_iter()
            .find(|s| s.id == id)
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
    });
    let driver = Driver::new(Arc::new(client), &app_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use suite_protocol::{FocusReason, SessionState};

    fn info(id: &str, cwd: &str, last: i64) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            title: None,
            cwd: cwd.into(),
            created_at: 0,
            last_activity: last,
            last_seq: 0,
            state: SessionState::Idle,
            queued: 0,
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
