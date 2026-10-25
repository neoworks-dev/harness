//! GitHub, read-only, for neo's apps (the terminal's and the desktop's): what to ask the
//! `gh` CLI for ([`gh`], which refuses anything that could write), what it answers
//! ([`model`]) and the text block that hands an issue or pull request to the model
//! ([`format`]). [`fetch`] runs the usual asks end to end.

pub mod fetch;
pub mod format;
pub mod gh;
pub mod model;
mod text;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

pub use text::strip_comments;

#[cfg(test)]
mod tests;
