//! Pad: a small GPUI app that hosts the agent's chat pane. A stand-in for Grove (the code
//! editor) while the bus client and the handoff are proven. Needs the suite daemon running.

mod field;
mod focus;
mod pad;
mod transcript;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use suite_protocol::{AppPresence, ConnectorKind, LaunchSpec};
use suite_sdk::{Client, Options};

/// Geist Regular, SIL Open Font License 1.1 (https://github.com/vercel/geist-font).
const FONT_DATA: &[u8] = include_bytes!("../assets/Geist-400.ttf");

fn presence(open: bool) -> AppPresence {
    AppPresence {
        app: pad::APP.into(),
        kind: ConnectorKind::App,
        title: Some("Pad".into()),
        open,
        context: None,
        mcp: None,
        // Lets the agent start Pad with apps_launch while it is closed.
        launch: std::env::current_exe().ok().map(|exe| LaunchSpec {
            command: vec![exe.to_string_lossy().into_owned()],
            cwd: None,
        }),
        home: None,
        pid: Some(i64::from(std::process::id())),
    }
}

fn main() {
    let mut options = Options::new(pad::APP);
    options.presence = Some(presence(true));
    options.reconnect = true;
    let client = match Client::connect(options) {
        Ok(client) => Arc::new(client),
        Err(err) => {
            eprintln!("suite-pad: {err}");
            std::process::exit(1);
        }
    };

    Application::new().run(move |cx| {
        if let Err(err) = cx.text_system().add_fonts(vec![Cow::Borrowed(FONT_DATA)]) {
            eprintln!("suite-pad: cannot load its font: {err}");
        }
        pad::bind_keys(cx);

        let bounds = Bounds::centered(None, size(px(560.), px(640.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| pad::Pad::new(client.clone(), cx));
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
                let _ = client.set_presence(presence(false));
            }
        })
        .detach();
        cx.activate(true);
    });
}
