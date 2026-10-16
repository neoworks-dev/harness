//! The suite's full-screen terminal app. `app` is the state (chat, editor, overlays, keys) and
//! its drawing; `driver` connects it to the daemon; `runtime` owns the terminal. `widgets` are
//! the side panels (plan, files/git, usage), each a pure state plus a draw function.

pub mod app;
pub mod chat;
pub mod driver;
pub mod editor;
pub mod history;
pub mod menu;
pub mod runtime;
pub mod sessions;
pub mod status;
pub mod style;
pub mod util;
pub mod widgets;
