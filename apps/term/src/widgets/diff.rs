//! The diff overlay: `git diff` of one file, loaded in the background, drawn side by side (old
//! left, new right) when it is wide enough and as a unified diff otherwise, scrolled by keys.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use suite_renderer_ratatui::Palette;
use unicode_width::UnicodeWidthChar;

use super::files::Mark;
use super::git::git;
use super::syntax::{self, Class, Highlighter, syntax_for};
use super::text::{clean_line, truncate, truncate_start, width};

/// More lines than this are dropped (a generated file diff can be megabytes).
const MAX_LINES: usize = 20_000;
/// Lines of context around a change, and "the whole file" (`f`).
const CONTEXT: u32 = 5;
const FULL_CONTEXT: u32 = 99_999;
/// The overlay is split from this width on (`s` overrides); each side needs this much text.
const SPLIT_MIN: usize = 90;
const MIN_TEXT: usize = 8;
/// Cells one Left / Right moves.
const HSTEP: usize = 8;
/// Shown after a line whose file ends without a newline.
const NO_EOL: &str = "  [no newline at EOF]";

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

/// A span of chars, start and end.
type Range = (usize, usize);

/// One line of one side of the split view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Side {
    /// The line number in its file (1-based).
    pub no: usize,
    pub text: String,
    /// The changed part of a paired line, in chars of `text`.
    pub hl: Option<Range>,
    /// The file ends here without a newline.
    pub no_eol: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Row {
    /// `@@ -a,b +c,d @@ heading`, across both sides.
    Hunk(String),
    /// A context line (both sides) or a changed row: a removal on the left paired with an
    /// addition on the right, either of them absent when the other run was longer.
    Line {
        left: Option<Side>,
        right: Option<Side>,
        changed: bool,
    },
    /// A message across both sides.
    Note(String),
}

/// The diff as rows for the split view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Aligned {
    pub rows: Vec<Row>,
    /// Row index of every hunk separator.
    pub hunks: Vec<usize>,
    /// Digits of the widest line number (at least 3).
    pub gutter: usize,
    /// The widest line, in cells.
    pub max_w: usize,
    /// git said "new file mode" / "deleted file mode".
    pub new_file: bool,
    pub deleted: bool,
}

impl Aligned {
    fn has_lines(&self) -> bool {
        self.rows.iter().any(|r| matches!(r, Row::Line { .. }))
    }
}

/// What the last draw showed (the keys page and clamp by it).
#[derive(Debug, Clone, Copy)]
struct View {
    /// Rows of diff text.
    rows: usize,
    /// Cells of text in the narrowest column.
    col_w: usize,
    split: bool,
}

pub(crate) struct DiffView {
    title: String,
    cwd: PathBuf,
    req: Option<DiffRequest>,
    /// Lines of context: the whole file (`f`).
    full: bool,
    rx: Option<Receiver<Result<String, String>>>,
    /// The unified view.
    lines: Vec<DiffLine>,
    /// The split view.
    aligned: Aligned,
    /// The widest unified line, in cells.
    uni_w: usize,
    /// What each char of the code is, for colors: per unified line (its text after the sign)
    /// and per split row (left, right). Empty: not highlighted (yet).
    syntax: Highlights,
    /// The highlighting being worked out on a thread; the code shows plain until it arrives.
    syntax_rx: Option<Receiver<Highlights>>,
    adds: usize,
    dels: usize,
    loaded: bool,
    /// The top row, in the rows of the mode `scroll_split` says.
    scroll: Cell<usize>,
    scroll_split: Cell<bool>,
    hscroll: usize,
    /// `s`: split (true) or unified (false) whatever the width says.
    force: Option<bool>,
    /// The new-side line to scroll back to when a reload (`f`) arrives.
    anchor: Option<usize>,
    view: Cell<View>,
}

impl DiffView {
    /// Starts `git diff` for `req` on a thread: the changes with a few lines around them.
    #[cfg(test)]
    pub(crate) fn open(cwd: PathBuf, req: DiffRequest) -> DiffView {
        DiffView::start(cwd, req, false)
    }

    fn start(cwd: PathBuf, req: DiffRequest, full: bool) -> DiffView {
        let title = match &req.orig {
            Some(orig) => format!("{orig} → {}", req.rel),
            None => req.rel.clone(),
        };
        let rx = spawn(cwd.clone(), req.clone(), full);
        DiffView {
            cwd,
            req: Some(req),
            rx: Some(rx),
            full,
            ..DiffView::blank(title)
        }
    }

    /// The whole file with its changes marked, unified: what Enter on a file opens.
    pub(crate) fn open_file(cwd: PathBuf, req: DiffRequest) -> DiffView {
        DiffView {
            force: Some(false),
            ..DiffView::start(cwd, req, true)
        }
    }

    fn blank(title: String) -> DiffView {
        DiffView {
            title: clean_line(&title),
            cwd: PathBuf::new(),
            req: None,
            full: false,
            rx: None,
            lines: Vec::new(),
            aligned: Aligned::default(),
            uni_w: 0,
            syntax: Highlights::default(),
            syntax_rx: None,
            adds: 0,
            dels: 0,
            loaded: false,
            scroll: Cell::new(0),
            scroll_split: Cell::new(false),
            hscroll: 0,
            force: None,
            anchor: None,
            view: Cell::new(View {
                rows: 20,
                col_w: 40,
                split: false,
            }),
        }
    }

    /// A diff that is already at hand (the GitHub view's pull request diffs).
    pub(crate) fn from_text(title: &str, text: &str) -> DiffView {
        let mut v = DiffView::blank(title.to_string());
        v.finish(Ok(text.to_string()));
        v
    }

    /// A diff somebody else is fetching: loading until [`DiffView::finish`] is given the text.
    pub(crate) fn loading(title: &str) -> DiffView {
        DiffView::blank(title.to_string())
    }

    pub(crate) fn finish(&mut self, result: Result<String, String>) {
        let note = |text: String| {
            (
                vec![DiffLine {
                    text,
                    kind: Kind::Note,
                }],
                0,
                0,
            )
        };
        let (lines, adds, dels) = match result {
            Ok(text) if text.trim().is_empty() => note("no changes".into()),
            Ok(text) => {
                self.aligned = align(&text);
                parse_diff(&text)
            }
            Err(msg) => note(msg),
        };
        if lines.len() == 1 && lines[0].kind == Kind::Note {
            self.aligned = Aligned::default();
        }
        self.uni_w = lines.iter().map(|l| width(&l.text)).max().unwrap_or(0);
        let path = self.req.as_ref().map(|r| r.rel.clone());
        self.syntax = Highlights::default();
        let (tx, rx) = mpsc::channel();
        let (copy, aligned) = (lines.clone(), self.aligned.clone());
        std::thread::spawn(move || {
            let _ = tx.send(highlight(path.as_deref(), &copy, &aligned));
        });
        self.syntax_rx = Some(rx);
        self.lines = lines;
        self.adds = adds;
        self.dels = dels;
        self.loaded = true;
        self.rx = None;
        if let Some(line) = self.anchor.take() {
            let at = self
                .aligned
                .rows
                .iter()
                .position(|r| matches!(r, Row::Line { right: Some(s), .. } if s.no >= line));
            self.set_position(at.unwrap_or(0));
        }
    }

