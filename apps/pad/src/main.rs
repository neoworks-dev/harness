//! Pad: a small GPUI app that stands in for the code editor (Grove) while the suite is built:
//! open files with line numbers and diagnostics on the left, the agent's chat pane on the
//! right, and tools (`open`, `reveal`, `selection`, `files`) for the agent. Needs the suite
//! daemon running. `suite-pad [path[:line]…]` opens files at startup.

mod buffer;
mod editor;
mod field;
mod focus;
mod gateway;
mod pad;
#[cfg(test)]
mod tests;
mod tools;
mod view;

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use suite_sdk::{Client, Options, default_socket_path};

/// Geist Regular, SIL Open Font License 1.1 (https://github.com/vercel/geist-font).
const FONT_DATA: &[u8] = include_bytes!("../assets/Geist-400.ttf");

/// `path` or `path:line`, made absolute.
fn startup_file(arg: &str) -> (PathBuf, Option<usize>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    if let Some((name, line)) = arg.rsplit_once(':')
        && let Ok(line) = line.parse::<usize>()
    {
        return (editor::resolve_path(name, &cwd), Some(line));
    }
    (editor::resolve_path(arg, &cwd), None)
}

fn main() {
    let socket = default_socket_path();
    let (server, requests) = match tools::serve() {
        Ok(started) => started,
        Err(err) => {
            eprintln!("suite-pad: cannot start its tool server: {err}");
            std::process::exit(1);
        }
    };
    let endpoint = server.endpoint();

    let mut options = Options::new(pad::APP);
    options.socket = Some(socket.clone());
    options.presence = Some(pad::presence(true, Some(endpoint.clone()), None));
    options.reconnect = true;
    let client = match Client::connect(options) {
        Ok(client) => Arc::new(client),
        Err(err) => {
            eprintln!("suite-pad: {err}");
            std::process::exit(1);
        }
    };
    let setup = pad::Setup {
        socket,
        requests,
        endpoint: Some(endpoint),
        server: Some(server),
        open: std::env::args()
            .skip(1)
            .map(|arg| startup_file(&arg))
            .collect(),
    };

    Application::new().run(move |cx| {
        if let Err(err) = cx.text_system().add_fonts(vec![Cow::Borrowed(FONT_DATA)]) {
            eprintln!("suite-pad: cannot load its font: {err}");
        }
        pad::bind_keys(cx);

        let bounds = Bounds::centered(None, size(px(1120.), px(720.)), cx);
        let window_client = client.clone();
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(pad::TITLE.into()),
                    ..Default::default()
                }),
                app_id: Some("suite-pad".into()),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| pad::Pad::new(window_client, setup, window, cx));
                window.focus(&gpui::Focusable::focus_handle(&view, cx));
                view
            },
        );
        if let Err(err) = opened {
            eprintln!("suite-pad: cannot open a window: {err}");
            cx.quit();
            return;
        }

        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        // Unregister before the process ends, so the daemon moves the agent home right away
        // rather than when it notices the closed socket.
        let client = client.clone();
        cx.on_app_quit(move |_| {
            let client = client.clone();
            async move {
                let _ = client.set_presence(pad::presence(false, None, None));
            }
        })
        .detach();
        cx.activate(true);
    });
}
