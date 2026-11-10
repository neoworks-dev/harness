//! Completing GitHub search syntax while it is typed: the qualifier (`label:`, `assignee:`…)
//! and then its value, from the repository's labels and people.

use super::model::{Kind, Label, Person};

/// Most candidates shown at once.
pub const MAX: usize = 8;

/// What a candidate is, which decides how it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    /// A qualifier to go on with (`label:`).
    Key,
    /// A repository label in its own color (`0xRRGGBB`).
    Label(u32),
    /// Somebody, with the address of a small picture of them if there is one.
    Person(Option<String>),
    /// A fixed value (`open`, `updated-desc`).
    Word,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// What the token becomes when this is accepted.
    pub insert: String,
    /// What the list shows.
    pub shown: String,
    pub look: Look,
    /// A muted word after it: a label's description.
    pub note: String,
}

/// Qualifiers worth offering, with whether they take people.
const KEYS: &[&str] = &[
    "label:", "assignee:", "author:", "mentions:", "involves:", "is:", "no:", "sort:",
];

const PEOPLE_KEYS: &[&str] = &["assignee", "author", "mentions", "involves", "commenter"];

/// Where the last word of `text` starts; a quoted value may hold spaces.
pub fn token_start(text: &str) -> usize {
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            start = i + c.len_utf8();
        }
    }
    start
}

/// Candidates for the last word of `text`. `kind` limits `is:` to what fits the tab.
pub fn suggest(text: &str, kind: Kind, labels: &[Label], people: &[Person]) -> Vec<Candidate> {
    let token = &text[token_start(text)..];
    let (negated, token) = match token.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", token),
    };
    let Some((key, value)) = token.split_once(':') else {
        return keys(negated, token);
    };
    let value = value.trim_start_matches('"');
    let key = key.to_ascii_lowercase();
    let make = |shown: &str, look: Look, note: &str| Candidate {
        insert: format!("{negated}{key}:{}", quote(shown)),
        shown: shown.to_string(),
        look,
        note: note.to_string(),
    };
    let mut out: Vec<Candidate> = match key.as_str() {
        "label" => ranked(labels.iter().map(|l| (l.name.as_str(), l)), value)
            .into_iter()
            .map(|l| make(&l.name, Look::Label(l.rgb), &l.description))
            .collect(),
        k if PEOPLE_KEYS.contains(&k) => {
            let me = [("@me", ("@me", None))].into_iter();
            let all = people.iter().map(|p| {
                let avatar = Some(p.avatar_url.as_str()).filter(|u| !u.is_empty());
                (p.login.as_str(), (p.login.as_str(), avatar))
            });
            ranked(me.chain(all), value)
                .into_iter()
                .map(|(name, avatar)| make(name, Look::Person(avatar.map(avatar_size)), ""))
                .collect()
        }
        "is" => {
            let words: &[&str] = match kind {
                Kind::Issue => &["open", "closed"],
                Kind::Pr => &["open", "closed", "merged", "draft"],
            };
            words_for(&make, words, value)
        }
        "no" => words_for(&make, &["assignee", "label", "milestone", "project"], value),
        "sort" => words_for(
            &make,
            &[
                "updated-desc",
                "updated-asc",
                "created-desc",
                "created-asc",
                "comments-desc",
            ],
            value,
        ),
        _ => Vec::new(),
    };
    // Nothing left to complete once the value is exactly the only candidate.
    if out.len() == 1 && out[0].shown.eq_ignore_ascii_case(value) {
        out.clear();
    }
    out.truncate(MAX);
    out
}

fn keys(negated: &str, prefix: &str) -> Vec<Candidate> {
    let prefix = prefix.to_ascii_lowercase();
    KEYS.iter()
        .filter(|k| k.starts_with(&prefix))
        .map(|k| Candidate {
            insert: format!("{negated}{k}"),
            shown: (*k).to_string(),
            look: Look::Key,
            note: String::new(),
        })
        .collect()
}

