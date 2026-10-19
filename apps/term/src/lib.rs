//! The suite's full-screen terminal app. `app` is the state (chat, editor, overlays, keys) and
//! its drawing; `driver` connects it to the daemon; `runtime` owns the terminal. `widgets` are
//! the side panels (plan, files/git, usage), each a pure state plus a draw function.

pub mod agents;
pub mod app;
pub mod chat;
pub mod driver;
pub mod editor;
pub mod github;
pub mod history;
pub mod images;
pub mod menu;
pub mod runtime;
pub mod secret;
pub mod sessions;
pub mod status;
pub mod style;
pub mod util;
pub mod widgets;
