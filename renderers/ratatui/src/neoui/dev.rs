//! Dev components: diffs, file trees, code references, commits, command output, test runs,
//! stack traces, GitHub issues, pull requests and checks, timelines. Each is drawn the way the
//! terminal already draws the thing it stands for: a `Diff` like an edit's diff, a `FileTree`
//! like the files panel.

use std::collections::BTreeMap;

use neo_protocol as pb;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{N, number};
use crate::render::{Cx, Lines, prefixed};
use crate::text::{clean, hard_wrap, line_width, sline, truncate, truncate_spans, width, wrap};

/// Output lines a `CommandOutput` shows from its end, and a failed `Test` of its message,
/// unless everything is shown in full.
const OUTPUT_TAIL: usize = 12;
const MESSAGE_LINES: usize = 6;

/// A unified diff's text as diff lines: headers (`diff --git`, `index`, `---`, `+++`) left out,
/// hunks numbering the lines after them. The `+++ b/` path is returned for a diff without one.
pub(crate) fn parse_diff(text: &str) -> (Vec<pb::DiffLine>, Option<String>) {
    let mut lines = Vec::new();
    let mut path = None;
    let (mut old, mut new) = (0i64, 0i64);
    let mut in_hunk = false;
    for raw in text.lines() {
        if !in_hunk {
            if let Some(p) = raw.strip_prefix("+++ ") {
                let p = p.trim();
                path = Some(p.strip_prefix("b/").unwrap_or(p).to_string());
                continue;
            }
            if raw.starts_with("diff --git")
                || raw.starts_with("index ")
                || raw.starts_with("--- ")
                || raw.starts_with("new file mode")
                || raw.starts_with("deleted file mode")
            {
                continue;
            }
        }
        if raw.starts_with("\\ ") {
            continue;
        }
        if let Some(rest) = raw.strip_prefix("@@") {
            // `@@ -12,5 +12,6 @@`: where the hunk starts on each side.
            let start = |sign: char| {
                rest.split_whitespace()
                    .find_map(|w| w.strip_prefix(sign))
                    .and_then(|r| r.split(',').next())
                    .and_then(|n| n.parse::<i64>().ok())
                    .unwrap_or(1)
            };
            old = start('-');
            new = start('+');
            in_hunk = true;
            lines.push(pb::DiffLine {
                kind: pb::DiffLineKind::Hunk,
                text: raw.to_string(),
                old: None,
                new: None,
            });
            continue;
        }
        let (kind, body) = match raw.chars().next() {
            Some('+') => (pb::DiffLineKind::Add, &raw[1..]),
            Some('-') => (pb::DiffLineKind::Remove, &raw[1..]),
            Some(' ') => (pb::DiffLineKind::Context, &raw[1..]),
            _ => (pb::DiffLineKind::Context, raw),
        };
        let (o, n) = match kind {
            pb::DiffLineKind::Add => {
                new += 1;
                (None, Some(new - 1))
            }
            pb::DiffLineKind::Remove => {
                old += 1;
                (Some(old - 1), None)
            }
            _ => {
                old += 1;
                new += 1;
                (Some(old - 1), Some(new - 1))
            }
        };
        lines.push(pb::DiffLine {
            kind,
            text: body.to_string(),
            old: in_hunk.then_some(o).flatten(),
            new: in_hunk.then_some(n).flatten(),
        });
    }
    (lines, path)
}

/// A file's mark and its color: `M`, `A`, `D`, `R`, `?`, or a muted dot for an unchanged one.
fn file_mark(status: Option<&str>, cx: &Cx<'_>) -> (&'static str, Color) {
    match status {
        Some("added") => ("A", cx.p.success),
        Some("deleted") => ("D", cx.p.danger),
        Some("renamed") => ("R", cx.p.accent),
        Some("untracked") => ("?", cx.p.muted),
        Some("unchanged") => ("·", cx.p.muted),
        _ => ("M", cx.p.accent),
    }
}

