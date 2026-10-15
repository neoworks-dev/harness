//! GPUI renderer for the suite's resolved view format. See the README.
//!
//! Two layers: [`layout`] turns a `ViewItem` plus a theme into a tree of boxes, text runs and
//! leaf shapes (pure, snapshot-tested through [`dump`]), and [`ChatView`] draws that tree with
//! GPUI and handles keys, clicks, patches and scrolling.

mod chat;
mod draw;
mod dump;
pub mod layout;
mod theme;

pub use chat::{ChatEvent, ChatView};
pub use draw::ChatFonts;
pub use dump::dump_items;
pub use layout::{ButtonEffect, ButtonSpec, ItemLayout, layout_item, layout_items};
pub use theme::{ChatTheme, Paint, Token, default_theme};
