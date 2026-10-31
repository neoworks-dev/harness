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
/// What term asks of kitty's keyboard protocol: keys like Ctrl-Enter apart from Enter, and
/// repeats and releases, so holding Space to talk ends the moment it is let go (voice.rs).
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES);

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
    // Some 220 grammars take a moment to load: not on the first code block's draw.
    std::thread::spawn(neo_renderer_ratatui::syntax::warm_up);
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
        execute!(io::stdout(), PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))?;
        write!(io::stdout(), "{MODIFY_OTHER_KEYS_ON}")?;
        io::stdout().flush()?;
        Ok(Screen { terminal, picker })
    }
}

impl Screen {
    /// Hands the terminal to `f` (an editor) as the shell would have it, then takes it back.
    fn suspended<T>(&mut self, f: impl FnOnce() -> T) -> io::Result<T> {
        let _ = plain_keys();
        execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture)?;
        ratatui::restore();
        let out = f();
        ratatui::crossterm::terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            ratatui::crossterm::terminal::EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KEYBOARD_FLAGS)
        )?;
        write!(io::stdout(), "{MODIFY_OTHER_KEYS_ON}")?;
        io::stdout().flush()?;
        self.terminal.clear()?;
        Ok(out)
    }
}

/// Runs `$VISUAL` (else `$EDITOR`, else `vi`) on `path` at `line`, and waits for it.
fn edit(path: &std::path::Path, line: Option<usize>) -> Result<(), String> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|e| !e.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|e| !e.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".into());
    let argv = editor_argv(&editor, &path.to_string_lossy(), line);
    let (program, args) = argv.split_first().ok_or("no editor")?;
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

/// The editor's command line for a file at a line: `+N file` for vi, vim, nano, emacs and
/// most terminal editors, `file:N` for Helix, Zed and Sublime, `-g file:N` for VS Code's kind.
pub(crate) fn editor_argv(editor: &str, path: &str, line: Option<usize>) -> Vec<String> {
    let mut argv: Vec<String> = editor.split_whitespace().map(str::to_string).collect();
    let name = argv
        .first()
        .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
        .unwrap_or_default();
    match (line, name.as_str()) {
        (None, _) => argv.push(path.to_string()),
        (Some(n), "code" | "codium" | "code-insiders" | "cursor" | "windsurf") => {
            argv.extend(["-g".to_string(), format!("{path}:{n}")]);
        }
        (Some(n), "hx" | "helix" | "zed" | "zeditor" | "subl") => argv.push(format!("{path}:{n}")),
        (Some(n), _) => argv.extend([format!("+{n}"), path.to_string()]),
    }
    argv
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
                    Event::Mouse(mouse) => app.handle_mouse(mouse),
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
                if let Some((path, line)) = out.edit {
                    if let Err(why) = screen.suspended(|| edit(&path, line))? {
                        app.handle_msg(crate::app::Msg::Notice(why));
                    }
                    app.mark_dirty();
                    break;
                }
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

#[cfg(test)]
mod tests {
    use super::editor_argv;

    #[test]
    fn each_editor_is_told_the_line_its_way() {
        let argv = |e: &str, line| editor_argv(e, "/w/a.rs", line);
        assert_eq!(argv("nvim", Some(12)), ["nvim", "+12", "/w/a.rs"]);
        assert_eq!(
            argv("/usr/bin/vim", Some(3)),
            ["/usr/bin/vim", "+3", "/w/a.rs"]
        );
        assert_eq!(argv("hx", Some(12)), ["hx", "/w/a.rs:12"]);
        assert_eq!(
            argv("code --wait", Some(7)),
            ["code", "--wait", "-g", "/w/a.rs:7"]
        );
        assert_eq!(argv("nano", None), ["nano", "/w/a.rs"]);
    }
}