/// `+12 −3`, a zero side left out, in the success and danger colors.
fn counts(added: Option<f64>, removed: Option<f64>, cx: &Cx<'_>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if let Some(a) = added.filter(|a| *a > 0.0) {
        spans.push(Span::styled(
            format!("+{}", number(a)),
            Style::new().fg(cx.p.success),
        ));
    }
    if let Some(r) = removed.filter(|r| *r > 0.0) {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("−{}", number(r)),
            Style::new().fg(cx.p.danger),
        ));
    }
    spans
}

/// `left` with `right` at the edge of `w` cells; `left` is cut to leave `right` room, and
/// `right` is dropped when even that leaves too little.
fn edge(left: Vec<Span<'static>>, right: Vec<Span<'static>>, w: usize) -> Line<'static> {
    let rw = line_width(&right);
    if rw == 0 || rw + 8 > w {
        return Line::from(truncate_spans(left, w));
    }
    let mut spans = truncate_spans(left, w - rw - 1);
    let used = line_width(&spans);
    spans.push(Span::raw(" ".repeat(w - used - rw)));
    spans.extend(right);
    Line::from(spans)
}

/// A folder of a `FileTree` while it is built: its folders, and its files by index.
#[derive(Default)]
struct Folder {
    dirs: BTreeMap<String, Folder>,
    files: Vec<usize>,
}

impl<'a> Cx<'a> {
    /// A `File`, `Test`, `Frame`, `Check` or `TimelineItem` outside its list: the list with it
    /// alone in it.
    pub(super) fn ui_alone(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let (parent, prop) = match n.name() {
            "File" => ("FileTree", "files"),
            "Test" => ("TestRun", "tests"),
            "Frame" => ("StackTrace", "frames"),
            "Check" => ("Checks", "checks"),
            _ => ("Timeline", "items"),
        };
        let list = serde_json::json!({ "component": parent, "props": { prop: [n.0.clone()] } });
        match N::new(&list) {
            Some(list) => self.ui_node(list, path, w),
            None => Vec::new(),
        }
    }

    pub(super) fn ui_diff(&mut self, n: N<'_>, w: usize) -> Lines {
        let (lines, found) = parse_diff(n.str("diff").unwrap_or(""));
        let path = n.line("path").or(found);
        self.diff(&pb::DiffComponent { path, lines }, w)
    }

    /// Folders first, then files, each by name; a folder holding only one folder is joined
    /// with it (`apps/term/src/`), like the files panel.
    pub(super) fn ui_file_tree(&mut self, n: N<'_>, w: usize) -> Lines {
        let files = n.nodes("files");
        let mut root = Folder::default();
        for (i, f) in files.iter().enumerate() {
            let path = f.line("path").unwrap_or_default();
            let mut parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
            parts.pop();
            let mut node = &mut root;
            for part in parts.into_iter().filter(|p| !p.is_empty()) {
                node = node.dirs.entry(format!("{part}/")).or_default();
            }
            node.files.push(i);
        }
        let mut out = Vec::new();
        self.tree_rows(&root, &files, 0, w, &mut out);
        out
    }

