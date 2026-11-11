//! `neo-gui --web DIR`: the same app in a browser tab. Instead of a window it starts svelte-gpui's
//! browser target, which serves the web build in `DIR` (`bun apps/gui/build.mjs --web`) and answers
//! the page's `invoke`, `files` and `fetch` calls over a WebSocket with the very commands the window
//! has, then opens the default browser on it and keeps serving.
//!
//! The page reaches the daemon itself, with the address and token `daemon_address` gives it. The
//! host has a token of its own, which opens the page's WebSocket and is only in the URL's
//! fragment: it is never printed to the log, except when no browser could be opened.

use std::path::{Path, PathBuf};

use svelte_gpui::{Commands, ServeOptions, serve};

use crate::desktop;

/// The directory after `--web`, when the command line has one.
pub fn directory(arguments: &[String]) -> Option<PathBuf> {
    let at = arguments.iter().position(|argument| argument == "--web")?;
    arguments.get(at + 1).map(PathBuf::from)
}

/// Serves `dir` until the process is stopped.
pub fn run(dir: &Path, commands: Commands) -> ! {
    if !dir.join("index.html").is_file() {
        eprintln!(
            "neo-gui: no web build in {}: run bun apps/gui/build.mjs --web",
            dir.display()
        );
        std::process::exit(1);
    }
    let server = match serve(ServeOptions::new(dir).commands(commands)) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("neo-gui: cannot serve {}: {error}", dir.display());
            std::process::exit(1);
        }
    };
    println!("serving {} on 127.0.0.1:{}", dir.display(), server.port());
    if let Err(error) = desktop::open_url(&server.url()) {
        // The only time the token is written down: without it the page cannot be reached.
        eprintln!(
            "neo-gui: could not open a browser ({error}); open {}",
            server.url()
        );
    }
    server.wait();
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|argument| argument.to_string()).collect()
    }

    #[test]
    fn the_directory_follows_the_flag() {
        assert_eq!(
            directory(&args(&["neo-gui", "--web", "dist/web"])),
            Some(PathBuf::from("dist/web"))
        );
    }

    #[test]
    fn without_the_flag_or_a_directory_it_is_the_window() {
        assert_eq!(directory(&args(&["neo-gui", "--bundle", "gui.js"])), None);
        assert_eq!(directory(&args(&["neo-gui", "--web"])), None);
    }
}
