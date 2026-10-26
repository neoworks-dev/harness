//! The process: terminal setup and restore, the event loop, and `main`.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui_image::picker::Picker;

use crate::driver::{self, Driver, StartError, StartOptions, USAGE};
use crate::history::HistoryStore;
use crate::images::{self, Assets, Images};
use crate::util::{now_ms, osc52};

/// How long to wait for a key before looking at the daemon's events again.
const POLL: Duration = Duration::from_millis(50);

pub fn main() -> ExitCode {
    let args = match driver::parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("neo-term: {why}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        eprintln!("neo-term needs a terminal (use `neo attach` in scripts)");
        return ExitCode::FAILURE;
    }
    let cwd = match args
        .dir
        .as_deref()
        .map_or_else(driver::current_dir, driver::work_dir)
    {
        Ok(cwd) => cwd,
        Err(why) => {
            eprintln!("neo-term: {why}");
            return ExitCode::FAILURE;
        }
    };
    // What term starts itself (the shell panel, git) runs there too.
    if let Err(err) = std::env::set_current_dir(&cwd) {
        eprintln!("neo-term: {cwd}: {err}");
        return ExitCode::FAILURE;
    }
    let started = driver::start(StartOptions {
        socket: None,
        cwd,
        home: std::env::var("HOME").ok(),
        no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        history: HistoryStore::in_data_dir(),
        prefs: crate::theme::Prefs::in_data_dir(),
        args,
    });
    let mut started = match started {
        Ok(started) => started,
        Err(StartError::DaemonDown) => {
            eprintln!("daemon not running: run neo (or neo up)");
            return ExitCode::FAILURE;
        }
        Err(StartError::Other(why)) => {
            eprintln!("neo-term: {why}");
            return ExitCode::FAILURE;
        }
    };
    // File type icons are Nerd Font glyphs; without such a font they are boxes.
    started
        .app
        .set_icons(std::env::var("NEO_TERM_ICONS").map_or(true, |v| v != "off"));
    match run(started.app, started.driver) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("neo-term: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Puts the terminal back when dropped, including while unwinding from a panic.
struct Screen {
    terminal: DefaultTerminal,
    /// What the terminal shows images with (None: `NEO_TERM_IMAGES=off`).
    picker: Option<Picker>,
}

impl Screen {
    fn enter() -> io::Result<Screen> {
        // `init` switches to the alternate screen, enables raw mode and installs a panic hook
        // that restores them; the hook is extended to end bracketed paste and mouse capture
        // too. The mouse is captured for the wheel; selecting text then takes shift-drag.
        let terminal = ratatui::init();
        // The terminal is asked what it can draw now: after raw mode and before any key or
        // mouse event can get mixed up with the answer.
        let mode = images::parse_mode(std::env::var(images::ENV).ok().as_deref());
        let picker = images::detect(mode);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
            let _ = plain_keys();
            previous(info);
        }));
        execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
        // Keys like Ctrl-Enter reported apart from Enter: kitty's keyboard protocol, and xterm's
        // modifyOtherKeys, which is what tmux (`extended-keys on`) listens for.
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        write!(io::stdout(), "{MODIFY_OTHER_KEYS_ON}")?;
        io::stdout().flush()?;
        Ok(Screen { terminal, picker })
    }
}

/// xterm's modifyOtherKeys, level 2, and back to the terminal's default.
const MODIFY_OTHER_KEYS_ON: &str = "\x1b[>4;2m";
const MODIFY_OTHER_KEYS_OFF: &str = "\x1b[>4m";

/// Gives the terminal its usual key reports back.
fn plain_keys() -> io::Result<()> {
    execute!(io::stdout(), PopKeyboardEnhancementFlags)?;
    write!(io::stdout(), "{MODIFY_OTHER_KEYS_OFF}")?;
    io::stdout().flush()
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = plain_keys();
        let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
        ratatui::restore();
    }
}

fn run(mut app: crate::app::App, driver: Driver) -> io::Result<()> {
    let mut screen = Screen::enter()?;
    if let Some(picker) = screen.picker.take() {
        // `asset://` handles are looked up with the daemon, and web images downloaded by it,
        // on the image thread.
        let client = driver.client().clone();
        let assets: Assets = std::sync::Arc::new(move |src| {
            if src.starts_with("asset://") {
                client
                    .get_asset(src)
                    .map(|asset| asset.path.into())
                    .map_err(|err| format!("asset lookup failed: {err}"))
            } else {
                client
                    .fetch_asset(src)
                    .map(|asset| asset.path.into())
                    .map_err(|err| format!("download failed: {err}"))
            }
        });
        let cwd = std::path::PathBuf::from(app.cwd());
        app.set_images(Images::new(picker, cwd, Some(assets)));
    }
    let size = screen.terminal.size()?;
    app.resize(size.width, size.height);
    loop {
        let mut quit = false;
        if event::poll(POLL)? {
            loop {
                let fx = match event::read()? {
                    Event::Key(key) => app.handle_key(key),
                    Event::Mouse(mouse) => {
                        app.handle_mouse(mouse);
                        Vec::new()
                    }
                    Event::Paste(text) => app.handle_paste(&text),
                    Event::Resize(w, h) => {
                        app.resize(w, h);
                        Vec::new()
                    }
                    _ => Vec::new(),
                };
                let out = driver.run(fx);
                quit |= out.quit;
                copy_to_clipboard(&out.copy)?;
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        let (out, _) = driver.pump(&mut app);
        quit |= out.quit;
        copy_to_clipboard(&out.copy)?;
        let fx = app.tick(now_ms());
        quit |= driver.run(fx).quit;
        if quit {
            return Ok(());
        }
        if app.take_clear() {
            screen.terminal.clear()?;
        }
        if app.take_dirty() {
            screen.terminal.draw(|frame| app.draw(frame))?;
        }
    }
}

fn copy_to_clipboard(texts: &[String]) -> io::Result<()> {
    let mut out = io::stdout();
    for text in texts {
        out.write_all(osc52(text).as_bytes())?;
    }
    out.flush()
}