    /// Picks up the loaded diff, or its highlighting. True when something arrived.
    pub(crate) fn poll(&mut self) -> bool {
        if let Some(rx) = &self.syntax_rx
            && let Ok(syntax) = rx.try_recv()
        {
            self.syntax = syntax;
            self.syntax_rx = None;
            return true;
        }
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

    /// Rows of the shown mode.
    fn len(&self, split: bool) -> usize {
        if split {
            self.aligned.rows.len()
        } else {
            self.lines.len()
        }
    }

    fn hunk_starts(&self, split: bool) -> Vec<usize> {
        if split {
            self.aligned.hunks.clone()
        } else {
            (0..self.lines.len())
                .filter(|&i| self.lines[i].kind == Kind::Hunk)
                .collect()
        }
    }

    fn max_scroll(&self) -> usize {
        let v = self.view.get();
        self.len(v.split).saturating_sub(v.rows.max(1))
    }

    fn max_hscroll(&self) -> usize {
        let v = self.view.get();
        let widest = if v.split {
            self.aligned.max_w
        } else {
            self.uni_w
        };
        widest.saturating_sub(v.col_w)
    }

    /// The top row of the other mode: the start of the hunk the top row is in.
    fn convert(&self, pos: usize, from_split: bool) -> usize {
        let from = self.hunk_starts(from_split);
        let to = self.hunk_starts(!from_split);
        match from.iter().rposition(|&h| h <= pos) {
            Some(i) => to.get(i).copied().unwrap_or(0),
            None => 0,
        }
    }

    /// Scrolls to a row of the split view, whatever mode is shown.
    fn set_position(&self, row: usize) {
        let split = self.view.get().split;
        self.scroll
            .set(if split { row } else { self.convert(row, true) });
        self.scroll_split.set(split);
    }

    /// The new-side line number at the top of the view.
    fn top_new_line(&self) -> Option<usize> {
        let v = self.view.get();
        let top = self.scroll.get().min(self.max_scroll());
        let top = if v.split {
            top
        } else {
            self.convert(top, false)
        };
        self.aligned.rows[top.min(self.aligned.rows.len())..]
            .iter()
            .find_map(|r| match r {
                Row::Line { right: Some(s), .. } => Some(s.no),
                _ => None,
            })
    }

    /// Asks git again with the whole file as context, or back with a few lines.
    fn toggle_full(&mut self) {
        // A diff that did not come from git here (a pull request's) is all there is.
        let Some(req) = self.req.clone() else {
            return;
        };
        self.full = !self.full;
        self.anchor = self.top_new_line();
        self.rx = Some(spawn(self.cwd.clone(), req, self.full));
        self.loaded = false;
    }

    /// Scrolls; true when the key closes the overlay (Esc, q).
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        let v = self.view.get();
        let rows = v.rows.max(1);
        let page = rows.saturating_sub(1).max(1);
        let half = (rows / 2).max(1);
        let max = self.max_scroll();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let mut s = self.scroll.get().min(max);
        match (key.code, ctrl) {
            (KeyCode::Esc, _) | (KeyCode::Char('q'), false) => return true,
            (KeyCode::Up | KeyCode::Char('k'), _) => s = s.saturating_sub(1),
            (KeyCode::Down | KeyCode::Char('j'), _) => s = (s + 1).min(max),
            (KeyCode::PageUp | KeyCode::Char('b'), _) => s = s.saturating_sub(page),
            (KeyCode::PageDown | KeyCode::Char(' '), _) => s = (s + page).min(max),
            (KeyCode::Char('u'), true) => s = s.saturating_sub(half),
            (KeyCode::Char('d'), true) => s = (s + half).min(max),
            (KeyCode::Home | KeyCode::Char('g'), _) => s = 0,
            (KeyCode::End | KeyCode::Char('G'), _) => s = max,
            (KeyCode::Char('n'), false) => {
                if let Some(h) = self.hunk_starts(v.split).into_iter().find(|&h| h > s) {
                    s = h.min(max);
                }
            }
            (KeyCode::Char('p'), false) => {
                let before = self.hunk_starts(v.split).into_iter().rfind(|&h| h < s);
                s = before.unwrap_or(0);
            }
            (KeyCode::Left | KeyCode::Char('h'), _) => {
                self.hscroll = self.hscroll.saturating_sub(HSTEP);
            }
            (KeyCode::Right | KeyCode::Char('l'), _) => {
                self.hscroll = (self.hscroll + HSTEP).min(self.max_hscroll());
            }
            (KeyCode::Char('0'), false) => self.hscroll = 0,
            (KeyCode::Char('s'), false) => self.force = Some(!v.split),
            (KeyCode::Char('f'), false) => self.toggle_full(),
            _ => {}
        }
        self.scroll.set(s);
        false
    }

    /// The columns of the split view at this width, None when the unified view is shown.
    fn split_geom(&self, w: usize) -> Option<Geom> {
        if !self.aligned.has_lines() || !self.force.unwrap_or(w >= SPLIT_MIN) {
            return None;
        }
        let gutter = self.aligned.gutter + 3;
        let left = w.saturating_sub(1) / 2;
        let right = w.saturating_sub(1) - left;
        // The left column is the narrower one.
        if left < gutter + MIN_TEXT {
            return None;
        }
        Some(Geom {
            gutter: self.aligned.gutter,
            left,
            right,
        })
    }

    /// Fills `area`: a title line (and the column headers of the split view), then the diff,
    /// long lines cut.
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        frame.render_widget(Clear, area);
        let w = area.width as usize;
        let look = Look::new(palette);
        let geom = self.split_geom(w);
        let split = geom.is_some();
        let head = if split { 2 } else { 1 };
        let rows = (area.height as usize).saturating_sub(head);
        let col_w = geom.map_or(w.saturating_sub(1), |g| g.left - g.gutter - 3);
        // A resize (or `s`) changed the mode: land on the same hunk.
        if self.scroll_split.get() != split {
            let pos = self.convert(self.scroll.get(), self.scroll_split.get());
            self.scroll.set(pos);
            self.scroll_split.set(split);
        }
        self.view.set(View { rows, col_w, split });
        let n = self.len(split);
        let first = self.scroll.get().min(n.saturating_sub(rows.max(1)));
        let hscroll = self.hscroll.min(self.max_hscroll());

        let shown = n.saturating_sub(first).min(rows);
        let title = self.title_line(w, first, shown, n, &look);
        let mut lines = vec![title];
        if let Some(g) = geom {
            lines.push(self.column_header(&g, &look));
            lines.extend(self.split_lines(&g, first, rows, hscroll, &look));
        } else {
            lines.extend(self.unified_lines(w, first, rows, hscroll, &look));
        }
        lines.truncate(area.height as usize);
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// The path, the counts of added and removed lines, and where the view is.
    fn title_line(
        &self,
        w: usize,
        first: usize,
        shown: usize,
        n: usize,
        look: &Look,
    ) -> Line<'static> {
        let muted = look.muted;
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
                Style::new().fg(look.success),
            ));
            title.push(Span::raw(" "));
            title.push(Span::styled(
                format!("−{}", self.dels),
                Style::new().fg(look.danger),
            ));
        }
        if self.full {
            title.push(Span::styled("  full file", muted));
        }
        let used: usize = title.iter().map(|s| width(&s.content)).sum();
        if shown > 0 {
            let (a, b) = (first + 1, first + shown);
            let long = format!("lines {a}–{b} of {n}");
            let short = format!("{a}–{b}/{n}");
            if let Some(pos) = [long, short].into_iter().find(|p| used + 2 + width(p) <= w) {
                title.push(Span::raw(" ".repeat(w - used - width(&pos))));
                title.push(Span::styled(pos, muted));
            }
        }
        Line::from(title)
    }

    /// "HEAD │ working tree" over the two columns.
    fn column_header(&self, g: &Geom, look: &Look) -> Line<'static> {
        let rename = self
            .req
            .as_ref()
            .and_then(|r| r.orig.as_ref().map(|o| (o, &r.rel)));
        let left = if self.aligned.new_file {
            "(new file)".to_string()
        } else if let Some((orig, _)) = rename {
            format!("HEAD · {}", clean_line(orig))
        } else {
            "HEAD".to_string()
        };
        let right = if self.aligned.deleted {
            "(deleted)".to_string()
        } else if let Some((_, rel)) = rename {
            format!("working tree · {}", clean_line(rel))
        } else {
            "working tree".to_string()
        };
        let style = with_bg(look.muted.add_modifier(Modifier::BOLD), look.band);
        Line::from(vec![
            Span::styled(fit(&format!(" {left}"), g.left), style),
            Span::styled("│", look.border),
            Span::styled(fit(&format!(" {right}"), g.right), style),
        ])
    }

    /// `rows` lines of the split view from row `first`, each exactly as wide as the overlay.
    fn split_lines(
        &self,
        g: &Geom,
        first: usize,
        rows: usize,
        hscroll: usize,
        look: &Look,
    ) -> Vec<Line<'static>> {
        let total = g.left + 1 + g.right;
        let mut out = Vec::with_capacity(rows);
        for (index, row) in self.aligned.rows.iter().enumerate().skip(first).take(rows) {
            let spans = match row {
                Row::Hunk(text) => {
                    let style = with_bg(look.muted, look.band);
                    let text = format!("⋯ {text}");
                    let chars = text.chars().map(|c| (c, 0));
                    windowed(chars, width(&text), 0, total, &[style; 3], true)
                }
                Row::Note(text) => {
                    let chars = text.chars().map(|c| (c, 0));
                    windowed(chars, width(text), 0, total, &[look.muted; 3], true)
                }
                Row::Line {
                    left,
                    right,
                    changed,
                } => {
                    let (lc, rc) = match self.syntax.split.get(index) {
                        Some((l, r)) => (l.as_slice(), r.as_slice()),
                        None => (&[][..], &[][..]),
                    };
                    let (lt, rt) = if *changed {
                        (Tone::Del, Tone::Add)
                    } else {
                        (Tone::Ctx, Tone::Ctx)
                    };
                    let mut spans =
                        side_spans(left.as_ref(), lc, lt, g.gutter, g.left, hscroll, look);
                    spans.push(Span::styled("│", look.border));
                    spans.extend(side_spans(
                        right.as_ref(),
                        rc,
                        rt,
                        g.gutter,
                        g.right,
                        hscroll,
                        look,
                    ));
                    spans
                }
            };
            out.push(Line::from(spans));
        }
        // Below the last row the columns go on.
        while out.len() < rows {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(g.left)),
                Span::styled("│", look.border),
                Span::raw(" ".repeat(g.right)),
            ]));
        }
        out
    }

    /// `rows` lines of the unified view from line `first`.
    fn unified_lines(
        &self,
        w: usize,
        first: usize,
        rows: usize,
        hscroll: usize,
        look: &Look,
    ) -> Vec<Line<'static>> {
        self.lines
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(index, l)| {
                let style = match l.kind {
                    Kind::Add => Style::new().fg(look.success),
                    Kind::Del => Style::new().fg(look.danger),
                    Kind::Hunk | Kind::Header | Kind::Note => look.muted,
                    Kind::Context => Style::new(),
                };
                // The +/-/space stays put when scrolling sideways.
                let (mark, body) = match l.kind {
                    Kind::Add | Kind::Del | Kind::Context => l
                        .text
                        .split_at(l.text.chars().next().map_or(0, char::len_utf8)),
                    _ => ("", l.text.as_str()),
                };
                let mut spans = vec![Span::styled(mark.to_string(), style)];
                let avail = w - width(mark);
                let classes = self
                    .syntax
                    .unified
                    .get(index)
                    .map_or(&[][..], Vec::as_slice);
                // Highlighted code keeps the change's color as a background tint.
                let tone = match l.kind {
                    Kind::Add => Tone::Add,
                    Kind::Del => Tone::Del,
                    _ => Tone::Ctx,
                };
                let lit = !classes.is_empty() && look.syntax.is_some();
                let base = if lit {
                    with_bg(Style::new(), look.bg(tone))
                } else {
                    style
                };
                let mut styles = vec![base, base, base];
                let chars: Vec<(char, usize)> = body
                    .chars()
                    .enumerate()
                    .map(|(i, c)| match look.code(classes.get(i).copied()) {
                        Some(code) => (c, style_id(&mut styles, base.patch(code))),
                        None => (c, 0),
                    })
                    .collect();
                spans.extend(windowed(
                    chars.into_iter(),
                    width(body),
                    hscroll,
                    avail,
                    &styles,
                    lit && tone != Tone::Ctx,
                ));
                Line::from(spans)
            })
            .collect()
    }
}

