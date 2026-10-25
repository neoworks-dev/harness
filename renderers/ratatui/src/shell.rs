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

/// `source` with every line wider than `w` broken after its `&&`, `||`, `|` and `;`, the parts
/// after the first indented two more. Only where bash takes a newline the same way: not inside
/// quotes, parentheses or `$(…)`, comments or heredoc bodies. Lines that fit stay as they are.
pub(crate) fn reflow(source: &str, w: usize) -> String {
    reflow_at(source, w, true)
}

/// [`reflow`] that keeps pipelines whole where it can: lines break after `&&`, `||` and `;`,
/// and after `|` too only when a pipeline is wider than `w` on its own.
pub(crate) fn reflow_lists(source: &str, w: usize) -> String {
    let lists = reflow_at(source, w, false);
    if lists.lines().all(|l| crate::text::width(l) <= w) {
        lists
    } else {
        reflow_at(source, w, true)
    }
}

fn reflow_at(source: &str, w: usize, pipes: bool) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    // The current line, and where it may break (char indices into it).
    let mut line = String::new();
    let mut breaks: Vec<usize> = Vec::new();
    let mut quote: Option<char> = None;
    let mut depth: i32 = 0;
    // Heredocs opened on the current line: their terminator and whether tabs are stripped.
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    let mut word_start = true;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            flush(&mut out, &line, &breaks, w);
            out.push('\n');
            line.clear();
            breaks.clear();
            word_start = true;
            i += 1;
            for (term, strip) in heredocs.drain(..) {
                while i < chars.len() {
                    let end = chars[i..]
                        .iter()
                        .position(|&c| c == '\n')
                        .map_or(chars.len(), |at| i + at);
                    let body: String = chars[i..end].iter().collect();
                    out.push_str(&body);
                    i = end;
                    if i < chars.len() {
                        out.push('\n');
                        i += 1;
                    }
                    let check = if strip {
                        body.trim_start_matches('\t')
                    } else {
                        &body
                    };
                    if check == term {
                        break;
                    }
                }
            }
            continue;
        }
        line.push(c);
        i += 1;
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
                continue;
            }
            Some(q) => {
                if c == '\\' && chars.get(i).is_some_and(|&n| n != '\n') {
                    line.push(chars[i]);
                    i += 1;
                } else if c == q {
                    quote = None;
                }
                continue;
            }
            None => {}
        }
        match c {
            '\\' => {
                if chars.get(i).is_some_and(|&n| n != '\n') {
                    line.push(chars[i]);
                    i += 1;
                }
                word_start = false;
            }
            '\'' | '"' | '`' => {
                quote = Some(c);
                word_start = false;
            }
            '#' if word_start => {
                while i < chars.len() && chars[i] != '\n' {
                    line.push(chars[i]);
                    i += 1;
                }
            }
            '(' => {
                depth += 1;
                word_start = true;
            }
            ')' => {
                depth -= 1;
                word_start = true;
            }
            // A here-string, not a heredoc.
            '<' if chars.get(i) == Some(&'<') && chars.get(i + 1) == Some(&'<') => {
                line.push_str("<<");
                i += 2;
                word_start = false;
            }
            '<' if chars.get(i) == Some(&'<') => {
                line.push('<');
                i += 1;
                let strip = chars.get(i) == Some(&'-');
                if strip {
                    line.push('-');
                    i += 1;
                }
                while chars.get(i).is_some_and(|&c| c == ' ' || c == '\t') {
                    line.push(chars[i]);
                    i += 1;
                }
                let mut term = String::new();
                while let Some(&c) = chars.get(i) {
                    if c.is_whitespace() || matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')') {
                        break;
                    }
                    line.push(c);
                    if c != '\'' && c != '"' && c != '\\' {
                        term.push(c);
                    }
                    i += 1;
                }
                // `1 << 2` shifts; a terminator is a word.
                if term.starts_with(|c: char| c.is_alphabetic() || c == '_') {
                    heredocs.push((term, strip));
                }
                word_start = false;
            }
            '|' | '&' | ';' => {
                let mut op = String::from(c);
                while let Some(&n) = chars.get(i).filter(|&&n| matches!(n, '|' | '&' | ';')) {
                    op.push(n);
                    line.push(n);
                    i += 1;
                }
                // `>&`/`&>` redirect and a lone `&` backgrounds: no break after those.
                let redirect = op.starts_with('&') && chars.get(i) == Some(&'>');
                if depth == 0
                    && !redirect
                    && (matches!(op.as_str(), "&&" | "||" | ";" | ";;")
                        || (pipes && matches!(op.as_str(), "|" | "|&")))
                {
                    breaks.push(line.chars().count());
                }
                word_start = true;
            }
            c if c.is_whitespace() => word_start = true,
            _ => word_start = false,
        }
    }
    flush(&mut out, &line, &breaks, w);
    out
}

