//! `@` file mentions in the editor: the word at the cursor starting with `@` opens a list of
//! files to put there (found by `neo_mention`: `@` alone lists the session's directory, `@src/`
//! a folder in it, anything else is matched loosely). Picking a folder keeps the list open in
//! it; picking a file ends the word.
//!
//! `/cd ` completes its argument the same way, with folders only and no `@`; `~/` starts at
//! home there.

use std::path::Path;

pub use neo_mention::Candidate;
use neo_mention::{Finder, split};

/// The `@word` the cursor is in: where its `@` is, and what follows it up to the cursor; or
/// `/cd`'s argument up to the cursor, which completes to folders only.
#[derive(Debug, Clone, PartialEq)]
pub struct Mention {
    pub start: usize,
    pub query: String,
    /// `/cd`'s argument: folders only, and no `@` in front.
    pub cd: bool,
}

impl Mention {
    /// What goes before a picked path: `@`, or nothing for `/cd`.
    pub fn sigil(&self) -> &'static str {
        if self.cd { "" } else { "@" }
    }

    /// A `/cd` argument that names a folder already (`src/`, `..`, nothing): Enter runs the
    /// command rather than picking.
    pub fn complete(&self) -> bool {
        self.cd && split(&self.query).1.is_empty()
    }
}

/// The `@word` ending at `cursor` (a byte offset), when the `@` starts a word; or the argument
/// of a `/cd ` the text starts with, spaces and all.
pub fn at_cursor(text: &str, cursor: usize) -> Option<Mention> {
    let before = text.get(..cursor)?;
    if let Some(arg) = before.strip_prefix("/cd ") {
        return Some(Mention {
            start: "/cd ".len(),
            query: arg.to_string(),
            cd: true,
        });
    }
    let start = before.rfind(|c: char| c.is_whitespace()).map_or(0, |i| {
        i + before[i..].chars().next().map_or(1, char::len_utf8)
    });
    let word = &before[start..];
    let query = word.strip_prefix('@')?;
    // An e-mail address or a second `@` isn't a mention.
    if query.contains('@') {
        return None;
    }
    Some(Mention {
        start,
        query: query.to_string(),
        cd: false,
    })
}

#[derive(Debug, Default)]
pub struct Mentions(Finder);

impl Mentions {
    /// What to offer for `m` in a session working in `cwd`, with `home` for `~/`.
    pub fn candidates(&self, cwd: &Path, home: Option<&Path>, m: &Mention) -> Vec<Candidate> {
        self.0.candidates(cwd, home, &m.query, m.cd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_word_at_the_cursor_is_a_mention_when_it_starts_with_an_at() {
        assert_eq!(
            at_cursor("look at @src/ap", 15),
            Some(Mention {
                start: 8,
                query: "src/ap".into(),
                cd: false,
            })
        );
        assert_eq!(at_cursor("@", 1).map(|m| m.query), Some(String::new()));
        assert_eq!(at_cursor("@..", 3).map(|m| m.query), Some("..".into()));
        assert_eq!(at_cursor("mail me@x.de", 12), None);
        assert_eq!(at_cursor("@a done", 7), None, "the cursor left the word");
        assert_eq!(at_cursor("ü @a", 5).map(|m| m.start), Some(3));
    }

    #[test]
    fn a_cd_argument_completes_and_names_a_folder_once_it_ends_in_one() {
        let m = at_cursor("/cd my docs/sr", 14).unwrap();
        assert_eq!((m.start, m.query.as_str(), m.cd), (4, "my docs/sr", true));
        assert_eq!(m.sigil(), "");
        assert!(!m.complete(), "a name is still being typed");
        for done in ["/cd ", "/cd src/", "/cd ..", "/cd ~"] {
            assert!(at_cursor(done, done.len()).unwrap().complete(), "{done}");
        }
        assert!(!at_cursor("@src/", 5).unwrap().complete());
        assert_eq!(at_cursor("/cdx", 4), None);
    }
}