/// The two columns of the split view (separator excluded): their widths in cells and the
/// digits of a line number.
#[derive(Debug, Clone, Copy)]
struct Geom {
    gutter: usize,
    left: usize,
    right: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Ctx,
    Del,
    Add,
}

/// The colors of the split view, mixed from the palette (none without colors).
struct Look {
    muted: Style,
    border: Style,
    success: Color,
    danger: Color,
    del_bg: Option<Color>,
    del_hl: Option<Color>,
    add_bg: Option<Color>,
    add_hl: Option<Color>,
    /// The blank half of an uneven run, and the bands (hunk separators, column headers).
    band: Option<Color>,
    /// The colors of code, by class; None without true colors (code stays plain).
    syntax: Option<[Style; 7]>,
}

impl Look {
    fn new(p: &Palette) -> Look {
        Look {
            muted: Style::new().fg(p.muted),
            border: Style::new().fg(p.border),
            success: p.success,
            danger: p.danger,
            del_bg: blend(p.bg, p.danger, 16),
            del_hl: blend(p.bg, p.danger, 38),
            add_bg: blend(p.bg, p.success, 16),
            add_hl: blend(p.bg, p.success, 38),
            band: matches!(p.surface, Color::Rgb(..)).then_some(p.surface),
            syntax: code_colors(p),
        }
    }

    /// The style a class of code is drawn with (patched over the row's), if code is colored.
    fn code(&self, class: Option<Class>) -> Option<Style> {
        Some(self.syntax?[class? as usize])
    }

    fn bg(&self, tone: Tone) -> Option<Color> {
        match tone {
            Tone::Ctx => None,
            Tone::Del => self.del_bg,
            Tone::Add => self.add_bg,
        }
    }

    /// The text of a row.
    fn base(&self, tone: Tone) -> Style {
        let fg = match tone {
            Tone::Ctx => return Style::new(),
            Tone::Del => self.danger,
            Tone::Add => self.success,
        };
        with_bg(Style::new().fg(fg), self.bg(tone))
    }

    /// The changed part of a paired line; reversed when there are no colors to mix.
    fn changed(&self, tone: Tone) -> Style {
        let hl = if tone == Tone::Del {
            self.del_hl
        } else {
            self.add_hl
        };
        match hl {
            Some(c) => self.base(tone).bg(c),
            None => self.base(tone).add_modifier(Modifier::REVERSED),
        }
    }
}

/// Colors of code, picked for a dark or a light background rather than taken from the theme:
/// its few colors (an accent as green as its success color) can't tell five classes apart.
fn code_colors(p: &Palette) -> Option<[Style; 7]> {
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
    // In the order of `Class`.
    Some([
        Style::new().fg(p.fg),
        Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
        fg(string),
        fg(keyword),
        fg(constant),
        fg(function),
        fg(kind),
    ])
}

/// What each char of a diff is, per unified line and per split row (left, right).
#[derive(Debug, Default)]
struct Highlights {
    unified: Vec<Vec<Class>>,
    split: Vec<(Vec<Class>, Vec<Class>)>,
}

/// Highlights the diff's lines with the grammar for `path` (a pull request's diff: the path in
/// each file's `+++ b/` header). Old and new lines are parsed apart, each in order, and a hunk
/// starts the parse over.
fn highlight(path: Option<&str>, lines: &[DiffLine], aligned: &Aligned) -> Highlights {
    let mut out = Highlights::default();
    if lines.len() > syntax::MAX_LINES {
        return out;
    }
    let header_path = |l: &DiffLine| l.text.strip_prefix("+++ b/").map(str::to_string);
    let first = path
        .map(str::to_string)
        .or_else(|| lines.iter().find_map(header_path));
    let pair = |path: Option<&str>| {
        path.and_then(syntax_for)
            .map(|s| (Highlighter::new(s), Highlighter::new(s)))
    };
    let mut hl = pair(first.as_deref());
    for l in lines {
        let body = l
            .text
            .get(l.text.chars().next().map_or(0, char::len_utf8)..)
            .unwrap_or("");
        let classes = match (l.kind, hl.as_mut()) {
            (Kind::Header, _) => {
                if path.is_none()
                    && let Some(p) = header_path(l)
                {
                    hl = pair(Some(&p));
                }
                Vec::new()
            }
            (Kind::Hunk, Some((old, new))) => {
                old.reset();
                new.reset();
                Vec::new()
            }
            (Kind::Context, Some((old, new))) => {
                old.line(body);
                new.line(body)
            }
            (Kind::Add, Some((_, new))) => new.line(body),
            (Kind::Del, Some((old, _))) => old.line(body),
            _ => Vec::new(),
        };
        out.unified.push(classes);
    }
    let Some((mut old, mut new)) = pair(first.as_deref()) else {
        return out;
    };
    for row in &aligned.rows {
        let classes = match row {
            Row::Hunk(_) => {
                old.reset();
                new.reset();
                (Vec::new(), Vec::new())
            }
            Row::Line {
                left,
                right,
                changed: false,
            } => {
                let l = left.as_ref().map(|s| old.line(&s.text)).unwrap_or_default();
                let r = right
                    .as_ref()
                    .map(|s| new.line(&s.text))
                    .unwrap_or_default();
                (l, r)
            }
            Row::Line { left, right, .. } => (
                left.as_ref().map(|s| old.line(&s.text)).unwrap_or_default(),
                right
                    .as_ref()
                    .map(|s| new.line(&s.text))
                    .unwrap_or_default(),
            ),
            Row::Note(_) => (Vec::new(), Vec::new()),
        };
        out.split.push(classes);
    }
    out
}

fn with_bg(style: Style, bg: Option<Color>) -> Style {
    match bg {
        Some(c) => style.bg(c),
        None => style,
    }
}

/// `over` mixed into `base` by `pct` percent; None unless both are true colors.
fn blend(base: Color, over: Color, pct: u32) -> Option<Color> {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (base, over) else {
        return None;
    };
    let mix = |a: u8, b: u8| ((u32::from(a) * (100 - pct) + u32::from(b) * pct) / 100) as u8;
    Some(Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1)))
}

