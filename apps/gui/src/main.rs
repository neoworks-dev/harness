//! `suite-gui`: the shared conversation in a native window. The UI is a Svelte app
//! (`apps/gui/ui`) run by svelte-gpui; this program adds what the UI can't do itself, as
//! commands it calls with `invoke`.
//!
//! ```text
//! bun apps/gui/build.mjs
//! cargo run --release -p suite-gui -- --bundle apps/gui/dist/gui.js
//! ```

mod clipboard;
mod commands;
mod desktop;
mod git;
mod github;
mod workspace;

use svelte_gpui::Options;

fn main() {
    let options = Options::from_command_line().unwrap_or_else(|| {
        eprintln!("usage: suite-gui --bundle apps/gui/dist/gui.js");
        std::process::exit(2);
    });
    svelte_gpui::run(options.commands(commands::all()));
}
