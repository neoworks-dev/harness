//! `neo-gui`: the shared conversation in a native window. The UI is a Svelte app
//! (`apps/gui/ui`) run by svelte-gpui; this program adds what the UI can't do itself, as
//! commands it calls with `invoke`.
//!
//! ```text
//! bun apps/gui/build.mjs
//! cargo run --release -p neo-gui -- --bundle apps/gui/dist/gui.js
//! ```
//!
//! With `--web DIR` it opens no window: it serves the UI built for a browser (see [`web`]).
//!
//! ```text
//! bun apps/gui/build.mjs --web
//! cargo run --release -p neo-gui -- --web apps/gui/dist/web
//! ```

mod clipboard;
mod commands;
mod desktop;
mod git;
mod github;
mod web;
mod workspace;

use svelte_gpui::Options;

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(dir) = web::directory(&arguments) {
        web::run(&dir, commands::all());
    }
    let options = Options::from_command_line().unwrap_or_else(|| {
        eprintln!(
            "usage: neo-gui --bundle apps/gui/dist/gui.js\n       neo-gui --web apps/gui/dist/web"
        );
        std::process::exit(2);
    });
    svelte_gpui::run(options.commands(commands::all()));
}