/// `s` cut or padded to exactly `n` cells.
fn fit(s: &str, n: usize) -> String {
    let mut out = truncate(s, n);
    out.push_str(&" ".repeat(n.saturating_sub(width(&out))));
    out
}

/// One column of a split row, exactly `col` cells: the line number, a sign, the text from cell
/// `skip` on (or a blank filler when this side has no line).
fn side_spans(
    side: Option<&Side>,
    classes: &[Class],
    tone: Tone,
    gutter: usize,
    col: usize,
    skip: usize,
    look: &Look,
) -> Vec<Span<'static>> {
    let Some(side) = side else {
        return vec![Span::styled(
            " ".repeat(col),
            with_bg(Style::new(), look.band),
        )];
    };
    let base = look.base(tone);
    let bg = with_bg(Style::new(), look.bg(tone));
    let sign = match tone {
        Tone::Ctx => " ",
        Tone::Del => "-",
        Tone::Add => "+",
    };
    let mut spans = vec![
        Span::styled(format!("{:>gutter$} ", side.no), bg.patch(look.muted)),
        Span::styled(sign, base),
        Span::styled(" ", bg),
    ];
    let avail = col.saturating_sub(gutter + 3);
    let hl = side.hl.filter(|_| tone != Tone::Ctx);
    // Highlighted code keeps the change's color as a background tint.
    let mut styles = vec![base, look.changed(tone), bg.patch(look.muted)];
    let chars: Vec<(char, usize)> = side
        .text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let changed = usize::from(hl.is_some_and(|(a, b)| (a..b).contains(&i)));
            let id = match look.code(classes.get(i).copied()) {
                Some(code) => {
                    let style = styles[changed].patch(code);
                    style_id(&mut styles, style)
                }
                None => changed,
            };
            (c, id)
        })
        .collect();
    let eol = side
        .no_eol
        .then_some(NO_EOL)
        .into_iter()
        .flat_map(str::chars);
    let total = width(&side.text) + if side.no_eol { width(NO_EOL) } else { 0 };
    spans.extend(windowed(
        chars.into_iter().chain(eol.map(|c| (c, 2))),
        total,
        skip,
        avail,
        &styles,
        true,
    ));
    spans
}

/// The index of `style` in `styles`, added when it isn't there yet.
fn style_id(styles: &mut Vec<Style>, style: Style) -> usize {
    match styles.iter().position(|s| *s == style) {
        Some(i) => i,
        None => {
            styles.push(style);
            styles.len() - 1
        }
    }
}

/// The cells `skip..skip + avail` of a line of `(char, style index)`, as spans. A wide
/// character cut by the left edge shows as blanks; when the line goes on past the right edge
/// the last cell is a `…`. Padded to `avail` cells with the first style when `pad`.
fn windowed(
    chars: impl Iterator<Item = (char, usize)>,
    total: usize,
    skip: usize,
    avail: usize,
    styles: &[Style],
    pad: bool,
) -> Vec<Span<'static>> {
    if avail == 0 {
        return Vec::new();
    }
    let more = total > skip + avail;
    let limit = if more { avail - 1 } else { avail };
    let mut runs: Vec<(String, usize)> = Vec::new();
    let mut push = |s: &str, id: usize| match runs.last_mut() {
        Some((text, last)) if *last == id => text.push_str(s),
        _ => runs.push((s.to_string(), id)),
    };
    let (mut pos, mut used) = (0, 0);
    for (c, id) in chars {
        let w = c.width().unwrap_or(0);
        let start = pos;
        pos += w;
        if start < skip {
            if w > 0 && pos > skip {
                let n = (pos - skip).min(limit - used);
                push(&" ".repeat(n), id);
                used += n;
            }
            continue;
        }
        if used + w > limit {
            break;
        }
        push(c.encode_utf8(&mut [0; 4]), id);
        used += w;
    }
    if more {
        push("…", 2);
        used += 1;
    }
    if pad && used < avail {
        push(&" ".repeat(avail - used), 0);
    }
    runs.into_iter()
        .map(|(text, id)| Span::styled(text, styles[id]))
        .collect()
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

/// The old and new first line numbers of `@@ -a,b +c,d @@`.
fn hunk_start(header: &str) -> (usize, usize) {
    let mut nums = header
        .split_whitespace()
        .filter(|t| t.starts_with(['-', '+']))
        .map(|t| t[1..].split(',').next().and_then(|n| n.parse().ok()));
    (
        nums.next().flatten().unwrap_or(1),
        nums.next().flatten().unwrap_or(1),
    )
}

/// Which run the last diff line belongs to (a `\ No newline` line refers to it).
#[derive(Clone, Copy)]
enum Last {
    None,
    Del,
    Add,
    Ctx,
}

/// Pairs the pending removals and additions row by row.
fn flush(rows: &mut Vec<Row>, dels: &mut Vec<Side>, adds: &mut Vec<Side>) {
    let (dels, adds) = (std::mem::take(dels), std::mem::take(adds));
    let n = dels.len().max(adds.len());
    let (mut dels, mut adds) = (dels.into_iter(), adds.into_iter());
    for _ in 0..n {
        let (mut left, mut right) = (dels.next(), adds.next());
        if let (Some(l), Some(r)) = (&mut left, &mut right) {
            (l.hl, r.hl) = intra(&l.text, &r.text);
        }
        rows.push(Row::Line {
            left,
            right,
            changed: true,
        });
    }
}

/// The unified diff as aligned rows: context on both sides; in a hunk each run of removals is
/// paired row by row with the run of additions after it, the shorter side left blank.
pub(crate) fn align(text: &str) -> Aligned {
    let mut a = Aligned::default();
    let (mut old_no, mut new_no) = (1, 1);
    let mut in_hunk = false;
    let (mut dels, mut adds) = (Vec::new(), Vec::new());
    let mut last = Last::None;
    let side = |no: usize, text: &str| Side {
        no,
        text: clean_line(text),
        hl: None,
        no_eol: false,
    };
    for raw in text.lines().take(MAX_LINES) {
        if raw.starts_with("diff ") {
            flush(&mut a.rows, &mut dels, &mut adds);
            in_hunk = false;
        } else if raw.starts_with("@@") {
            flush(&mut a.rows, &mut dels, &mut adds);
            in_hunk = true;
            last = Last::None;
            (old_no, new_no) = hunk_start(raw);
            a.hunks.push(a.rows.len());
            a.rows.push(Row::Hunk(clean_line(raw)));
        } else if !in_hunk {
            a.new_file |= raw.starts_with("new file mode");
            a.deleted |= raw.starts_with("deleted file mode");
        } else if raw.starts_with('\\') {
            let marked: Vec<&mut Side> = match last {
                Last::Del => dels.last_mut().into_iter().collect(),
                Last::Add => adds.last_mut().into_iter().collect(),
                Last::Ctx => match a.rows.last_mut() {
                    Some(Row::Line { left, right, .. }) => {
                        left.iter_mut().chain(right.iter_mut()).collect()
                    }
                    _ => Vec::new(),
                },
                Last::None => Vec::new(),
            };
            for s in marked {
                s.no_eol = true;
            }
        } else if let Some(rest) = raw.strip_prefix('-') {
            if !adds.is_empty() {
                flush(&mut a.rows, &mut dels, &mut adds);
            }
            dels.push(side(old_no, rest));
            old_no += 1;
            last = Last::Del;
        } else if let Some(rest) = raw.strip_prefix('+') {
            adds.push(side(new_no, rest));
            new_no += 1;
            last = Last::Add;
        } else {
            flush(&mut a.rows, &mut dels, &mut adds);
            let rest = raw.strip_prefix(' ').unwrap_or(raw);
            a.rows.push(Row::Line {
                left: Some(side(old_no, rest)),
                right: Some(side(new_no, rest)),
                changed: false,
            });
            old_no += 1;
            new_no += 1;
            last = Last::Ctx;
        }
    }
    flush(&mut a.rows, &mut dels, &mut adds);
    if text.lines().nth(MAX_LINES).is_some() {
        a.rows
            .push(Row::Note(format!("… cut after {MAX_LINES} lines")));
    }
    let mut most = 0;
    for row in &a.rows {
        if let Row::Line { left, right, .. } = row {
            for s in left.iter().chain(right) {
                most = most.max(s.no);
                let eol = if s.no_eol { width(NO_EOL) } else { 0 };
                a.max_w = a.max_w.max(width(&s.text) + eol);
            }
        }
    }
    a.gutter = most.to_string().len().max(3);
    a
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The parts that differ between a removed and the added line that replaces it, as char ranges:
/// what is left after the common start and end, widened to whole words. None for a side when
/// nothing of it remains or all of it does.
fn intra(old: &str, new: &str) -> (Option<Range>, Option<Range>) {
    let (a, b): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let most = a.len().min(b.len());
    let mut p = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut s = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(most - p)
        .take_while(|(x, y)| x == y)
        .count();
    while p > 0
        && is_word(a[p - 1])
        && (a.get(p).is_some_and(|&c| is_word(c)) || b.get(p).is_some_and(|&c| is_word(c)))
    {
        p -= 1;
    }
    while s > 0 && is_word(a[a.len() - s]) {
        let (ia, ib) = (a.len() - s, b.len() - s);
        if (ia > 0 && is_word(a[ia - 1])) || (ib > 0 && is_word(b[ib - 1])) {
            s -= 1;
        } else {
            break;
        }
    }
    let range = |len: usize| {
        let r = (p, len - s);
        (r.0 < r.1 && r != (0, len)).then_some(r)
    };
    (range(a.len()), range(b.len()))
}

/// Starts `git diff` on a thread.
fn spawn(cwd: PathBuf, req: DiffRequest, full: bool) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let loaded = load(&cwd, &req, full);
        // The whole file asked for, and git sees no change in it: the file as it is.
        let loaded = match loaded {
            Ok(text) if full && text.trim().is_empty() => file_as_diff(&cwd, &req.rel),
            other => other,
        };
        let _ = tx.send(loaded);
    });
    rx
}

