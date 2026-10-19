//! Shell commands as styled spans: command names, strings, variables, operators and comments.
//! A tokenizer good enough for one-liners and short scripts, not a parser: whatever it doesn't
//! know is plain text, and the text itself is never changed.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::Palette;
use crate::text::push_text;

/// The languages a code block is highlighted as shell for.
pub(crate) fn is_shell(language: Option<&str>) -> bool {
    matches!(
        language.map(str::to_ascii_lowercase).as_deref(),
        Some("bash" | "sh" | "shell" | "zsh" | "console" | "shell-session")
    )
}

/// Words after which the next word is a command again (`sudo rm`, `xargs grep`).
const PREFIXES: &[&str] = &[
    "sudo", "time", "env", "exec", "nohup", "xargs", "command", "builtin", "then", "do", "else",
    "if", "elif", "while", "until", "!",
];

#[derive(Clone, Copy)]
struct Styles {
    command: Style,
    string: Style,
    variable: Style,
    operator: Style,
    comment: Style,
}

/// Styled spans for `source`, newlines kept. `base` is patched under every span (the code
/// block's colors).
pub(crate) fn highlight(source: &str, p: &Palette, base: Style) -> Vec<Span<'static>> {
    let st = Styles {
        command: base.fg(p.accent).add_modifier(Modifier::BOLD),
        string: base.fg(p.success),
        variable: base.fg(p.accent),
        operator: base.fg(p.muted),
        comment: base.fg(p.muted).add_modifier(Modifier::ITALIC),
    };
    let chars: Vec<char> = source.chars().collect();
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut i = 0;
    // The next word is a command name.
    let mut command_next = true;
    let emit = |out: &mut Vec<Span<'static>>, text: &[char], style: Style| {
        push_text(out, &text.iter().collect::<String>(), style);
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                emit(&mut out, &chars[i..=i], base);
                command_next = true;
                i += 1;
            }
            c if c.is_whitespace() => {
                let start = i;
                while i < chars.len() && chars[i].is_whitespace() && chars[i] != '\n' {
                    i += 1;
                }
                emit(&mut out, &chars[start..i], base);
            }
            '#' => {
                let start = i;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                emit(&mut out, &chars[start..i], st.comment);
            }
            '|' | '&' | ';' | '(' | ')' | '<' | '>' | '`' => {
                let start = i;
                while i < chars.len() && matches!(chars[i], '|' | '&' | ';' | '<' | '>') {
                    i += 1;
                }
                if i == start {
                    i += 1;
                }
                // `2>&1` and `> file` redirect; the rest separate commands.
                let op: String = chars[start..i].iter().collect();
                command_next = !op.contains('>') && !op.contains('<') || op.contains('|');
                if op == ")" {
                    command_next = false;
                }
                emit(&mut out, &chars[start..i], st.operator);
            }
            _ => {
                let start = i;
                let mut parts = Vec::new();
                let word = word(&chars, &mut i, &st, base, &mut parts);
                if command_next && !is_assignment(&chars[start..i]) {
                    // The word's plain parts are the command name.
                    for part in &mut parts {
                        if part.style == base {
                            part.style = st.command;
                        }
                    }
                    command_next = PREFIXES.contains(&word.as_str());
                }
                for part in parts {
                    push_text(&mut out, &part.content, part.style);
                }
            }
        }
    }
    out
}

