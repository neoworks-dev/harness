//! Shell commands: highlighted by syntect's Bash grammar, and long lines broken after their
//! `&&`, `||`, `|` and `;` where bash takes a newline the same way.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::Palette;
use crate::syntax::{block_colors, highlight_spans, syntax_for_language};

/// The languages a code block is a shell command in (its long lines may be reflowed).
pub(crate) fn is_shell(language: Option<&str>) -> bool {
    matches!(
        language.map(str::to_ascii_lowercase).as_deref(),
        Some("bash" | "sh" | "shell" | "zsh" | "console" | "shell-session")
    )
}

/// Styled spans for `source`, newlines kept and the text unchanged. `base` is patched under
/// every span (the code block's colors).
pub(crate) fn highlight(source: &str, p: &Palette, base: Style) -> Vec<Span<'static>> {
    match syntax_for_language("bash") {
        Some(bash) => highlight_spans(source, bash, &block_colors(p), base),
        None => vec![Span::styled(source.to_string(), base)],
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_break_but_pipelines_stay_whole() {
        assert_eq!(
            reflow_lists("cd ~/x && grep -rn a src | head -5; echo done", 30),
            "cd ~/x &&\n  grep -rn a src | head -5;\n  echo done"
        );
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