/// Files larger than this aren't shown whole.
const FILE_LIMIT: u64 = 2 << 20;

/// A file without changes, as a diff of only context lines, so the view shows it like any other.
fn file_as_diff(cwd: &Path, rel: &str) -> Result<String, String> {
    let path = cwd.join(rel);
    let meta = std::fs::metadata(&path).map_err(|e| format!("{rel}: {e}"))?;
    if meta.len() > FILE_LIMIT {
        return Err(format!("{rel} is larger than 2 MB"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
    if bytes.contains(&0) {
        return Err(format!("{rel} is a binary file"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = format!(
        "--- a/{rel}\n+++ b/{rel}\n@@ -1,{n} +1,{n} @@\n",
        n = lines.len()
    );
    for line in lines {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
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
/// for untracked ones. `full` keeps the whole file as context.
fn load(cwd: &Path, req: &DiffRequest, full: bool) -> Result<String, String> {
    if req.rel.ends_with('/') {
        return Err("untracked directory".into());
    }
    let context = format!("-U{}", if full { FULL_CONTEXT } else { CONTEXT });
    let base = ["diff", "--no-color", "--no-ext-diff", &context];
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

    /// Three hunks: a changed line, an uneven run with a tab, a pure addition, a long line and a
    /// file without a final newline.
    const BIG: &str = "diff --git a/src/app.ts b/src/app.ts
index 1111111..2222222 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,6 +1,6 @@ imports
 import { readFile } from \"node:fs\";
-import { join } from \"node:path\";
+import { join, resolve } from \"node:path\";
 
 const limit = 10;
 const name = \"app\";
@@ -20,8 +20,9 @@ export function load(path: string) {
 \tconst text = readFile(path);
-\tif (!text) return null;
-\tconst parsed = JSON.parse(text);
-\treturn parsed;
+\tif (!text) {
+\t\treturn null;
+\t}
+\treturn JSON.parse(text);
 }
 
 export const version = 3;
@@ -40,4 +41,7 @@ export function save(path: string, data: unknown) {
 \twriteFile(path, JSON.stringify(data));
+\t// Keep a backup next to the file.
+\tcopyFile(path, path + \".bak\");
+\tlog(\"saved\");
 }
-export const long = \"a very long line that does not fit into a column of the split view, so it is cut and can be scrolled sideways\";
+export const long = \"a very long line that does not fit into a column of the split view, so it is cut and can be scrolled sideways!\";
-last
\\ No newline at end of file
+last line
\\ No newline at end of file
";

    fn press(v: &mut DiffView, code: KeyCode) -> bool {
        v.key(KeyEvent::from(code))
    }

    fn ch(v: &mut DiffView, c: char) -> bool {
        press(v, KeyCode::Char(c))
    }

    fn draw_to(v: &DiffView, t: &mut Terminal<TestBackend>) -> String {
        t.draw(|f| v.draw(f, f.area(), &Palette::default()))
            .unwrap();
        dump(t.backend().buffer())
    }

    fn render(v: &DiffView, w: u16, h: u16) -> String {
        draw_to(v, &mut Terminal::new(TestBackend::new(w, h)).unwrap())
    }

    fn snapshot(name: &str, actual: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/snapshots")
            .join(format!("{name}.txt"));
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("{actual}\n")).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("missing snapshot {name} (UPDATE_SNAPSHOTS=1 to write)\n{actual}")
        });
        assert_eq!(actual, expected.trim_end_matches('\n'), "snapshot {name}");
    }

    /// Rows as `old | new`: line number, sign, text; `∅` for a blank side, `¬` after a line
    /// without a final newline.
    fn show(a: &Aligned) -> Vec<String> {
        let side = |s: &Option<Side>, sign: char| match s {
            Some(s) => format!(
                "{}{sign}{}{}",
                s.no,
                s.text,
                if s.no_eol { " ¬" } else { "" }
            ),
            None => "∅".to_string(),
        };
        a.rows
            .iter()
            .map(|r| match r {
                Row::Hunk(h) => format!("@ {h}"),
                Row::Note(n) => format!("! {n}"),
                Row::Line {
                    left,
                    right,
                    changed,
                } => {
                    let (l, r) = if *changed { ('-', '+') } else { (' ', ' ') };
                    format!("{} | {}", side(left, l), side(right, r))
                }
            })
            .collect()
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
        let a = align("@@ -1 +1 @@\n+\tx\x1b[0m\n");
        assert_eq!(show(&a)[1], "∅ | 1+    x[0m");
    }

    #[test]
    fn aligns_rows() {
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "modify",
                "@@ -1,3 +1,3 @@\n keep\n-old\n+new\n tail\n",
                &[
                    "@ @@ -1,3 +1,3 @@",
                    "1 keep | 1 keep",
                    "2-old | 2+new",
                    "3 tail | 3 tail",
                ],
            ),
            (
                "pure add",
                "@@ -0,0 +1,2 @@\n+a\n+b\n",
                &["@ @@ -0,0 +1,2 @@", "∅ | 1+a", "∅ | 2+b"],
            ),
            (
                "pure delete",
                "@@ -3,2 +2,0 @@\n-x\n-y\n",
                &["@ @@ -3,2 +2,0 @@", "3-x | ∅", "4-y | ∅"],
            ),
            (
                "two hunks",
                "@@ -1,2 +1,2 @@ fn a\n c\n-d\n+D\n@@ -10,2 +10,3 @@ fn b\n e\n+f\n g\n",
                &[
                    "@ @@ -1,2 +1,2 @@ fn a",
                    "1 c | 1 c",
                    "2-d | 2+D",
                    "@ @@ -10,2 +10,3 @@ fn b",
                    "10 e | 10 e",
                    "∅ | 11+f",
                    "11 g | 12 g",
                ],
            ),
            (
                "more removed than added",
                "@@ -1,3 +1 @@\n-a\n-b\n-c\n+X\n",
                &["@ @@ -1,3 +1 @@", "1-a | 1+X", "2-b | ∅", "3-c | ∅"],
            ),
            (
                "more added than removed",
                "@@ -1,2 +1,4 @@\n-a\n-b\n+1\n+2\n+3\n+4\n z\n",
                &[
                    "@ @@ -1,2 +1,4 @@",
                    "1-a | 1+1",
                    "2-b | 2+2",
                    "∅ | 3+3",
                    "∅ | 4+4",
                    "3 z | 5 z",
                ],
            ),
            (
                "two runs between context",
                "@@ -1,5 +1,5 @@\n-a\n+A\n m\n-b\n-c\n+B\n+C\n",
                &[
                    "@ @@ -1,5 +1,5 @@",
                    "1-a | 1+A",
                    "2 m | 2 m",
                    "3-b | 3+B",
                    "4-c | 4+C",
                ],
            ),
            (
                "no newline at the end of the old file",
                "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n",
                &["@ @@ -1 +1 @@", "1-old ¬ | 1+new"],
            ),
            (
                "no newline at the end of the new file",
                "@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n",
                &["@ @@ -1 +1 @@", "1-old | 1+new ¬"],
            ),
            (
                "no newline after context",
                "@@ -1,2 +1,2 @@\n same\n\\ No newline at end of file\n",
                &["@ @@ -1,2 +1,2 @@", "1 same ¬ | 1 same ¬"],
            ),
            (
                "an empty context line",
                "@@ -1,3 +1,3 @@\n a\n\n-b\n+B\n",
                &["@ @@ -1,3 +1,3 @@", "1 a | 1 a", "2  | 2 ", "3-b | 3+B"],
            ),
            (
                "a removed line that looks like a header",
                "@@ -1,2 +1,2 @@\n--- dashes\n+++ plus\n",
                &["@ @@ -1,2 +1,2 @@", "1--- dashes | 1+++ plus"],
            ),
        ];
        for (name, diff, want) in cases {
            let full = format!("diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n{diff}");
            assert_eq!(show(&align(&full)), *want, "{name}");
        }
    }

    #[test]
    fn aligned_knows_hunks_flags_and_widths() {
        let a = align(BIG);
        assert_eq!(a.hunks.len(), 3);
        assert!(a.hunks.iter().all(|&i| matches!(a.rows[i], Row::Hunk(_))));
        assert_eq!(a.gutter, 3);
        assert!(a.max_w > 100);
        assert!(!a.new_file && !a.deleted);
        let new = align(
            "diff --git a/n b/n\nnew file mode 100644\nindex 0..1\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+x\n",
        );
        assert!(new.new_file && !new.deleted);
        let gone = align(
            "diff --git a/n b/n\ndeleted file mode 100644\n--- a/n\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n",
        );
        assert!(gone.deleted && !gone.new_file);
        // Line numbers beyond 999 widen the gutter.
        assert_eq!(align("@@ -1200,1 +1200,1 @@\n a\n").gutter, 4);
        // A rename without edits has no rows to pair.
        let renamed =
            align("diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n");
        assert!(!renamed.has_lines());
    }

    #[test]
    fn marks_the_changed_part_of_a_pair() {
        let parts = |old: &str, new: &str| {
            let (a, b) = intra(old, new);
            let cut = |s: &str, r: Option<(usize, usize)>| {
                r.map(|(x, y)| s.chars().skip(x).take(y - x).collect::<String>())
            };
            (cut(old, a), cut(new, b))
        };
        let some = |a: &str, b: &str| (Some(a.to_string()), Some(b.to_string()));
        assert_eq!(parts("foo(a)", "foo(b, c)"), some("a", "b, c"));
        assert_eq!(parts("hello world", "hello there"), some("world", "there"));
        // Whole words, not letters.
        assert_eq!(
            parts("x = getName(1)", "x = getTitle(1)"),
            some("getName", "getTitle")
        );
        // Nothing in common or nothing different: no mark.
        assert_eq!(parts("a b", "x y"), (None, None));
        assert_eq!(parts("same", "same"), (None, None));
        assert_eq!(parts("getName", "getTitle"), (None, None));
        // A pure insertion marks only the new side.
        assert_eq!(parts("abc", "abc d"), (None, Some(" d".to_string())));
        // The pairs of a diff carry the marks; unpaired lines and context do not.
        let a =
            align("@@ -1,3 +1,2 @@\n let x = foo(a);\n-let y = foo(a);\n-gone\n+let y = foo(b);\n");
        let Row::Line { left, right, .. } = &a.rows[2] else {
            panic!()
        };
        assert_eq!(left.as_ref().unwrap().hl, Some((12, 13)));
        assert_eq!(right.as_ref().unwrap().hl, Some((12, 13)));
        let Row::Line { left, right, .. } = &a.rows[3] else {
            panic!()
        };
        assert_eq!((left.as_ref().unwrap().hl, right.is_none()), (None, true));
    }

    #[test]
    fn windows_by_cells() {
        let text = |s: &str, skip, avail| -> String {
            let spans = windowed(
                s.chars().map(|c| (c, 0)),
                width(s),
                skip,
                avail,
                &[Style::new(); 3],
                true,
            );
            let out: String = spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(width(&out), avail, "{s:?} {skip} {avail}");
            out
        };
        assert_eq!(text("hello", 0, 8), "hello   ");
        assert_eq!(text("hello world", 0, 8), "hello w…");
        assert_eq!(text("hello world", 6, 8), "world   ");
        assert_eq!(text("hello world", 3, 5), "lo w…");
        // A wide character cut by either edge leaves blanks, never half a character.
        assert_eq!(text("日本語です", 0, 5), "日本…");
        assert_eq!(text("日本語です", 1, 5), " 本… ");
        assert_eq!(text("日本語です", 2, 6), "本語… ");
        assert_eq!(text("日本語", 5, 4), "    ");
        assert_eq!(text("x", 4, 3), "   ");
        assert_eq!(text("abc", 0, 1), "…");
        assert!(
            windowed(
                "abc".chars().map(|c| (c, 0)),
                3,
                0,
                0,
                &[Style::new(); 3],
                true
            )
            .is_empty()
        );
    }

    #[test]
    fn draws_the_unified_view_when_narrow() {
        let v = DiffView::from_text("src/a.ts", SAMPLE);
        let mut t = Terminal::new(TestBackend::new(24, 8)).unwrap();
        let out = draw_to(&v, &mut t);
        let first = out.lines().next().unwrap();
        assert_eq!(first, "src/a.ts  +1 −2   1–7/10");
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
    fn unified_view_scrolls_sideways_with_the_mark_in_place() {
        let mut v = DiffView::from_text("f", "@@ -1 +1 @@\n+0123456789abcdef\n");
        let mut t = Terminal::new(TestBackend::new(10, 4)).unwrap();
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
        ch(&mut v, 'l');
        draw_to(&v, &mut t);
        ch(&mut v, 'l');
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+89abcdef");
        // Not past the end.
        ch(&mut v, 'l');
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+89abcdef");
        press(&mut v, KeyCode::Left);
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
        ch(&mut v, 'l');
        ch(&mut v, '0');
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
    }

    #[test]
    fn draws_the_split_view() {
        let v = DiffView::from_text("src/app.ts", BIG);
        let out = render(&v, 120, 30);
        snapshot("diff-split-120x30", &out);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("src/app.ts  +"));
        assert!(lines[0].ends_with("lines 1–23 of 23"));
        assert!(lines[1].starts_with(" HEAD"));
        assert!(lines[1].contains("│ working tree"));
        // The separator is in the same column on every row, hunk bands excepted.
        let sep = lines[1].chars().position(|c| c == '│').unwrap();
        for l in &lines[2..] {
            if !l.starts_with('⋯') {
                assert_eq!(l.chars().nth(sep), Some('│'), "{l}");
            }
        }
    }

    #[test]
    fn narrow_terminals_fall_back_to_the_unified_view() {
        let v = DiffView::from_text("src/app.ts", BIG);
        let out = render(&v, 60, 14);
        snapshot("diff-unified-60x14", &out);
        assert!(out.contains("diff --git"));
        assert!(!out.contains("HEAD"));
        // Just wide enough for the split.
        assert!(render(&v, 90, 6).contains("working tree"));
        assert!(!render(&v, 89, 6).contains("working tree"));
    }

    #[test]
    fn headers_name_both_sides() {
        let new =
            "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+x\n";
        let out = render(&DiffView::from_text("n", new), 100, 6);
        assert!(
            out.lines().nth(1).unwrap().starts_with(" (new file)"),
            "{out}"
        );
        assert!(out.contains("│ working tree"));
        let gone = "diff --git a/n b/n\ndeleted file mode 100644\n--- a/n\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
        let out = render(&DiffView::from_text("n", gone), 100, 6);
        assert!(out.lines().nth(1).unwrap().contains("│ (deleted)"), "{out}");
        let mut renamed = DiffView::from_text("old.rs → new.rs", "@@ -1 +1 @@\n-a\n+b\n");
        renamed.req = Some(DiffRequest {
            rel: "new.rs".into(),
            orig: Some("old.rs".into()),
            mark: Mark::Renamed,
            in_repo: true,
        });
        let out = render(&renamed, 100, 6);
        assert!(out.lines().next().unwrap().starts_with("old.rs → new.rs"));
        assert!(
            out.lines().nth(1).unwrap().starts_with(" HEAD · old.rs"),
            "{out}"
        );
        assert!(out.contains("│ working tree · new.rs"));
    }

    impl DiffView {
        fn wait_for_syntax(&mut self) {
            if let Some(rx) = self.syntax_rx.take() {
                self.syntax = rx.recv().unwrap();
            }
        }
    }

    /// The cells showing `text` on row `y`, by where it is found.
    fn cells_of(buf: &ratatui::buffer::Buffer, y: u16, text: &str) -> Vec<(Color, Color)> {
        let row: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        let at = row.find(text).expect("the text on the row");
        let x = row[..at].chars().count() as u16;
        (x..x + text.chars().count() as u16)
            .map(|x| (buf[(x, y)].fg, buf[(x, y)].bg))
            .collect()
    }

    #[test]
    fn code_is_colored_by_its_grammar_on_the_change_tint() {
        let diff = "--- a/src/m.rs\n+++ b/src/m.rs\n@@ -1,2 +1,2 @@\n fn keep() {}\n-let a = \"x\";\n+let a = \"y\";\n";
        let mut v = DiffView::from_text("src/m.rs", diff);
        v.wait_for_syntax();
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        draw_to(&v, &mut t);
        let buf = t.backend().buffer();
        let colors = code_colors(&Palette::default()).unwrap();
        let keyword = colors[Class::Keyword as usize].fg.unwrap();
        let string = colors[Class::String as usize].fg.unwrap();
        // Unified at this width: under the title, the headers and the hunk, the context line,
        // then the removal and the addition.
        assert!(cells_of(buf, 4, "fn").iter().all(|(fg, _)| *fg == keyword));
        let removed = cells_of(buf, 5, "let");
        assert!(
            removed
                .iter()
                .all(|(fg, bg)| *fg == keyword && *bg != Color::Reset)
        );
        assert!(
            cells_of(buf, 6, "\"y\"")
                .iter()
                .all(|(fg, _)| *fg == string)
        );
        // The sign keeps the change's color.
        assert_eq!(buf[(0, 6)].fg, Palette::default().success);
    }

    #[test]
    fn a_big_file_is_highlighted_quickly() {
        let mut text = String::from("--- a/big.rs\n+++ b/big.rs\n@@ -1,5000 +1,5000 @@\n");
        for i in 0..5000 {
            text.push_str(&format!(
                " fn f{i}(x: u32) -> u32 {{ x * {i} + \"s\".len() as u32 }} // {i}\n"
            ));
        }
        let started = std::time::Instant::now();
        let mut v = DiffView::from_text("big.rs", &text);
        // The text is there right away; the colors follow.
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(v.syntax.unified.is_empty());
        v.wait_for_syntax();
        let took = started.elapsed();
        assert_eq!(v.syntax.unified.len(), v.lines.len());
        eprintln!("5000 lines highlighted in {took:?}");
        assert!(took < std::time::Duration::from_secs(60), "{took:?}");
    }

    #[test]
    fn paints_changes_and_blank_halves() {
        let p = Palette::default();
        let v = DiffView::from_text(
            "f",
            "@@ -1,2 +1,3 @@\n same\n-a = foo(1)\n+a = foo(2)\n+extra\n",
        );
        let mut t = Terminal::new(TestBackend::new(100, 8)).unwrap();
        draw_to(&v, &mut t);
        let buf = t.backend().buffer();
        let (del_bg, add_bg) = (buf[(10, 4)].bg, buf[(60, 4)].bg);
        assert_ne!(del_bg, Color::Reset);
        assert_ne!(add_bg, Color::Reset);
        assert_eq!(buf[(10, 4)].fg, p.danger);
        assert_eq!(buf[(60, 4)].fg, p.success);
        assert_eq!(buf[(10, 3)].bg, Color::Reset); // context
        // The changed digit has the stronger mark, the rest of the line the row's.
        assert_ne!(buf[(6 + 8, 4)].bg, del_bg);
        assert_eq!(buf[(6 + 2, 4)].bg, del_bg);
        // The blank half is shaded, the line numbers are muted, signs say which side.
        assert_eq!(buf[(10, 5)].bg, p.surface);
        assert_eq!(buf[(2, 3)].fg, p.muted);
        assert_eq!((buf[(4, 4)].symbol(), buf[(54, 4)].symbol()), ("-", "+"));
        assert_eq!(buf[(4, 5)].symbol(), " ");
        // Without colors, the mark is reversed and nothing is painted.
        let plain = Palette {
            fg: Color::Reset,
            bg: Color::Reset,
            surface: Color::Reset,
            border: Color::Reset,
            accent: Color::Reset,
            muted: Color::Reset,
            success: Color::Reset,
            danger: Color::Reset,
        };
        t.draw(|f| v.draw(f, f.area(), &plain)).unwrap();
        let buf = t.backend().buffer();
        assert_eq!(buf[(10, 4)].bg, Color::Reset);
        assert!(buf[(6 + 8, 4)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(6 + 2, 4)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn scrolls_clamped_and_closes() {
        let text: String = std::iter::once("@@ -1 +1 @@\n".to_string())
            .chain((0..40).map(|i| format!("+line {i}\n")))
            .collect();
        let mut v = DiffView::from_text("f", &text);
        // Narrow, so unified: 10 rows of text under the title.
        let mut t = Terminal::new(TestBackend::new(30, 11)).unwrap();
        let line = |out: &str, n: usize| out.lines().nth(n).unwrap().to_string();
        draw_to(&v, &mut t); // learns the height
        assert!(!press(&mut v, KeyCode::Down));
        let out = draw_to(&v, &mut t);
        assert!(line(&out, 1).starts_with("+line 0"));
        assert!(line(&out, 0).ends_with("lines 2–11 of 41"));
        press(&mut v, KeyCode::Char('G'));
        let out = draw_to(&v, &mut t);
        assert!(out.lines().last().unwrap().starts_with("+line 39"));
        assert!(line(&out, 0).ends_with("lines 32–41 of 41"));
        press(&mut v, KeyCode::Down);
        assert_eq!(draw_to(&v, &mut t).lines().last().unwrap(), "+line 39");
        press(&mut v, KeyCode::PageUp);
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 30")
        );
        v.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 25")
        );
        v.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 30")
        );
        press(&mut v, KeyCode::PageDown);
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with("+line 39")
        );
        press(&mut v, KeyCode::Char('g'));
        assert!(line(&draw_to(&v, &mut t), 1).starts_with("@@"));
        ch(&mut v, 'j');
        ch(&mut v, 'j');
        ch(&mut v, 'k');
        assert!(line(&draw_to(&v, &mut t), 1).starts_with("+line 0"));
        assert!(press(&mut v, KeyCode::Esc));
        assert!(ch(&mut v, 'q'));
    }

    /// 40 numbered lines with a change after every 12th, as the split view shows them.
    fn spread() -> DiffView {
        let mut text = String::new();
        for hunk in 0..3 {
            let at = 1 + hunk * 20;
            text.push_str(&format!("@@ -{at},12 +{at},12 @@ section {hunk}\n"));
            for i in 0..5 {
                text.push_str(&format!(" ctx {hunk}.{i}\n"));
            }
            text.push_str(&format!("-old {hunk}\n+new {hunk}\n"));
            for i in 5..10 {
                text.push_str(&format!(" ctx {hunk}.{i}\n"));
            }
        }
        DiffView::from_text("f", &text)
    }

    #[test]
    fn jumps_between_hunks_and_pages_in_the_split_view() {
        let mut v = spread();
        let mut t = Terminal::new(TestBackend::new(120, 10)).unwrap(); // 8 rows
        let out = draw_to(&v, &mut t);
        assert!(v.view.get().split);
        assert_eq!(v.aligned.hunks, [0, 12, 24]);
        assert!(
            out.lines()
                .nth(2)
                .unwrap()
                .starts_with("⋯ @@ -1,12 +1,12 @@ section 0")
        );
        assert!(out.lines().next().unwrap().ends_with("lines 1–8 of 36"));
        ch(&mut v, 'n');
        assert_eq!(v.scroll.get(), 12);
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .nth(2)
                .unwrap()
                .contains("section 1")
        );
        ch(&mut v, 'n');
        assert_eq!(v.scroll.get(), 24);
        // The last hunk is further than the view can scroll: clamped, and n stays put.
        ch(&mut v, 'n');
        assert_eq!(v.scroll.get(), 24);
        ch(&mut v, 'p');
        assert_eq!(v.scroll.get(), 12);
        ch(&mut v, 'p');
        assert_eq!(v.scroll.get(), 0);
        ch(&mut v, 'p');
        assert_eq!(v.scroll.get(), 0);
        press(&mut v, KeyCode::PageDown);
        assert_eq!(v.scroll.get(), 7);
        press(&mut v, KeyCode::PageDown);
        press(&mut v, KeyCode::PageDown);
        press(&mut v, KeyCode::PageDown);
        press(&mut v, KeyCode::PageDown);
        // 36 rows, 8 shown.
        assert_eq!(v.scroll.get(), 28);
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .next()
                .unwrap()
                .ends_with("lines 29–36 of 36")
        );
        // The mouse wheel is Up / Down.
        press(&mut v, KeyCode::Up);
        press(&mut v, KeyCode::Up);
        assert_eq!(v.scroll.get(), 26);
        press(&mut v, KeyCode::End);
        assert_eq!(v.scroll.get(), 28);
        press(&mut v, KeyCode::Home);
        assert_eq!(v.scroll.get(), 0);
    }

    #[test]
    fn scrolls_both_sides_sideways() {
        let mut v = DiffView::from_text("src/app.ts", BIG);
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let out = draw_to(&v, &mut t);
        let at = |out: &str, needle: &str| out.lines().position(|l| l.contains(needle)).unwrap();
        let long = at(&out, "export const long");
        let cut = out.lines().nth(long).unwrap();
        assert_eq!(cut.matches('…').count(), 2, "{cut}");
        assert!(!cut.contains("sideways"));
        // Left / Right (and h / l) move both sides by 8 cells.
        press(&mut v, KeyCode::Right);
        ch(&mut v, 'l');
        let out = draw_to(&v, &mut t);
        let cut = out.lines().nth(long).unwrap();
        assert!(cut.contains("g = \"a very"), "{cut}");
        for _ in 0..20 {
            press(&mut v, KeyCode::Right);
        }
        let out = draw_to(&v, &mut t);
        let cut = out.lines().nth(long).unwrap();
        // The end of both lines is in view, and no `…` is left.
        assert_eq!(cut.matches("sideways").count(), 2, "{cut}");
        assert!(cut.trim_end().ends_with("sideways!\";"), "{cut}");
        assert_eq!(v.hscroll, v.max_hscroll());
        // The line numbers stay.
        assert!(out.lines().nth(2).unwrap().starts_with("⋯"));
        press(&mut v, KeyCode::Left);
        assert_eq!(v.hscroll, v.max_hscroll() - 8);
        ch(&mut v, '0');
        assert_eq!(v.hscroll, 0);
        for _ in 0..30 {
            press(&mut v, KeyCode::Left);
        }
        assert_eq!(v.hscroll, 0);
    }

    #[test]
    fn s_switches_between_split_and_unified() {
        let mut v = spread();
        let mut t = Terminal::new(TestBackend::new(120, 10)).unwrap();
        draw_to(&v, &mut t);
        ch(&mut v, 'n');
        ch(&mut v, 'n');
        assert!(v.view.get().split);
        ch(&mut v, 's');
        let out = draw_to(&v, &mut t);
        assert!(!v.view.get().split);
        assert!(!out.contains("working tree"));
        // The top row is the same hunk.
        assert!(
            out.lines()
                .nth(1)
                .unwrap()
                .starts_with("@@ -41,12 +41,12 @@ section 2"),
            "{out}"
        );
        ch(&mut v, 'p');
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("@@ -21,12")
        );
        ch(&mut v, 's');
        let out = draw_to(&v, &mut t);
        assert!(v.view.get().split);
        assert!(out.lines().nth(2).unwrap().contains("section 1"));
        // Too narrow for two columns whatever `s` says.
        let narrow = render(&v, 24, 8);
        assert!(!narrow.contains("working tree"));
    }

    #[test]
    fn no_row_is_wider_than_the_overlay() {
        let mut text = String::from("@@ -1,6 +1,6 @@ 日本語のヘッダー\n");
        text.push_str(" 日本語\tの文字列と\tlatin text that goes on and on and on and on\n");
        text.push_str("-\t\told\t日本語の長い行がここにありますそしてさらに続きますよ\n");
        text.push_str("+\t\tnew\t日本語の長い行がここにあります、そしてさらに続きますよ\n");
        text.push_str("\\ No newline at end of file\n");
        text.push_str("+extra 😀 emoji 😀 line with a combining e\u{301} accent and more text\n");
        text.push_str(&format!("-{}\n", "x".repeat(300)));
        text.push_str(" ok\n");
        let mut v = DiffView::from_text("日本語/パス/名前/とても/長い/ファイル名.rs", &text);
        let p = Palette::default();
        let look = Look::new(&p);
        for force in [None, Some(true), Some(false)] {
            v.force = force;
            for w in 1..=140usize {
                for h in [1u16, 3, 9] {
                    for hscroll in [0, 3, 21] {
                        v.hscroll = hscroll;
                        // The rows themselves.
                        match v.split_geom(w) {
                            Some(g) => {
                                let rows = v.split_lines(&g, 0, 12, hscroll, &look);
                                for l in rows {
                                    assert_eq!(l.width(), w, "split {w} {hscroll}: {l:?}");
                                }
                            }
                            None => {
                                for l in v.unified_lines(w, 0, 12, hscroll, &look) {
                                    assert!(l.width() <= w, "unified {w} {hscroll}: {l:?}");
                                }
                            }
                        }
                        // And through the terminal, with every key.
                        for key in ['n', 'l', 'j', 'G', 'p', 'h', 's'] {
                            ch(&mut v, key);
                            // (a fresh terminal each time: the test backend leaves half
                            // of a replaced wide character behind)
                            let out = render(&v, w as u16, h);
                            assert_eq!(out.split('\n').count(), h as usize);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_diff_says_so() {
        let v = DiffView::from_text("f", "");
        assert_eq!(v.lines()[0].text, "no changes");
        assert_eq!(v.counts(), (0, 0));
        // A note is shown in the unified view at any width.
        assert!(render(&v, 120, 4).contains("no changes"));
        assert!(!render(&v, 120, 4).contains("HEAD"));
        let failed = DiffView::from_text("f", "").lines().len();
        assert_eq!(failed, 1);
    }

    #[test]
    fn a_rename_without_edits_shows_git_header() {
        let v = DiffView::from_text(
            "x → y",
            "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n",
        );
        let out = render(&v, 120, 6);
        assert!(out.contains("rename from x"));
        assert!(!out.contains("working tree"));
    }

    #[test]
    fn big_diffs_are_cut() {
        let text: String = std::iter::once("@@ -1 +1 @@\n".to_string())
            .chain((0..MAX_LINES + 5).map(|i| format!("+l{i}\n")))
            .collect();
        let v = DiffView::from_text("f", &text);
        assert_eq!(v.lines().len(), MAX_LINES + 1);
        assert!(matches!(v.aligned.rows.last(), Some(Row::Note(n)) if n.contains("cut after")));
        assert!(render(&v, 120, 5).contains("lines 1–3 of"));
    }

    fn git_run(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
    }

    #[test]
    fn f_reloads_with_the_whole_file_and_keeps_the_place() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("git not found, skipping");
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("suite-term-diff-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        git_run(&dir, &["init", "-q"]);
        let lines = |mid: &str| -> String {
            (1..=60)
                .map(|i| {
                    if i == 30 {
                        format!("{mid}\n")
                    } else {
                        format!("line {i}\n")
                    }
                })
                .collect()
        };
        std::fs::write(dir.join("f.txt"), lines("thirty")).unwrap();
        git_run(&dir, &["add", "."]);
        git_run(
            &dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        );
        std::fs::write(dir.join("f.txt"), lines("THIRTY")).unwrap();
        let req = DiffRequest {
            rel: "f.txt".into(),
            orig: None,
            mark: Mark::Modified,
            in_repo: true,
        };
        let wait = |v: &mut DiffView| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !v.is_loaded() {
                assert!(std::time::Instant::now() < deadline, "timed out");
                v.poll();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let mut v = DiffView::open(dir.clone(), req);
        wait(&mut v);
        let mut t = Terminal::new(TestBackend::new(120, 20)).unwrap();
        let out = draw_to(&v, &mut t);
        // Five lines of context each way: 25 to 35.
        assert_eq!(v.aligned.rows.len(), 1 + 11);
        assert!(out.contains("@@ -25,11 +25,11 @@"), "{out}");
        assert!(out.contains("working tree"));
        ch(&mut v, 'f');
        assert!(!v.is_loaded());
        wait(&mut v);
        let out = draw_to(&v, &mut t);
        assert!(out.lines().next().unwrap().contains("full file"));
        assert!(matches!(&v.aligned.rows[0], Row::Hunk(h) if h == "@@ -1,60 +1,60 @@"));
        assert_eq!(v.aligned.rows.len(), 1 + 60);
        // The line at the top (25) is still at the top.
        let first = out.lines().nth(2).unwrap();
        assert!(first.contains("line 25"), "{out}");
        // And back.
        ch(&mut v, 'f');
        wait(&mut v);
        let out = draw_to(&v, &mut t);
        assert!(!out.lines().next().unwrap().contains("full file"));
        assert!(matches!(&v.aligned.rows[0], Row::Hunk(h) if h.starts_with("@@ -25,11")));
        assert!(
            out.lines().nth(2).unwrap().starts_with("⋯ @@ -25,11"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
