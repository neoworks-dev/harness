//! The terminal app against the real daemon (`bun run daemon/src/main.ts` with the echo agent),
//! driven headless: keys go into the app, frames are drawn into a test backend. Skipped with a
//! message when bun isn't installed.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use neo_term::app::App;
use neo_term::driver::{Args, Driver, StartOptions, Started, start};
use neo_term::history::HistoryStore;
use neo_term::util::now_ms;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> TempDir {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("neo-term-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Daemon {
    child: Child,
    socket: PathBuf,
    dir: TempDir,
}

impl Daemon {
    fn start() -> Option<Daemon> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = TempDir::new("daemon");
        let socket = dir.join("bus.sock");
        let child = match Command::new("bun")
            .args(["run", "daemon/src/main.ts"])
            .current_dir(root)
            .env("NEO_AGENT", "echo")
            .env("NEO_SERVICES", "none")
            .env("NEO_GUESTS", "none")
            .env("NEO_SOCKET", &socket)
            .env("NEO_DATA_DIR", dir.join("data"))
            .env("NEO_PORT", "0")
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
        let daemon = Daemon { child, socket, dir };
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

    fn open(&self, cwd: &str, args: Args) -> Started {
        match start(StartOptions {
            socket: Some(self.socket.clone()),
            cwd: cwd.into(),
            home: Some("/nonexistent-home".into()),
            no_color: true,
            history: HistoryStore::new(Some(self.dir.join("history"))),
            prefs: neo_term::theme::Prefs::default(),
            args,
        }) {
            Ok(started) => started,
            Err(err) => panic!("start failed: {err:?}"),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Ui {
    app: App,
    driver: Driver,
    terminal: Terminal<TestBackend>,
}

impl Ui {
    fn new(started: Started) -> Ui {
        let mut ui = Ui {
            app: started.app,
            driver: started.driver,
            terminal: Terminal::new(TestBackend::new(100, 30)).unwrap(),
        };
        ui.app.resize(100, 30);
        ui
    }

    fn screen(&mut self) -> String {
        self.terminal.draw(|f| self.app.draw(f)).unwrap();
        let buffer = self.terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Runs the loop until the screen satisfies `done`.
    fn until(&mut self, what: &str, done: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.driver.pump(&mut self.app);
            let fx = self.app.tick(now_ms());
            self.driver.run(fx);
            let screen = self.screen();
            if done(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; screen:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn key(&mut self, code: KeyCode) {
        let fx = self.app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        self.driver.run(fx);
    }

    fn type_line(&mut self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c));
        }
        self.key(KeyCode::Enter);
    }
}

#[test]
fn chat_with_the_echo_agent_from_the_terminal_app() {
    let Some(daemon) = Daemon::start() else {
        return;
    };
    let work = TempDir::new("work");
    let cwd = work.0.to_string_lossy().into_owned();
    let mut ui = Ui::new(daemon.open(&cwd, Args::default()));
    let first_session = ui.app.session().to_string();

    // The chat view of the new session arrives; the status line names the directory.
    let screen = ui.until("an empty session", |s| s.contains("enter send"));
    let project = cwd.rsplit('/').next().unwrap();
    assert!(screen.contains(&format!("{project} · neo")), "{screen}");

    // Typing sends input; the echo agent's reply is drawn in the chat.
    ui.type_line("hello from the terminal");
    let screen = ui.until("the echo reply", |s| {
        s.contains("echo #1: hello from the terminal")
    });
    assert!(screen.contains("› hello from the terminal"), "{screen}");
    ui.until("the turn to end", |s| !s.contains("working"));

    // Registered as the app "terminal", and the agent is here with this session.
    let probe = neo_sdk::Client::connect({
        let mut o = neo_sdk::Options::new("probe");
        o.socket = Some(daemon.socket.clone());
        o
    })
    .unwrap();
    let events = probe.events();
    let focus = loop {
        match events.recv_blocking() {
            Ok(neo_sdk::Event::Focus(f)) => break f,
            Ok(_) => {}
            Err(e) => panic!("{e}"),
        }
    };
    assert_eq!(focus.app.as_deref(), Some("terminal"));
    assert_eq!(focus.session, first_session);
    let me = probe
        .list_connectors()
        .unwrap()
        .into_iter()
        .find(|c| c.app == "terminal")
        .expect("registered");
    assert!(me.connected);
    assert_eq!(me.context.as_ref().and_then(|c| c.get("cwd")), Some(&cwd));

    // The sessions sidebar follows the daemon's session.changed events: title and preview.
    let side = |screen: &str| {
        screen
            .lines()
            .map(|l| l.chars().take(29).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let screen = ui.until("the session in the sidebar", |s| {
        let side = side(s);
        // The shown session's title goes on under itself.
        side.contains("hello from the")
            && side.contains("terminal")
            && side.contains("[app_connected]")
    });
    assert!(screen.contains("Sessions · "), "{screen}");
    assert!(screen.contains("· neo"), "the session's harness: {screen}");

    // Approvals: shift-tab goes through the daemon (ask, review, edits, auto, ask) and the mode
    // comes back as a setting event.
    assert!(
        ui.screen()
            .contains("? ask before acting (shift+tab to cycle)")
    );
    ui.key(KeyCode::BackTab);
    ui.until("review", |s| {
        s.contains("⏵ review each edit (shift+tab to cycle)")
    });
    ui.key(KeyCode::BackTab);
    ui.until("accept edits", |s| {
        s.contains("⏵⏵ accept edits on (shift+tab to cycle)")
    });
    ui.key(KeyCode::BackTab);
    ui.until("auto approvals", |s| {
        s.contains("⏵⏵ auto mode on (shift+tab to cycle)")
    });

    // /clear: the same session goes on, with the context cleared.
    ui.type_line("/clear");
    ui.until("the cleared context", |s| s.contains("context cleared"));
    assert_eq!(ui.app.session(), first_session);

    // /new: the daemon makes a fresh session current and the app follows it.
    ui.type_line("/new");
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.app.session() == first_session {
        ui.driver.pump(&mut ui.app);
        assert!(Instant::now() < deadline, "never followed the new session");
        std::thread::sleep(Duration::from_millis(10));
    }
    // (The sidebar still lists the old session, so look at the chat pane only.)
    let chat = |s: &str| {
        s.lines()
            .map(|l| l.chars().skip(31).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let screen = ui.until("an empty chat", |s| !chat(s).contains("echo #1"));
    assert!(
        !chat(&screen).contains("hello from the terminal"),
        "{screen}"
    );
    ui.type_line("again");
    ui.until("the echo in the new session", |s| {
        chat(s).contains("echo #1: again")
    });

    // The first session is still there and the switcher can go back to it.
    ui.type_line("/sessions");
    ui.until("the session list", |s| s.contains("new session in"));
    let screen = ui.screen();
    assert!(
        screen.contains("session ") || screen.contains(&cwd),
        "{screen}"
    );
}

#[test]
fn a_second_terminal_registers_under_its_own_name_and_resumes_the_session() {
    let Some(daemon) = Daemon::start() else {
        return;
    };
    let work = TempDir::new("work2");
    let cwd = work.0.to_string_lossy().into_owned();
    let mut first = Ui::new(daemon.open(&cwd, Args::default()));
    first.until("session", |s| s.contains("enter send"));
    first.type_line("one");
    first.until("echo", |s| s.contains("echo #1: one"));

    // A second app in the same directory picks the agent's current session, with its history
    // replayed, and does not replace the first one.
    let mut second = Ui::new(daemon.open(&cwd, Args::default()));
    assert_eq!(second.app.session(), first.app.session());
    second.until("replay", |s| s.contains("echo #1: one"));
    let probe = neo_sdk::Client::connect({
        let mut o = neo_sdk::Options::new("probe");
        o.socket = Some(daemon.socket.clone());
        o
    })
    .unwrap();
    let names: Vec<_> = probe
        .list_connectors()
        .unwrap()
        .into_iter()
        .filter(|c| c.connected)
        .map(|c| c.app)
        .collect();
    assert!(
        names.contains(&"terminal".to_string()) && names.contains(&"terminal-2".to_string()),
        "{names:?}"
    );

    // What one types, the other shows.
    second.type_line("two");
    first.until("the other terminal's input", |s| s.contains("echo #2: two"));
    // `--new` starts another session.
    let third = Ui::new(daemon.open(
        &cwd,
        Args {
            new: true,
            ..Args::default()
        },
    ));
    assert_ne!(third.app.session(), first.app.session());
}
