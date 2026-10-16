//! The suite's full-screen terminal app. `app` is the shell (chat, editor, status lines,
//! keys); `widgets` are the side panels (plan, files/git, usage), each a pure state plus a
//! draw function.

pub mod app;
pub mod widgets;