/// One line of [`reflow`]: as it is when it fits, else broken at `breaks`.
fn flush(out: &mut String, line: &str, breaks: &[usize], w: usize) {
    if breaks.is_empty() || crate::text::width(line) <= w {
        out.push_str(line);
        return;
    }
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let chars: Vec<char> = line.chars().collect();
    let mut from = 0;
    let mut first = true;
    for &at in breaks.iter().chain(std::iter::once(&chars.len())) {
        let part: String = chars[from..at].iter().collect();
        from = at;
        let part = if first { part.trim_end() } else { part.trim() };
        if part.is_empty() {
            continue;
        }
        if !first {
            out.push('\n');
            out.push_str(&indent);
            out.push_str("  ");
        }
        out.push_str(part);
        first = false;
    }
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
    fn lists_break_but_pipelines_stay_whole() {
        assert_eq!(
            reflow_lists("cd ~/x && grep -rn a src | head -5; echo done", 30),
            "cd ~/x &&\n  grep -rn a src | head -5;\n  echo done"
        );
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
    fn long_lines_break_after_their_operators() {
        assert_eq!(
            reflow("cd ~/x && grep -rn 'a && b' src | head -5; echo done", 30),
            "cd ~/x &&\n  grep -rn 'a && b' src |\n  head -5;\n  echo done"
        );
        // What fits, and what is inside $(…), quotes or a comment, stays.
        let short = "a && b | c";
        assert_eq!(reflow(short, 80), short);
        let nested = "echo \"$(git log | head -1 && date)\" # x && y";
        assert_eq!(reflow(nested, 10), nested);
        // Redirects and background jobs don't break.
        assert_eq!(
            reflow("make 2>&1 &> log & sleep 1", 5),
            "make 2>&1 &> log & sleep 1"
        );
    }

    #[test]
    fn heredoc_bodies_are_left_alone() {
        let source =
            "cat > f.py <<'PY' && python3 f.py\nimport os; print(1) | 2; x && y\nPY\nls | wc -l";
        assert_eq!(
            reflow(source, 12),
            "cat > f.py <<'PY' &&\n  python3 f.py\nimport os; print(1) | 2; x && y\nPY\nls | wc -l"
        );
        // A here-string and a shift open no body.
        assert_eq!(
            reflow("x <<< a && y\nb && c", 5),
            "x <<< a &&\n  y\nb &&\n  c"
        );
        assert_eq!(
            reflow("echo $((1 << 2)) && y\nb", 5),
            "echo $((1 << 2)) &&\n  y\nb"
        );
    }

    #[test]
    fn reflowed_commands_still_parse() {
        let Ok(bash) = std::process::Command::new("bash").arg("--version").output() else {
            return;
        };
        assert!(bash.status.success());
        for source in [
            "cd /tmp && ls -la | grep x || echo none; for i in 1 2; do echo $i; done",
            "if [ -f a ]; then cat a | sort; else echo b; fi && echo c",
            "cat <<EOF | wc -l && echo ok\nline one && two\nEOF\necho after; echo end",
            "case $x in a) echo a;; b) echo b;; esac && echo z",
            "git commit -m \"$(printf 'a\\nb')\" && git push 2>&1 | tail -3",
        ] {
            let flowed = reflow(source, 8);
            let out = std::process::Command::new("bash")
                .args(["-n", "-c", &flowed])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{flowed}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
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
