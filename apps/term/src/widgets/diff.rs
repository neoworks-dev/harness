//! The diff overlay: `git diff` of one file, loaded in the background, scrolled by keys.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use suite_renderer_ratatui::Palette;

use super::files::Mark;
use super::git::git;
use super::text::{clean_line, truncate, truncate_start};

/// More lines than this are dropped (a generated file diff can be megabytes).
const MAX_LINES: usize = 20_000;

/// What to diff: the selected row of the files panel.
#[derive(Debug, Clone)]
pub(crate) struct DiffRequest {
    /// Relative to the session directory.
    pub rel: String,
    /// The old path of a rename.
    pub orig: Option<String>,
    pub mark: Mark,
    /// Whether git knows the directory as a repository.
    pub in_repo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Header,
    Hunk,
    Add,
    Del,
    Context,
    /// A message of ours (nothing to show, git failed).
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffLine {
    pub text: String,
    pub kind: Kind,
}

pub(crate) struct DiffView {
    title: String,
    rx: Option<Receiver<Result<String, String>>>,
    lines: Vec<DiffLine>,
    adds: usize,
    dels: usize,
    loaded: bool,
    scroll: usize,
    /// Rows of diff text the last draw had (for paging and clamping).
    view_h: Cell<usize>,
}

impl DiffView {
    /// Starts `git diff` for `req` on a thread.
    pub(crate) fn open(cwd: PathBuf, req: DiffRequest) -> DiffView {
        let (tx, rx) = mpsc::channel();
        let title = req.rel.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load(&cwd, &req));
        });
        DiffView {
            rx: Some(rx),
            ..DiffView::blank(title)
        }
    }

    fn blank(title: String) -> DiffView {
        DiffView {
            title: clean_line(&title),
            rx: None,
            lines: Vec::new(),
            adds: 0,
            dels: 0,
            loaded: false,
            scroll: 0,
            view_h: Cell::new(20),
        }
    }

    /// An already loaded diff (tests).
    #[cfg(test)]
    pub(crate) fn from_text(title: &str, text: &str) -> DiffView {
        let mut v = DiffView::blank(title.to_string());
        v.finish(Ok(text.to_string()));
        v
    }

    fn finish(&mut self, result: Result<String, String>) {
        let (lines, adds, dels) = match result {
            Ok(text) if text.trim().is_empty() => (
                vec![DiffLine {
                    text: "no changes".into(),
                    kind: Kind::Note,
                }],
                0,
                0,
            ),
            Ok(text) => parse_diff(&text),
            Err(msg) => (
                vec![DiffLine {
                    text: msg,
                    kind: Kind::Note,
                }],
                0,
                0,
            ),
        };
        self.lines = lines;
        self.adds = adds;
        self.dels = dels;
        self.loaded = true;
        self.rx = None;
    }

    /// Picks up the loaded diff. True when it arrived.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.finish(result);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.finish(Err("git diff failed".into()));
                true
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    #[cfg(test)]
    pub(crate) fn lines(&self) -> &[DiffLine] {
        &self.lines
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.adds, self.dels)
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.view_h.get().max(1))
    }

    /// Scrolls; true when the key closes the overlay (Esc, q).
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        let page = self.view_h.get().saturating_sub(1).max(1);
        let half = (self.view_h.get() / 2).max(1);
        let max = self.max_scroll();
        let up = |s: usize, n: usize| s.saturating_sub(n);
        let down = |s: usize, n: usize| (s + n).min(max);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.scroll = self.scroll.min(max);
        match (key.code, ctrl) {
            (KeyCode::Esc, _) | (KeyCode::Char('q'), false) => return true,
            (KeyCode::Up | KeyCode::Char('k'), _) => self.scroll = up(self.scroll, 1),
            (KeyCode::Down | KeyCode::Char('j'), _) => self.scroll = down(self.scroll, 1),
            (KeyCode::PageUp | KeyCode::Char('b'), _) => self.scroll = up(self.scroll, page),
            (KeyCode::PageDown | KeyCode::Char(' ' | 'f'), _) => {
                self.scroll = down(self.scroll, page)
            }
            (KeyCode::Char('u'), true) => self.scroll = up(self.scroll, half),
            (KeyCode::Char('d'), true) => self.scroll = down(self.scroll, half),
            (KeyCode::Home | KeyCode::Char('g'), _) => self.scroll = 0,
            (KeyCode::End | KeyCode::Char('G'), _) => self.scroll = max,
            _ => {}
        }
        false
    }

    /// Fills `area`: a title line, then the diff, long lines cut.
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        frame.render_widget(Clear, area);
        let w = area.width as usize;
        let muted = Style::new().fg(palette.muted);
        let mut title = vec![Span::styled(
            truncate_start(&self.title, w.saturating_sub(14).max(1)),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if !self.loaded {
            title.push(Span::styled("  …", muted));
        } else if self.adds + self.dels > 0 {
            title.push(Span::raw("  "));
            title.push(Span::styled(
                format!("+{}", self.adds),
                Style::new().fg(palette.success),
            ));
            title.push(Span::raw(" "));
            title.push(Span::styled(
                format!("−{}", self.dels),
                Style::new().fg(palette.danger),
            ));
        }
        let title_area = Rect { height: 1, ..area };
        frame.render_widget(Paragraph::new(Line::from(title)), title_area);
        if area.height < 2 {
            return;
        }
        let body = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };
        let rows = body.height as usize;
        self.view_h.set(rows);
        let start = self.scroll.min(self.lines.len().saturating_sub(rows));
        let shown: Vec<Line> = self
            .lines
            .iter()
            .skip(start)
            .take(rows)
            .map(|l| {
                let style = match l.kind {
                    Kind::Add => Style::new().fg(palette.success),
                    Kind::Del => Style::new().fg(palette.danger),
                    Kind::Hunk | Kind::Header | Kind::Note => muted,
                    Kind::Context => Style::new(),
                };
                Line::from(Span::styled(truncate(&l.text, w), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(shown), body);
    }
}

/// Lines with their meaning, and the number of added and removed lines.
pub(crate) fn parse_diff(text: &str) -> (Vec<DiffLine>, usize, usize) {
    let mut lines = Vec::new();
    let (mut adds, mut dels) = (0, 0);
    let mut in_hunk = false;
    for raw in text.lines().take(MAX_LINES) {
        let kind = if raw.starts_with("diff ") {
            in_hunk = false;
            Kind::Header
        } else if raw.starts_with("@@") {
            in_hunk = true;
            Kind::Hunk
        } else if !in_hunk || raw.starts_with('\\') {
            Kind::Header
        } else if raw.starts_with('+') {
            adds += 1;
            Kind::Add
        } else if raw.starts_with('-') {
            dels += 1;
            Kind::Del
        } else {
            Kind::Context
        };
        lines.push(DiffLine {
            text: clean_line(raw),
            kind,
        });
    }
    if text.lines().nth(MAX_LINES).is_some() {
        lines.push(DiffLine {
            text: format!("… cut after {MAX_LINES} lines"),
            kind: Kind::Note,
        });
    }
    (lines, adds, dels)
}

fn run(cmd: &mut std::process::Command, no_index: bool) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    // --no-index exits 1 when the files differ.
    let ok = out.status.success() || (no_index && out.status.code() == Some(1));
    if ok {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err("git diff failed".into())
    }
}

/// The diff text of one row: against HEAD (staged and unstaged together), or the whole file
/// for untracked ones.
fn load(cwd: &Path, req: &DiffRequest) -> Result<String, String> {
    if req.rel.ends_with('/') {
        return Err("untracked directory".into());
    }
    let base = ["diff", "--no-color", "--no-ext-diff"];
    if req.mark == Mark::Untracked || (!req.in_repo && req.mark == Mark::Added) {
        return run(
            git(cwd)
                .args(base)
                .args(["--no-index", "--", "/dev/null"])
                .arg(&req.rel),
            true,
        );
    }
    if !req.in_repo {
        return Err("no diff outside a git repository".into());
    }
    let mut paths = vec![req.rel.clone()];
    paths.extend(req.orig.clone());
    let has_head = git(cwd)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .is_ok_and(|o| o.status.success());
    if has_head {
        return run(git(cwd).args(base).args(["HEAD", "--"]).args(&paths), false);
    }
    // No commit yet: what is staged, then what isn't.
    let staged = run(
        git(cwd).args(base).args(["--cached", "--"]).args(&paths),
        false,
    )?;
    let unstaged = run(git(cwd).args(base).arg("--").args(&paths), false)?;
    Ok(staged + &unstaged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::testutil::dump;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const SAMPLE: &str = "diff --git a/src/a.ts b/src/a.ts\nindex 111..222 100644\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,3 +1,3 @@\n keep\n-old line\n+new line\n--- dashes\n\\ No newline at end of file\n";

    fn press(v: &mut DiffView, code: KeyCode) -> bool {
        v.key(KeyEvent::from(code))
    }

    #[test]
    fn classifies_lines_and_counts() {
        let (lines, adds, dels) = parse_diff(SAMPLE);
        let kinds: Vec<Kind> = lines.iter().map(|l| l.kind).collect();
        use Kind::*;
        assert_eq!(
            kinds,
            [
                Header, Header, Header, Header, Hunk, Context, Del, Add, Del, Header
            ]
        );
        assert_eq!((adds, dels), (1, 2));
    }

    #[test]
    fn tabs_expand_and_controls_vanish() {
        let (lines, ..) = parse_diff("@@ -1 +1 @@\n+\tx\x1b[0m\n");
        assert_eq!(lines[1].text, "+   x[0m");
    }

    #[test]
    fn draws_title_and_cuts_long_lines() {
        let v = DiffView::from_text("src/a.ts", SAMPLE);
        let mut t = Terminal::new(TestBackend::new(24, 8)).unwrap();
        t.draw(|f| v.draw(f, f.area(), &Palette::default()))
            .unwrap();
        let out = dump(t.backend().buffer());
        let first = out.lines().next().unwrap();
        assert_eq!(first, "src/a.ts  +1 −2");
        assert_eq!(out.lines().nth(1).unwrap(), "diff --git a/src/a.ts b…");
        assert_eq!(out.lines().count(), 8);
        // Colors.
        let p = Palette::default();
        let buf = t.backend().buffer();
        assert_eq!(buf[(0, 5)].fg, p.muted); // @@
        assert_eq!(buf[(0, 7)].fg, p.danger); // -old line
        assert_eq!(buf[(10, 0)].fg, p.success); // +1 in the title
    }

    #[test]
    fn scrolls_clamped_and_closes() {
        let text: String = std::iter::once("@@ -1 +1 @@\n".to_string())
            .chain((0..40).map(|i| format!("+line {i}\n")))
            .collect();
        let mut v = DiffView::from_text("f", &text);
        let mut t = Terminal::new(TestBackend::new(20, 11)).unwrap();
        let draw = |v: &DiffView, t: &mut Terminal<TestBackend>| {
            t.draw(|f| v.draw(f, f.area(), &Palette::default()))
                .unwrap();
            dump(t.backend().buffer())
        };
        draw(&v, &mut t); // learns the height: 10 rows
        assert!(!press(&mut v, KeyCode::Down));
        assert!(
            draw(&v, &mut t)
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("+line 0")
        );
        press(&mut v, KeyCode::Char('G'));
        let out = draw(&v, &mut t);
        assert!(out.lines().last().unwrap().starts_with("+line 39"));
        press(&mut v, KeyCode::Down);
        assert_eq!(draw(&v, &mut t).lines().last().unwrap(), "+line 39");
        press(&mut v, KeyCode::PageUp);
        assert!(
            draw(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 30")
        );
        v.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(
            draw(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 25")
        );
        v.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(
            draw(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 30")
        );
        press(&mut v, KeyCode::Char('g'));
        assert!(draw(&v, &mut t).lines().nth(1).unwrap().starts_with("@@"));
        assert!(press(&mut v, KeyCode::Esc));
        assert!(press(&mut v, KeyCode::Char('q')));
    }

    #[test]
    fn empty_diff_says_so() {
        let v = DiffView::from_text("f", "");
        assert_eq!(v.lines()[0].text, "no changes");
        assert_eq!(v.counts(), (0, 0));
    }
}
