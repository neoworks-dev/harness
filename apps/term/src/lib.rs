//! Neo's full-screen terminal app. `app` is the state (chat, editor, overlays, keys) and
//! its drawing; `driver` connects it to the daemon; `runtime` owns the terminal. `widgets` are
//! the side panels (plan, files/git, usage), each a pure state plus a draw function.

pub mod agents;
pub mod app;
pub mod attach;
pub mod chat;
pub mod choice;
pub mod details;
pub mod driver;
pub mod editor;
pub mod forms;
pub mod github;
pub mod history;
pub mod images;
pub mod jobs;
pub mod markdown;
pub mod mention;
pub mod menu;
pub mod reader;
pub mod runtime;
pub mod secret;
pub mod sessions;
pub mod status;
pub mod style;
pub mod theme;
pub mod toast;
pub mod util;
pub mod voice;
pub mod widgets;
