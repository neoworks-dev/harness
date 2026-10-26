//! Syntax highlighting for code, diffs and files: syntect parses each line into scopes with
//! bat's grammars (`two-face`, some 220 languages), and the scopes become a few classes that
//! [`code_colors`] colors for a dark or a light background.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::Palette;

/// A grammar, as [`syntax_for`] finds it.
pub use syntect::parsing::SyntaxReference;

/// What a stretch of code is, as far as colors go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Class {
    #[default]
    Plain,
    Comment,
    String,
    Keyword,
    /// Numbers and constants (`true`, `None`), a shell's `$VAR` and `-flag`.
    Constant,
    /// A function's name where it is defined or called; a shell's command.
    Function,
    Type,
    /// Markup: a heading and its `#`.
    Heading,
    /// Markup: `**strong**`.
    Strong,
    /// Markup: `*emphasis*`.
    Emphasis,
    /// Markup: a link's target.
    Link,
    /// Markup: `> a quote`.
    Quote,
    /// A diff's added line.
    Inserted,
    /// A diff's removed line.
    Deleted,
}

/// How many classes there are: the length of [`code_colors`].
pub const CLASSES: usize = 14;

/// A diff in the chat with more lines than this is drawn without highlighting: it is laid out
/// whole, unlike the file view, which highlights only what is on screen.
pub const MAX_LINES: usize = 20_000;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Loads the grammars now (it takes a moment), so the first highlighted file doesn't wait.
pub fn warm_up() {
    syntaxes();
}