/// Reads one word (with its quotes and `$` parts) into `out`; returns its text.
fn word(
    chars: &[char],
    i: &mut usize,
    st: &Styles,
    base: Style,
    out: &mut Vec<Span<'static>>,
) -> String {
    let start = *i;
    while *i < chars.len() {
        let c = chars[*i];
        if c.is_whitespace() || matches!(c, '|' | '&' | ';' | '(' | ')' | '<' | '>' | '`') {
            break;
        }
        match c {
            '\'' => {
                let from = *i;
                *i += 1;
                while *i < chars.len() && chars[*i] != '\'' {
                    *i += 1;
                }
                *i = (*i + 1).min(chars.len());
                push(out, &chars[from..*i], st.string);
            }
            '"' => {
                let from = *i;
                *i += 1;
                while *i < chars.len() && chars[*i] != '"' {
                    if chars[*i] == '\\' {
                        *i += 1;
                    }
                    *i += 1;
                }
                *i = (*i + 1).min(chars.len());
                push(out, &chars[from..*i], st.string);
            }
            '$' => {
                let from = *i;
                *i += 1;
                if chars.get(*i) == Some(&'{') {
                    while *i < chars.len() && chars[*i] != '}' {
                        *i += 1;
                    }
                    *i = (*i + 1).min(chars.len());
                } else if chars.get(*i) == Some(&'(') {
                    // `$(` opens a command of its own: the `(` comes next as an operator.
                    push(out, &chars[from..*i], st.operator);
                    break;
                } else {
                    while *i < chars.len()
                        && (chars[*i].is_alphanumeric() || matches!(chars[*i], '_' | '?' | '@'))
                    {
                        *i += 1;
                    }
                }
                push(out, &chars[from..*i], st.variable);
            }
            '\\' => {
                let from = *i;
                *i = (*i + 2).min(chars.len());
                push(out, &chars[from..*i], base);
            }
            _ => {
                push(out, &chars[*i..=*i], base);
                *i += 1;
            }
        }
    }
    chars[start..*i].iter().collect()
}

fn push(out: &mut Vec<Span<'static>>, text: &[char], style: Style) {
    push_text(out, &text.iter().collect::<String>(), style);
}

/// `NAME=value` before a command.
fn is_assignment(word: &[char]) -> bool {
    let Some(eq) = word.iter().position(|c| *c == '=') else {
        return false;
    };
    eq > 0
        && word[..eq]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(source: &str) -> Vec<(String, &'static str)> {
        let p = Palette::default();
        let base = Style::new();
        highlight(source, &p, base)
            .into_iter()
            .filter(|s| !s.content.trim().is_empty())
            .map(|s| {
                let class = if s.style.add_modifier.contains(Modifier::BOLD) {
                    "command"
                } else if s.style.add_modifier.contains(Modifier::ITALIC) {
                    "comment"
                } else if s.style.fg == Some(p.success) {
                    "string"
                } else if s.style.fg == Some(p.accent) {
                    "variable"
                } else if s.style.fg == Some(p.muted) {
                    "operator"
                } else {
                    "plain"
                };
                (s.content.trim().to_string(), class)
            })
            .collect()
    }

    #[test]
    fn commands_strings_variables_and_operators() {
        assert_eq!(
            classes(r#"grep -rn "a b" $HOME | head -5 && echo 'done' # ok"#),
            [
                ("grep".into(), "command"),
                ("-rn".into(), "plain"),
                ("\"a b\"".into(), "string"),
                ("$HOME".into(), "variable"),
                ("|".into(), "operator"),
                ("head".into(), "command"),
                ("-5".into(), "plain"),
                ("&&".into(), "operator"),
                ("echo".into(), "command"),
                ("'done'".into(), "string"),
                ("# ok".into(), "comment"),
            ]
        );
    }

    #[test]
    fn prefixes_assignments_and_redirects() {
        let got = classes("FOO=1 sudo make install 2>&1 > log.txt");
        assert_eq!(got[0], ("FOO=1".into(), "plain"));
        assert_eq!(got[1], ("sudo".into(), "command"));
        assert_eq!(got[2], ("make".into(), "command"));
        assert_eq!(got[3], ("install 2".into(), "plain"));
        // After a redirect comes a file, not a command.
        assert_eq!(got.last().unwrap(), &("log.txt".to_string(), "plain"));
    }

    #[test]
    fn the_text_is_never_changed() {
        for source in [
            "a 'unterminated",
            "x \"es\\\"c\" $(ls)",
            "",
            "\n\n",
            "${A:-b} \\| c",
        ] {
            let joined: String = highlight(source, &Palette::default(), Style::new())
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(joined, source);
        }
    }
}