fn words_for(
    make: &impl Fn(&str, Look, &str) -> Candidate,
    words: &[&str],
    value: &str,
) -> Vec<Candidate> {
    ranked(words.iter().map(|w| (*w, *w)), value)
        .into_iter()
        .map(|w| make(w, Look::Word, ""))
        .collect()
}

/// The items whose name starts with, or else contains, `query` (any case), in that order.
fn ranked<'a, T>(items: impl Iterator<Item = (&'a str, T)>, query: &str) -> Vec<T> {
    let q = query.to_lowercase();
    let (mut starts, mut contains) = (Vec::new(), Vec::new());
    for (name, item) in items {
        let n = name.to_lowercase();
        if n.starts_with(&q) || n.trim_start_matches('@').starts_with(&q) {
            starts.push(item);
        } else if n.contains(&q) {
            contains.push(item);
        }
    }
    starts.extend(contains);
    starts
}

/// The avatar at a size a thumbnail needs (GitHub scales it with `s`).
fn avatar_size(url: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}s=64")
}

/// A value in quotes when it has a space or a colon.
fn quote(value: &str) -> String {
    if value.contains(char::is_whitespace) || value.contains(':') {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

/// `text` with its last word replaced by `candidate`; a qualifier stays open for its value,
/// anything else is finished with a space.
pub fn accept(text: &str, candidate: &Candidate) -> String {
    let mut out = text[..token_start(text)].to_string();
    out.push_str(&candidate.insert);
    if candidate.look != Look::Key {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(name: &str) -> Label {
        Label {
            name: name.into(),
            rgb: 0xff0000,
            description: String::new(),
        }
    }

    fn person(login: &str) -> Person {
        Person {
            login: login.into(),
            avatar_url: String::new(),
        }
    }

    fn inserts(c: &[Candidate]) -> Vec<&str> {
        c.iter().map(|c| c.insert.as_str()).collect()
    }

    #[test]
    fn labels_match_by_prefix_then_substring_and_are_quoted() {
        let labels = [label("area:neo"), label("bug"), label("needs info")];
        let s = suggest("foo label:ne", Kind::Issue, &labels, &[]);
        assert_eq!(inserts(&s), ["label:\"needs info\"", "label:\"area:neo\""]);
        assert_eq!(accept("foo label:ne", &s[0]), "foo label:\"needs info\" ");
    }

    #[test]
    fn a_quoted_value_may_hold_spaces() {
        let labels = [label("needs info")];
        let s = suggest("x label:\"needs i", Kind::Issue, &labels, &[]);
        assert_eq!(accept("x label:\"needs i", &s[0]), "x label:\"needs info\" ");
    }

    #[test]
    fn people_start_with_me_and_keep_a_minus() {
        let people = [person("moritz"), person("neoworks-bot")];
        let s = suggest("assignee:", Kind::Issue, &[], &people);
        assert_eq!(inserts(&s), ["assignee:@me", "assignee:moritz", "assignee:neoworks-bot"]);
        let s = suggest("-assignee:mo", Kind::Issue, &[], &people);
        assert_eq!(inserts(&s), ["-assignee:moritz"]);
    }

    #[test]
    fn a_qualifier_opens_for_its_value_and_a_finished_value_offers_nothing() {
        let s = suggest("la", Kind::Issue, &[], &[]);
        assert_eq!(accept("la", &s[0]), "label:");
        assert!(suggest("label:bug", Kind::Issue, &[label("bug")], &[]).is_empty());
        assert!(suggest("menu", Kind::Issue, &[], &[]).is_empty());
    }

    #[test]
    fn is_depends_on_the_tab() {
        assert_eq!(suggest("is:", Kind::Issue, &[], &[]).len(), 2);
        assert_eq!(suggest("is:", Kind::Pr, &[], &[]).len(), 4);
    }
}