/// The scopes each class starts with, most specific first (the first match wins).
fn classify(stack: &ScopeStack) -> Class {
    static RULES: OnceLock<Vec<(Scope, Class)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            ("comment", Class::Comment),
            ("markup.heading", Class::Heading),
            ("entity.name.section", Class::Heading),
            ("punctuation.definition.heading", Class::Heading),
            ("markup.bold", Class::Strong),
            ("markup.italic", Class::Emphasis),
            ("markup.raw", Class::String),
            ("markup.underline.link", Class::Link),
            ("meta.link", Class::Link),
            ("markup.quote", Class::Quote),
            ("punctuation.definition.list_item", Class::Keyword),
            ("markup.inserted", Class::Inserted),
            ("markup.deleted", Class::Deleted),
            ("variable.other.readwrite.shell", Class::Constant),
            ("punctuation.definition.variable.shell", Class::Constant),
            ("variable.parameter.option", Class::Constant),
            ("string", Class::String),
            ("constant.character.escape", Class::String),
            ("constant", Class::Constant),
            ("entity.name.function", Class::Function),
            ("support.function", Class::Function),
            ("variable.function", Class::Function),
            ("entity.name.tag", Class::Keyword),
            ("entity.other.attribute-name", Class::Type),
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
pub fn syntax_for(path: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map_or(name, |(_, ext)| ext);
    set.find_syntax_by_extension(ext)
        .or_else(|| set.find_syntax_by_extension(name))
        .or_else(|| set.find_syntax_by_extension(&ext.to_ascii_lowercase()))
        .or_else(|| match ext {
            "jsonc" | "json5" => set.find_syntax_by_extension("json"),
            _ => None,
        })
        .filter(|s| s.name != "Plain Text")
}

/// The grammar for a code block's language (` ```ts `, ` ```shell `): a name, an alias or an
/// extension; None when there is none.
pub fn syntax_for_language(language: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let lang = language.trim().to_ascii_lowercase();
    let lang = match lang.as_str() {
        "shell" | "sh" | "zsh" | "console" | "shell-session" | "shellscript" => "bash",
        "js" | "javascript" | "node" => "js",
        "ts" | "typescript" => "ts",
        "py" | "python3" => "python",
        "yml" => "yaml",
        "text" | "txt" | "plain" | "plaintext" => return None,
        other => other,
    };
    set.find_syntax_by_extension(lang)
        .or_else(|| set.find_syntax_by_token(lang))
        .filter(|s| s.name != "Plain Text")
}

/// Highlights one version of a file line by line. A hunk of a diff starts with `reset`: the
/// lines before it aren't there, so the parse starts over.
#[derive(Clone)]
pub struct Highlighter {
    syntax: &'static SyntaxReference,
    state: ParseState,
    stack: ScopeStack,
}

impl Highlighter {
    pub fn new(syntax: &'static SyntaxReference) -> Highlighter {
        Highlighter {
            syntax,
            state: ParseState::new(syntax),
            stack: ScopeStack::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Highlighter::new(self.syntax);
    }

    /// The class of each char of `line` (without its newline).
    pub fn line(&mut self, line: &str) -> Vec<Class> {
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

/// `source` as styled spans, newlines kept and the text unchanged: highlighted by `syntax` in
/// `colors` (see [`block_colors`]), each patched over `base`.
pub fn highlight_spans(
    source: &str,
    syntax: &'static SyntaxReference,
    colors: &[Style; CLASSES],
    base: Style,
) -> Vec<Span<'static>> {
    let mut h = Highlighter::new(syntax);
    let mut out: Vec<Span<'static>> = Vec::new();
    for (i, line) in source.split('\n').enumerate() {
        if i > 0 {
            crate::text::push_text(&mut out, "\n", base);
        }
        let classes = h.line(line);
        let mut run = String::new();
        let mut run_class = None;
        for (c, class) in line.chars().zip(classes) {
            if run_class != Some(class) && !run.is_empty() {
                let style = base.patch(colors[run_class.unwrap_or_default() as usize]);
                crate::text::push_text(&mut out, &std::mem::take(&mut run), style);
            }
            run_class = Some(class);
            run.push(c);
        }
        if !run.is_empty() {
            let style = base.patch(colors[run_class.unwrap_or_default() as usize]);
            crate::text::push_text(&mut out, &run, style);
        }
    }
    out
}

/// Colors of code by [`Class`], picked for a dark or a light background rather than taken from
/// the theme: its few colors (an accent as green as its success color) can't tell five classes
/// apart. None without true colors.
pub fn code_colors(p: &Palette) -> Option<[Style; CLASSES]> {
    let (Color::Rgb(r, g, b), Color::Rgb(..)) = (p.bg, p.fg) else {
        return None;
    };
    let dark = u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 < 128_000;
    let rgb = |c: u32| Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let [keyword, string, constant, function, kind] = if dark {
        [0xc7_92ea, 0xc3_e88d, 0xf7_8c6c, 0x82_aaff, 0xff_cb6b]
    } else {
        [0x7c_3aed, 0x1a_7f37, 0xb4_5309, 0x1f_5fcc, 0x9a_6700]
    };
    let fg = |c: u32| Style::new().fg(rgb(c));
    let bold = Modifier::BOLD;
    // In the order of `Class`.
    Some([
        Style::new().fg(p.fg),
        Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
        fg(string),
        fg(keyword),
        fg(constant),
        fg(function),
        fg(kind),
        fg(keyword).add_modifier(bold),
        Style::new().add_modifier(bold),
        Style::new().add_modifier(Modifier::ITALIC),
        fg(function).add_modifier(Modifier::UNDERLINED),
        Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
        Style::new().fg(p.success),
        Style::new().fg(p.danger),
    ])
}

/// Colors for a code block: [`code_colors`] with true colors, else the theme's few (plain
/// modifiers with `NO_COLOR`), so a command still reads as one.
pub fn block_colors(p: &Palette) -> [Style; CLASSES] {
    if let Some(colors) = code_colors(p) {
        return colors;
    }
    let fg = |c: Color| Style::new().fg(c);
    let (bold, italic) = (Modifier::BOLD, Modifier::ITALIC);
    [
        Style::new(),
        fg(p.muted).add_modifier(italic),
        fg(p.success),
        fg(p.accent),
        fg(p.accent),
        fg(p.accent).add_modifier(bold),
        fg(p.accent),
        fg(p.accent).add_modifier(bold),
        Style::new().add_modifier(bold),
        Style::new().add_modifier(italic),
        fg(p.accent).add_modifier(Modifier::UNDERLINED),
        fg(p.muted).add_modifier(italic),
        fg(p.success),
        fg(p.danger),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(h: &mut Highlighter, line: &str) -> Vec<(String, Class)> {
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

    fn classes(path: &str, line: &str) -> Vec<(String, Class)> {
        runs(
            &mut Highlighter::new(syntax_for(path).expect("a grammar")),
            line,
        )
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
    fn languages_beyond_syntects_own_have_their_grammars() {
        for (path, name) in [
            ("ui/app.ts", "TypeScript"),
            ("ui/App.tsx", "TypeScriptReact"),
            ("Cargo.toml", "TOML"),
            ("src/App.svelte", "Svelte"),
            ("src/App.vue", "Vue Component"),
            ("Dockerfile", "Dockerfile"),
            ("flake.nix", "Nix"),
        ] {
            assert_eq!(
                syntax_for(path).map(|s| s.name.as_str()),
                Some(name),
                "{path}"
            );
        }
        assert!(syntax_for("notes.unknownext").is_none());
        assert!(syntax_for("README.txt").is_none());
        let got = classes("ui/app.ts", "const n: number = 42;");
        assert!(got.contains(&("const".into(), Class::Keyword)), "{got:?}");
        assert!(got.contains(&("42".into(), Class::Constant)), "{got:?}");
    }

    #[test]
    fn code_block_languages_and_their_aliases() {
        let name = |l: &str| syntax_for_language(l).map(|s| s.name.clone());
        assert_eq!(name("rust"), Some("Rust".into()));
        assert_eq!(name("shell"), name("bash"));
        assert_eq!(name("console"), name("bash"));
        assert_eq!(name("ts"), Some("TypeScript".into()));
        assert_eq!(name("Python"), Some("Python".into()));
        assert_eq!(name("text"), None);
        assert_eq!(name("nonsense-lang"), None);
    }

    #[test]
    fn markdown_has_headings_emphasis_code_and_links() {
        let mut h = Highlighter::new(syntax_for("README.md").unwrap());
        let head = runs(&mut h, "# Title");
        assert!(head.iter().all(|(_, c)| *c == Class::Heading), "{head:?}");
        let got = runs(&mut h, "Some *em*, **strong**, `code` and [a](http://x)");
        assert!(got.contains(&("*em*".into(), Class::Emphasis)), "{got:?}");
        assert!(
            got.contains(&("**strong**".into(), Class::Strong)),
            "{got:?}"
        );
        assert!(got.contains(&("`code`".into(), Class::String)), "{got:?}");
        assert!(
            got.contains(&("[a](http://x)".into(), Class::Link)),
            "{got:?}"
        );
    }

    #[test]
    fn shell_commands_variables_strings_and_comments() {
        let mut h = Highlighter::new(syntax_for_language("bash").unwrap());
        let got = runs(&mut h, r#"ls -la "$HOME/x" | grep foo # look"#);
        assert!(got.contains(&("ls".into(), Class::Function)), "{got:?}");
        assert!(got.contains(&("grep".into(), Class::Function)), "{got:?}");
        assert!(got.contains(&("$HOME".into(), Class::Constant)), "{got:?}");
        assert!(got.contains(&("/x\"".into(), Class::String)), "{got:?}");
        assert!(
            got.iter()
                .any(|(t, c)| t.contains("look") && *c == Class::Comment)
        );
    }

    #[test]
    fn spans_keep_the_text_and_its_newlines() {
        let colors = block_colors(&Palette::default());
        let src = "echo hi\n\nls  -a";
        let spans = highlight_spans(
            src,
            syntax_for_language("sh").unwrap(),
            &colors,
            Style::new(),
        );
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, src);
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
