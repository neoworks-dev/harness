//! Syntax highlighting for the file and diff view: syntect's grammars (its default set, chosen
//! by file name) parse each line into scopes, and the scopes become a few classes the palette
//! colors, so code takes the colors of the app's theme instead of a syntax theme of its own.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

/// What a stretch of code is, as far as colors go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Class {
    #[default]
    Plain,
    Comment,
    String,
    Keyword,
    /// Numbers and constants (`true`, `None`).
    Constant,
    /// A function's name where it is defined or called.
    Function,
    Type,
}

/// Lines beyond this many are drawn without highlighting: a huge file stays quick to open.
pub(crate) const MAX_LINES: usize = 20_000;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The scopes each class starts with, most specific first (the first match wins).
fn classify(stack: &ScopeStack) -> Class {
    static RULES: OnceLock<Vec<(Scope, Class)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            ("comment", Class::Comment),
            ("string", Class::String),
            ("constant.character.escape", Class::String),
            ("constant", Class::Constant),
            ("entity.name.function", Class::Function),
            ("support.function", Class::Function),
            ("variable.function", Class::Function),
            ("entity.name.type", Class::Type),
            ("entity.name.class", Class::Type),
            ("entity.name.struct", Class::Type),
            ("entity.name.enum", Class::Type),
            ("support.type", Class::Type),
            ("support.class", Class::Type),
            ("storage.type", Class::Keyword),
            ("storage.modifier", Class::Keyword),
            ("keyword.operator", Class::Plain),
            ("keyword", Class::Keyword),
        ]
        .into_iter()
        .filter_map(|(name, class)| Scope::new(name).ok().map(|s| (s, class)))
        .collect()
    });
    // The innermost scope decides: a string inside a comment's code example is a string.
    for scope in stack.as_slice().iter().rev() {
        if let Some((_, class)) = rules.iter().find(|(prefix, _)| prefix.is_prefix_of(*scope)) {
            return *class;
        }
    }
    Class::Plain
}

/// The grammar for a file, by its name; None when there is none (or it is plain text).
pub(crate) fn syntax_for(path: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map_or(name, |(_, ext)| ext);
    set.find_syntax_by_extension(ext)
        .or_else(|| set.find_syntax_by_extension(name))
        .or_else(|| match ext {
            // Not in syntect's default set: the nearest that is.
            "ts" | "tsx" | "mts" | "cts" | "jsx" | "mjs" | "cjs" => {
                set.find_syntax_by_extension("js")
            }
            "toml" => set.find_syntax_by_extension("ini"),
            "svelte" | "vue" => set.find_syntax_by_extension("html"),
            _ => None,
        })
        .filter(|s| s.name != "Plain Text")
}

/// Highlights one version of a file line by line. A hunk of a diff starts with `reset`: the
/// lines before it aren't there, so the parse starts over.
pub(crate) struct Highlighter {
    syntax: &'static SyntaxReference,
    state: ParseState,
    stack: ScopeStack,
}

impl Highlighter {
    pub(crate) fn new(syntax: &'static SyntaxReference) -> Highlighter {
        Highlighter {
            syntax,
            state: ParseState::new(syntax),
            stack: ScopeStack::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Highlighter::new(self.syntax);
    }

    /// The class of each char of `line` (without its newline).
    pub(crate) fn line(&mut self, line: &str) -> Vec<Class> {
        let text = format!("{line}\n");
        let Ok(ops) = self.state.parse_line(&text, syntaxes()) else {
            self.reset();
            return vec![Class::Plain; line.chars().count()];
        };
        let mut out = Vec::with_capacity(line.len());
        let mut at = 0;
        let mut push_upto = |end: usize, class: Class, out: &mut Vec<Class>| {
            let end = end.min(line.len());
            if end > at {
                out.extend(std::iter::repeat_n(class, line[at..end].chars().count()));
                at = end;
            }
        };
        for (offset, op) in ops {
            push_upto(offset, classify(&self.stack), &mut out);
            if self.stack.apply(&op).is_err() {
                self.stack = ScopeStack::new();
            }
        }
        push_upto(line.len(), classify(&self.stack), &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(path: &str, line: &str) -> Vec<(String, Class)> {
        let mut h = Highlighter::new(syntax_for(path).expect("a grammar"));
        let classes = h.line(line);
        let mut runs: Vec<(String, Class)> = Vec::new();
        for (c, class) in line.chars().zip(classes) {
            match runs.last_mut() {
                Some((text, last)) if *last == class => text.push(c),
                _ => runs.push((c.to_string(), class)),
            }
        }
        runs.into_iter()
            .filter(|(t, _)| !t.trim().is_empty())
            .collect()
    }

    #[test]
    fn rust_keywords_strings_comments_and_functions() {
        let got = classes("src/a.rs", r#"fn main() { let s = "hi"; } // done"#);
        assert!(got.contains(&("fn".into(), Class::Keyword)), "{got:?}");
        assert!(got.contains(&("main".into(), Class::Function)), "{got:?}");
        assert!(got.contains(&("\"hi\"".into(), Class::String)), "{got:?}");
        assert!(
            got.iter()
                .any(|(t, c)| t.contains("// done") && *c == Class::Comment),
            "{got:?}"
        );
    }

    #[test]
    fn typescript_uses_the_javascript_grammar_and_unknown_files_have_none() {
        let got = classes("ui/app.ts", "const n = 42;");
        assert!(got.contains(&("const".into(), Class::Keyword)), "{got:?}");
        assert!(got.contains(&("42".into(), Class::Constant)), "{got:?}");
        assert!(syntax_for("notes.unknownext").is_none());
        assert!(syntax_for("README.txt").is_none());
    }

    #[test]
    fn every_char_gets_a_class_and_state_carries_across_lines() {
        let mut h = Highlighter::new(syntax_for("a.rs").unwrap());
        let first = h.line("/* a ü");
        assert_eq!(first.len(), "/* a ü".chars().count());
        // Still inside the comment on the next line; after a reset it isn't.
        assert!(h.line("b */").iter().take(1).all(|c| *c == Class::Comment));
        h.line("/* again");
        h.reset();
        assert!(h.line("let x = 1;").contains(&Class::Keyword));
    }
}