    fn tree_rows(&self, dir: &Folder, files: &[N<'_>], depth: usize, w: usize, out: &mut Lines) {
        let indent = "  ".repeat(depth);
        for (name, mut node) in &dir.dirs {
            let mut name = name.clone();
            while node.files.is_empty() && node.dirs.len() == 1 {
                let Some((next, only)) = node.dirs.iter().next() else {
                    break;
                };
                name.push_str(next);
                node = only;
            }
            out.push(Line::from(truncate_spans(
                vec![
                    Span::styled(format!("{indent}▾ "), self.p.muted_style()),
                    Span::raw(name),
                ],
                w,
            )));
            self.tree_rows(node, files, depth + 1, w, out);
        }
        let mut shown = dir.files.clone();
        let base = |i: &usize| {
            files
                .get(*i)
                .and_then(|f| f.line("path"))
                .map(|p| {
                    let t = p.trim_end_matches('/');
                    t.rsplit('/').next().unwrap_or(t).to_lowercase()
                })
                .unwrap_or_default()
        };
        shown.sort_by_key(base);
        for i in shown {
            let Some(f) = files.get(i) else { continue };
            let path = f.line("path").unwrap_or_default();
            let trimmed = path.trim_end_matches('/');
            let mut name = trimmed.rsplit('/').next().unwrap_or(trimmed).to_string();
            if path.ends_with('/') {
                name.push('/');
            }
            let status = f.str("status");
            let (mark, color) = file_mark(status, self);
            let name_style = match status {
                Some("untracked" | "unchanged") => self.p.muted_style(),
                Some("deleted") => Style::new().fg(self.p.danger),
                Some("added") => Style::new().fg(self.p.success),
                _ => Style::new(),
            };
            let mut left = vec![
                Span::raw(indent.clone()),
                Span::styled(format!("{mark} "), Style::new().fg(color)),
                Span::styled(name, name_style),
            ];
            if let Some(note) = f.line("note") {
                left.push(Span::styled(format!("  {note}"), self.p.muted_style()));
            }
            let right = counts(f.num("added"), f.num("removed"), self);
            out.push(edge(left, right, w));
        }
    }

    /// `src/app.rs:120  let x = 1;`: the place in the accent, what is there muted.
    pub(super) fn ui_code_ref(&mut self, n: N<'_>, w: usize) -> Lines {
        let mut place = n.line("path").unwrap_or_default();
        if let Some(line) = n.num("line") {
            place.push_str(&format!(":{}", number(line)));
        }
        let mut spans = vec![Span::styled(
            place,
            Style::new()
                .fg(self.p.accent)
                .add_modifier(Modifier::UNDERLINED),
        )];
        if let Some(text) = n.line("text") {
            spans.push(Span::styled(
                format!("  {}", text.trim()),
                self.p.muted_style(),
            ));
        }
        vec![Line::from(truncate_spans(spans, w))]
    }

    /// `7e422be Shift-Tab goes back            +12 −3`, then `moritz · 2h ago · 3 files` muted.
    pub(super) fn ui_commit(&mut self, n: N<'_>, w: usize) -> Lines {
        let sha = n.line("sha").unwrap_or_default();
        let sha: String = sha.chars().take(7).collect();
        let left = vec![
            Span::styled(format!("{sha} "), Style::new().fg(self.p.accent)),
            Span::styled(
                n.line("title").unwrap_or_default(),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ];
        let mut out = vec![edge(
            left,
            counts(n.num("added"), n.num("removed"), self),
            w,
        )];
        let mut about: Vec<String> = Vec::new();
        about.extend(n.line("author"));
        about.extend(n.line("date"));
        if let Some(files) = n.num("files") {
            let s = if files == 1.0 { "" } else { "s" };
            about.push(format!("{} file{s}", number(files)));
        }
        if !about.is_empty() {
            let indent = " ".repeat(width(&sha) + 1);
            out.push(sline(
                truncate(&format!("{indent}{}", about.join(" · ")), w),
                self.p.muted_style(),
            ));
        }
        out
    }

    /// `$ cargo test              ✓ exit 0 · 13s`, then the end of the output, muted.
    pub(super) fn ui_command_output(&mut self, n: N<'_>, w: usize) -> Lines {
        let exit = n.num("exit");
        let mut right = Vec::new();
        if let Some(code) = exit {
            let (glyph, color) = if code == 0.0 {
                ("✓", self.p.success)
            } else {
                ("✗", self.p.danger)
            };
            right.push(Span::styled(
                format!("{glyph} exit {}", number(code)),
                Style::new().fg(color),
            ));
        }
        if let Some(d) = n.line("duration") {
            if !right.is_empty() {
                right.push(Span::styled(" · ", self.p.muted_style()));
            }
            right.push(Span::styled(d, self.p.muted_style()));
        }
        let mut left = vec![Span::styled("$ ", self.p.muted_style())];
        if let Some(cwd) = n.line("cwd") {
            left.push(Span::styled(format!("{cwd} "), self.p.muted_style()));
        }
        left.push(Span::styled(
            n.line("command").unwrap_or_default(),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        let mut out = vec![edge(left, right, w)];
        let output = clean(n.str("output").unwrap_or("")).into_owned();
        let all: Vec<&str> = output.trim_end().lines().collect();
        let keep = if self.o.expanded {
            all.len()
        } else {
            OUTPUT_TAIL
        };
        if all.len() > keep {
            out.push(sline(
                format!("… {} lines above", all.len() - keep),
                self.p.muted_style(),
            ));
        }
        let failed = exit.is_some_and(|c| c != 0.0);
        for line in &all[all.len().saturating_sub(keep)..] {
            let style = if failed && is_error_line(line) {
                Style::new().fg(self.p.danger)
            } else {
                self.p.muted_style()
            };
            out.extend(hard_wrap(line, w).into_iter().map(|l| sline(l, style)));
        }
        out
    }

    /// `cargo test  ✓ 434 passed  ✗ 1 failed  ○ 2 skipped  · 13s`, then each test given, a
    /// failed one with its message under it.
    pub(super) fn ui_test_run(&mut self, n: N<'_>, w: usize) -> Lines {
        let mut head: Vec<Span<'static>> = Vec::new();
        if let Some(name) = n.line("name") {
            head.push(Span::styled(
                format!("{name}  "),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        }
        let parts = [
            ("passed", "✓", self.p.success),
            ("failed", "✗", self.p.danger),
            ("skipped", "○", self.p.muted),
        ];
        let mut first = true;
        for (prop, glyph, color) in parts {
            let Some(count) = n.num(prop) else { continue };
            if count == 0.0 && prop != "passed" {
                continue;
            }
            if !first {
                head.push(Span::raw("  "));
            }
            first = false;
            head.push(Span::styled(
                format!("{glyph} {} {prop}", number(count)),
                Style::new().fg(color),
            ));
        }
        if let Some(d) = n.line("duration") {
            head.push(Span::styled(format!(" · {d}"), self.p.muted_style()));
        }
        let mut out = Vec::new();
        if !head.is_empty() {
            out.extend(wrap(&head, w));
        }
        for t in n.nodes("tests") {
            let (glyph, color) = match t.str("status") {
                Some("passed") => ("✓", self.p.success),
                Some("skipped") => ("○", self.p.muted),
                _ => ("✗", self.p.danger),
            };
            let left = vec![
                Span::styled(format!("{glyph} "), Style::new().fg(color)),
                Span::raw(t.line("name").unwrap_or_default()),
            ];
            let right = t
                .line("duration")
                .map(|d| vec![Span::styled(d, self.p.muted_style())])
                .unwrap_or_default();
            out.push(edge(left, right, w));
            let message = clean(t.str("message").unwrap_or("")).into_owned();
            let lines: Vec<&str> = message.trim_end().lines().collect();
            let keep = if self.o.expanded {
                lines.len()
            } else {
                MESSAGE_LINES
            };
            let style = Style::new().fg(self.p.danger);
            let mut body = Vec::new();
            for line in lines.iter().take(keep) {
                let room = w.saturating_sub(4);
                body.extend(hard_wrap(line, room).into_iter().map(|l| sline(l, style)));
            }
            if lines.len() > keep {
                body.push(sline(
                    format!("… {} more lines", lines.len() - keep),
                    self.p.muted_style(),
                ));
            }
            out.extend(prefixed(
                body,
                vec![Span::raw("    ")],
                vec![Span::raw("    ")],
            ));
        }
        out
    }

    /// `✗ panicked: index out of bounds`, then `  at name  src/app.rs:120` per frame.
    pub(super) fn ui_stack_trace(&mut self, n: N<'_>, w: usize) -> Lines {
        let message = n.line("message").unwrap_or_default();
        let head = match n.line("kind") {
            Some(kind) => format!("{kind}: {message}"),
            None => message,
        };
        let danger = Style::new().fg(self.p.danger).add_modifier(Modifier::BOLD);
        let mut out = prefixed(
            wrap(&[Span::styled(head, danger)], w.saturating_sub(2).max(1)),
            vec![Span::styled("✗ ", Style::new().fg(self.p.danger))],
            vec![Span::raw("  ")],
        );
        for f in n.nodes("frames") {
            let mut place = f.line("path").unwrap_or_default();
            if let Some(line) = f.num("line") {
                place.push_str(&format!(":{}", number(line)));
            }
            let mut spans = vec![Span::styled("  at ", self.p.muted_style())];
            if let Some(name) = f.line("name") {
                spans.push(Span::raw(format!("{name}  ")));
            }
            spans.push(Span::styled(place, Style::new().fg(self.p.accent)));
            out.push(Line::from(truncate_spans(spans, w)));
        }
        out
    }

    /// A state pill, `#53` and the title, then the repository, author and labels.
    pub(super) fn ui_issue(&mut self, n: N<'_>, w: usize) -> Lines {
        let (label, color) = match n.str("state") {
            Some("closed") => ("Closed", self.p.accent),
            _ => ("Open", self.p.success),
        };
        let mut out = self.github_title(n, label, color, w);
        let mut about: Vec<Span<'static>> = Vec::new();
        let mut words: Vec<String> = Vec::new();
        words.extend(n.line("repo"));
        words.extend(n.line("author"));
        if let Some(c) = n.num("comments").filter(|c| *c > 0.0) {
            let s = if c == 1.0 { "" } else { "s" };
            words.push(format!("{} comment{s}", number(c)));
        }
        if !words.is_empty() {
            about.push(Span::styled(words.join(" · "), self.p.muted_style()));
        }
        let labels: Vec<String> = n
            .prop("labels")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str())
                    .map(crate::text::one_line)
                    .collect()
            })
            .unwrap_or_default();
        for l in labels {
            if !about.is_empty() {
                about.push(Span::raw(" "));
            }
            let style = Style::new()
                .fg(self.p.muted)
                .bg(self.p.tint(Some(self.p.muted)));
            about.extend(self.pill(l, style));
        }
        if !about.is_empty() {
            out.push(Line::from(truncate_spans(about, w)));
        }
        out
    }

    /// A state pill, `#41` and the title, then `branch → base · author`, the checks, the
    /// review and the line counts.
    pub(super) fn ui_pull_request(&mut self, n: N<'_>, w: usize) -> Lines {
        let (label, color) = match n.str("state") {
            Some("draft") => ("Draft", self.p.muted),
            Some("merged") => ("Merged", self.p.accent),
            Some("closed") => ("Closed", self.p.danger),
            _ => ("Open", self.p.success),
        };
        let mut out = self.github_title(n, label, color, w);
        let muted = self.p.muted_style();
        let mut left: Vec<Span<'static>> = Vec::new();
        let mut words: Vec<String> = Vec::new();
        match (n.line("branch"), n.line("base")) {
            (Some(b), Some(base)) => words.push(format!("{b} → {base}")),
            (Some(b), None) => words.push(b),
            _ => {}
        }
        words.extend(n.line("repo"));
        words.extend(n.line("author"));
        if !words.is_empty() {
            left.push(Span::styled(words.join(" · "), muted));
        }
        let sep = |spans: &mut Vec<Span<'static>>| {
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", muted));
            }
        };
        if let Some(checks) = n.str("checks") {
            let (text, color) = match checks {
                "passing" => ("✓ checks pass", self.p.success),
                "failing" => ("✗ checks fail", self.p.danger),
                _ => ("◐ checks running", self.p.muted),
            };
            sep(&mut left);
            left.push(Span::styled(text, Style::new().fg(color)));
        }
        if let Some(review) = n.str("review") {
            let (text, color) = match review {
                "approved" => ("✓ approved", self.p.success),
                "changes" => ("✗ changes requested", self.p.danger),
                _ => ("○ review pending", self.p.muted),
            };
            sep(&mut left);
            left.push(Span::styled(text, Style::new().fg(color)));
        }
        let right = counts(n.num("added"), n.num("removed"), self);
        if !left.is_empty() || !right.is_empty() {
            out.push(edge(left, right, w));
        }
        out
    }

    /// `[Open] #41 Title`: the state as a pill in its color, the number muted, the title bold,
    /// wrapped.
    fn github_title(&self, n: N<'_>, label: &str, color: Color, w: usize) -> Lines {
        let style = Style::new().fg(color).bg(self.p.tint(Some(color)));
        let mut spans = self.pill(label.to_string(), style);
        if let Some(num) = n.num("number") {
            spans.push(Span::styled(
                format!(" #{}", number(num)),
                self.p.muted_style(),
            ));
        }
        spans.push(Span::styled(
            format!(" {}", n.line("title").unwrap_or_default()),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        wrap(&spans, w)
    }

    /// `✓ build        2m 10s` per check, the failed ones in the danger color.
    pub(super) fn ui_checks(&mut self, n: N<'_>, w: usize) -> Lines {
        n.nodes("checks")
            .into_iter()
            .map(|c| {
                let (glyph, color) = match c.str("status") {
                    Some("passed") => ("✓", self.p.success),
                    Some("failed") => ("✗", self.p.danger),
                    Some("skipped") => ("○", self.p.muted),
                    _ => ("◐", self.p.accent),
                };
                let name_style = match c.str("status") {
                    Some("failed") => Style::new().fg(self.p.danger),
                    Some("skipped") => self.p.muted_style(),
                    _ => Style::new(),
                };
                let left = vec![
                    Span::styled(format!("{glyph} "), Style::new().fg(color)),
                    Span::styled(c.line("name").unwrap_or_default(), name_style),
                ];
                let right = c
                    .line("duration")
                    .map(|d| vec![Span::styled(d, self.p.muted_style())])
                    .unwrap_or_default();
                edge(left, right, w)
            })
            .collect()
    }

    /// `● 14:02  Deployed to prod`, the detail under it, a line joining the dots.
    pub(super) fn ui_timeline(&mut self, n: N<'_>, w: usize) -> Lines {
        let items = n.nodes("items");
        let time_w = items
            .iter()
            .filter_map(|i| i.line("time"))
            .map(|t| width(&t))
            .max()
            .unwrap_or(0);
        let rail = self.p.muted_style();
        let mut out = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let color = item.tone().map_or(self.p.accent, |t| self.p.tone(t));
            let last = i + 1 == items.len();
            let mut spans = vec![Span::styled("● ", Style::new().fg(color))];
            if time_w > 0 {
                let t = item.line("time").unwrap_or_default();
                spans.push(Span::styled(
                    format!("{t}{}  ", " ".repeat(time_w - width(&t))),
                    self.p.muted_style(),
                ));
            }
            spans.push(Span::styled(
                item.line("title").unwrap_or_default(),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            out.push(Line::from(truncate_spans(spans, w)));
            let gutter = if last { "  " } else { "│ " };
            let pad = " ".repeat(if time_w > 0 { time_w + 2 } else { 0 });
            if let Some(detail) = item.line("detail") {
                let body = wrap(
                    &[Span::styled(detail, self.p.muted_style())],
                    w.saturating_sub(2 + pad.len()).max(1),
                );
                let lead = vec![Span::styled(gutter, rail), Span::raw(pad.clone())];
                out.extend(prefixed(body, lead.clone(), lead));
            }
        }
        out
    }
}

/// A line of output that reads as an error: what a failed command's tail is read for.
fn is_error_line(line: &str) -> bool {
    let l = line.trim_start().to_lowercase();
    l.starts_with("error")
        || l.starts_with("fatal")
        || l.contains("failed")
        || l.contains("panicked")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unified_diff_is_parsed_with_its_numbers_and_path() {
        let (lines, path) = parse_diff(
            "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,3 +10,3 @@ fn x\n ctx\n-old\n+new\n\\ No newline at end of file\n",
        );
        assert_eq!(path.as_deref(), Some("src/a.rs"));
        let got: Vec<_> = lines
            .iter()
            .map(|l| (l.kind, l.text.as_str(), l.old, l.new))
            .collect();
        assert_eq!(
            got,
            [
                (pb::DiffLineKind::Hunk, "@@ -10,3 +10,3 @@ fn x", None, None),
                (pb::DiffLineKind::Context, "ctx", Some(10), Some(10)),
                (pb::DiffLineKind::Remove, "old", Some(11), None),
                (pb::DiffLineKind::Add, "new", None, Some(11)),
            ]
        );
    }

    #[test]
    fn error_lines_are_told_apart() {
        assert!(is_error_line("error[E0308]: mismatched types"));
        assert!(is_error_line("  thread 'main' panicked at src/main.rs:2:5"));
        assert!(!is_error_line("   Compiling neo v0.1.0"));
    }
}
