//! The diff overlay: `git diff` of one file, loaded in the background, drawn side by side (old
//! left, new right) when it is wide enough and as a unified diff otherwise, scrolled by keys.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use neo_renderer_ratatui::Palette;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthChar;

use super::files::Mark;
use super::git::git;
use super::lit::{Lazy, Step};
use super::text::{clean_line, truncate, truncate_start, width};
use neo_renderer_ratatui::syntax::{self, Class, SyntaxReference, code_colors, syntax_for};

/// More lines than this are dropped (a generated file diff can be hundreds of megabytes).
/// Highlighting doesn't mind: it only looks at what is on screen.
const MAX_LINES: usize = 1_000_000;
/// Lines of context around a change, and "the whole file" (`f`).
const CONTEXT: u32 = 5;
const FULL_CONTEXT: u32 = 99_999;
/// The overlay is split from this width on (`s` overrides); each side needs this much text.
const SPLIT_MIN: usize = 90;
const MIN_TEXT: usize = 8;
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
    /// Cells across, for the comment blocks.
    width: usize,
}

/// A comment on lines of the file, written in the view (`<leader>c`) and put in the prompt as
/// `@path:start-end text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment {
    /// The first and last line it is about: the new version's numbers, or the old one's when
    /// the lines are only there (removed lines).
    pub start: usize,
    pub end: usize,
    pub old: bool,
    pub text: String,
}

impl Comment {
    /// How the prompt has it: `@src/a.rs:12-14 text`, `@src/a.rs:12 text` for one line.
    pub(crate) fn mention(&self, rel: &str) -> String {
        let at = if self.start == self.end {
            format!("@{rel}:{}", self.start)
        } else {
            format!("@{rel}:{}-{}", self.start, self.end)
        };
        format!("{at} {}", self.text)
    }
}

/// A comment being written: its lines, the text so far, and the comment it replaces.
#[derive(Debug, Clone)]
struct Writing {
    start: usize,
    end: usize,
    old: bool,
    text: String,
    editing: Option<usize>,
}

/// `v` or `V`: where the selection started (a row of the shown mode and a char of it), and
/// whether it takes whole lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Visual {
    row: usize,
    col: usize,
    line: bool,
}

/// A key waiting for the next one: `g` (`gg`), `y` (`yy`), the leader (`\` or Space).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    G,
    Y,
    Leader,
}

/// What row's text shows of the cursor and the selection: chars `a..b` selected, the char the
/// cursor is on (past the end: a blank after it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Marks {
    sel: Option<(usize, usize)>,
    cursor: Option<usize>,
}

/// The comment blocks drawn above the rows, by row, for one mode, width and state of the
/// comments.
#[derive(Debug, Default)]
struct Blocks {
    key: Option<(bool, usize, u64)>,
    at: HashMap<usize, Vec<Vec<Line<'static>>>>,
}

/// A comment as a block across the view: `╭ @path:12-14`, its text wrapped under a `│`, and a
/// `╰` with the keys while it is written.
fn block(
    head: &str,
    text: &str,
    cells: usize,
    writing: bool,
    colors: (Color, Color),
) -> Vec<Line<'static>> {
    let (accent, muted) = colors;
    let edge = Style::new().fg(accent);
    let inner = cells.saturating_sub(2).max(1);
    let mut lines = vec![Line::from(vec![
        Span::styled("╭ ", edge),
        Span::styled(truncate(head, inner), edge.add_modifier(Modifier::BOLD)),
    ])];
    let mut body = super::text::wrap(text, inner);
    if writing {
        // The cursor after the text, on a line of its own when the last one is full.
        let full = body.last().is_some_and(|l| width(l) >= inner);
        if body.is_empty() || full {
            body.push(String::new());
        }
    }
    if body.is_empty() {
        body.push(String::new());
    }
    let n = body.len();
    for (i, row) in body.into_iter().enumerate() {
        let mut spans = vec![Span::styled("│ ", edge), Span::raw(row)];
        if writing && i + 1 == n {
            spans.push(Span::styled(
                " ",
                Style::new().add_modifier(Modifier::REVERSED),
            ));
        }
        lines.push(Line::from(spans));
    }
    let foot = if writing {
        "╰ enter keeps it · esc gives up · empty deletes it"
    } else {
        "╰"
    };
    lines.push(Line::from(Span::styled(
        truncate(foot, cells.max(1)),
        Style::new().fg(muted),
    )));
    lines
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
    /// The line number of each unified line (the new file's; a removed line's in the old
    /// one); None for headers and notes.
    numbers: Vec<Option<usize>>,
    /// The split view.
    aligned: Aligned,
    /// The widest unified line, in cells.
    uni_w: usize,
    /// What each char of the code is, for colors, per unified line and per split row: worked
    /// out on a thread for the rows on screen (plain until it arrives). None: no grammar.
    lit_uni: Option<Lazy>,
    lit_split: Option<Lazy>,
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
    /// Just loaded whole: the next draw scrolls to the first change.
    to_change: Cell<bool>,
    view: Cell<View>,
    /// Digits typed before a key, vim's count (`5j`, `120G`).
    count: Option<usize>,
    /// `/` is being typed: the pattern so far.
    typing: Option<String>,
    /// The last search, for `n` / `N`.
    pattern: Option<String>,
    /// Said in the title line until the next key (`no match`).
    message: Option<String>,
    /// `<leader>e` asked for the file in the editor, at this line.
    edit: Option<(PathBuf, Option<usize>)>,
    /// `y` copied this, for the app to put on the clipboard.
    copy: Option<String>,
    /// Long lines wrap at the edge (`<leader>w` switches to cutting them and scrolling sideways).
    wrap: bool,
    /// The cursor: a row of the mode `scroll_split` says, and a char of the row's text.
    cursor: Cell<usize>,
    col: usize,
    /// The column up and down go back to (vim's), `usize::MAX` after `$`.
    want: usize,
    visual: Cell<Option<Visual>>,
    pending: Option<Pending>,
    /// The comments on this file, in the order written.
    comments: Vec<Comment>,
    writing: Option<Writing>,
    /// What the comments changed since the app last asked: each as the prompt had it and has
    /// it now (none: not there).
    changes: Vec<(Option<String>, Option<String>)>,
    /// Bumped when the comments or the one being written change.
    comments_gen: u64,
    blocks: RefCell<Blocks>,
    /// The accent and muted colors of the last draw, for the comment blocks.
    colors: Cell<(Color, Color)>,
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
    /// `/diff`'s choice to start with: split where it fits (the width decides), or always
    /// unified. `s` still switches.
    pub(crate) fn with_split(mut self, split: bool) -> DiffView {
        self.force = (!split).then_some(false);
        self
    }

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
            numbers: Vec::new(),
            aligned: Aligned::default(),
            uni_w: 0,
            lit_uni: None,
            lit_split: None,
            adds: 0,
            dels: 0,
            loaded: false,
            scroll: Cell::new(0),
            scroll_split: Cell::new(false),
            hscroll: 0,
            force: None,
            anchor: None,
            to_change: Cell::new(false),
            view: Cell::new(View {
                rows: 20,
                col_w: 40,
                split: false,
                width: 80,
            }),
            count: None,
            typing: None,
            pattern: None,
            message: None,
            edit: None,
            copy: None,
            wrap: true,
            cursor: Cell::new(0),
            col: 0,
            want: 0,
            visual: Cell::new(None),
            pending: None,
            comments: Vec::new(),
            writing: None,
            changes: Vec::new(),
            comments_gen: 0,
            blocks: RefCell::new(Blocks::default()),
            colors: Cell::new((Color::Reset, Color::Reset)),
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
        let first = !self.loaded;
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
        let path = self.req.as_ref().map(|r| r.rel.as_str());
        let (grammar, uni, split) = steps(path, &lines, &self.aligned);
        self.lit_uni = Lazy::start(grammar, uni);
        self.lit_split = Lazy::start(grammar, split);
        self.numbers = line_numbers(&lines);
        self.lines = lines;
        self.adds = adds;
        self.dels = dels;
        self.loaded = true;
        self.rx = None;
        self.comments_gen += 1;
        self.visual.set(None);
        self.cursor.set(
            self.cursor
                .get()
                .min(self.len(self.scroll_split.get()).saturating_sub(1)),
        );
        if let Some(line) = self.anchor.take() {
            let at = self
                .aligned
                .rows
                .iter()
                .position(|r| matches!(r, Row::Line { right: Some(s), .. } if s.no >= line));
            self.set_position(at.unwrap_or(0));
            let split = self.scroll_split.get();
            let row = at.unwrap_or(0);
            let row = if split {
                row
            } else {
                self.convert_row(row, true)
            };
            self.cursor.set(row);
            self.col = self.col.min(self.last_col(row, split));
        } else if first && self.full && self.req.is_some() {
            // A file opened whole starts at its first change (the draw knows the mode).
            self.to_change.set(true);
        }
    }

    /// The first changed row in the mode's rows, a few lines above it.
    fn change_position(&self, split: bool) -> Option<usize> {
        self.first_change(split)
            .map(|at| at.saturating_sub(CONTEXT as usize))
    }

    /// The first changed row in the mode's rows.
    fn first_change(&self, split: bool) -> Option<usize> {
        if split {
            self.aligned
                .rows
                .iter()
                .position(|r| matches!(r, Row::Line { changed: true, .. }))
        } else {
            self.lines
                .iter()
                .position(|l| matches!(l.kind, Kind::Add | Kind::Del))
        }
    }

    /// Picks up the loaded diff, or its highlighting. True when something arrived.
    pub(crate) fn poll(&mut self) -> bool {
        let mut lit = false;
        for lazy in [&mut self.lit_uni, &mut self.lit_split]
            .into_iter()
            .flatten()
        {
            lit |= lazy.poll();
        }
        if lit {
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

    /// The last top row: the one from which the rest just fills the screen (wrapped rows
    /// counted as the lines they take).
    fn max_scroll(&self) -> usize {
        let v = self.view.get();
        let len = self.len(v.split);
        if !self.wrap && self.comments.is_empty() && self.writing.is_none() {
            return len.saturating_sub(v.rows.max(1));
        }
        let (mut top, mut used) = (len, 0);
        while top > 0 {
            used += self.height(top - 1, v.split);
            if used > v.rows.max(1) {
                break;
            }
            top -= 1;
        }
        top.min(len.saturating_sub(1))
    }

    fn max_hscroll(&self) -> usize {
        let v = self.view.get();
        if self.wrap {
            return 0;
        }
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
        let split = self.view.get().split;
        self.anchor = self
            .row_nos(self.cursor.get(), split)
            .1
            .or_else(|| self.top_new_line());
        self.rx = Some(spawn(self.cwd.clone(), req, self.full));
        self.loaded = false;
    }

    /// Moves the cursor, selects, comments, searches, asks for the editor; true when the key
    /// closes the overlay (Esc outside visual mode, q). Vim's keys: h/j/k/l, w/b/e, 0/^/$,
    /// gg/G, H/M/L; a count before a key repeats it (`5j`) or names a line (`120G`); Ctrl-D/U
    /// half a page, Ctrl-F/B and PgUp/PgDn a page, Ctrl-E/Y scroll a line; `/` searches (smart
    /// case), `n` / `N` go to the next / previous match (without a search, `n` / `p` go between
    /// hunks, as `}` / `{` always do); `v` and `V` select characters and lines. The leader (`\`
    /// or Space) then `c` comments on the line or the selection (on a commented line: edits the
    /// comment), `w` wraps or cuts long lines, `e` opens the file in the editor at the cursor.
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        self.message = None;
        if self.writing.is_some() {
            self.write_key(key);
            return false;
        }
        if self.typing.is_some() {
            self.search_key(key);
            return false;
        }
        let v = self.view.get();
        let split = v.split;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(pending) = self.pending.take() {
            match (pending, key.code) {
                (Pending::G, KeyCode::Char('g')) => {
                    self.cursor.set(0);
                    self.col = 0;
                    self.want = 0;
                }
                (Pending::Y, KeyCode::Char('y')) => {
                    let cur = self.cursor.get();
                    let n = self.count.unwrap_or(1);
                    let last = self.len(split).saturating_sub(1);
                    self.yank(cur, (cur + n - 1).min(last), None, split);
                }
                (Pending::Leader, KeyCode::Char('c')) => self.start_comment(),
                (Pending::Leader, KeyCode::Char('w')) => {
                    self.wrap = !self.wrap;
                    self.hscroll = 0;
                }
                (Pending::Leader, KeyCode::Char('e')) => match &self.req {
                    Some(req) => {
                        let line = self.row_nos(self.cursor.get(), split).1;
                        let line = line.or_else(|| self.top_new_line());
                        self.edit = Some((self.cwd.join(&req.rel), line));
                    }
                    None => self.message = Some("not a file here".into()),
                },
                _ => {}
            }
            self.count = None;
            self.follow();
            return false;
        }
        if let (KeyCode::Char(c @ '0'..='9'), false) = (key.code, ctrl)
            && (c != '0' || self.count.is_some())
        {
            let digit = c.to_digit(10).unwrap_or(0) as usize;
            self.count = Some(self.count.unwrap_or(0).saturating_mul(10) + digit);
            return false;
        }
        let count = self.count.take();
        let n = count.unwrap_or(1);
        let rows = v.rows.max(1);
        let page = rows.saturating_sub(1).max(1);
        let half = (rows / 2).max(1);
        let len = self.len(split);
        let last = len.saturating_sub(1);
        let cur = self.cursor.get().min(last);
        // Moves the cursor (and the view with it, by the same rows) up or down.
        let by = |this: &mut Self, rows: isize, scroll: bool| {
            let to = cur.saturating_add_signed(rows).min(last);
            this.set_row(to, split);
            if scroll {
                let s = this.scroll.get().saturating_add_signed(rows);
                this.scroll.set(s.min(this.max_scroll()));
            }
        };
        let signed = |x: usize| isize::try_from(x).unwrap_or(isize::MAX);
        match (key.code, ctrl) {
            (KeyCode::Esc, _) if self.visual.get().is_some() => self.visual.set(None),
            (KeyCode::Esc, _) | (KeyCode::Char('q'), false) => return true,
            (KeyCode::Up | KeyCode::Char('k'), false) => by(self, -signed(n), false),
            (KeyCode::Down | KeyCode::Char('j'), false) => by(self, signed(n), false),
            (KeyCode::Char('y'), true) => self.scroll_only(-signed(n)),
            (KeyCode::Char('e'), true) => self.scroll_only(signed(n)),
            (KeyCode::PageUp, _) | (KeyCode::Char('b'), true) => by(self, -signed(page * n), true),
            (KeyCode::PageDown, _) | (KeyCode::Char('f'), true) => {
                by(self, signed(page * n), true);
            }
            (KeyCode::Char('u'), true) => by(self, -signed(half * n), true),
            (KeyCode::Char('d'), true) => by(self, signed(half * n), true),
            (KeyCode::Home, _) => {
                self.set_row(0, split);
                self.scroll.set(0);
            }
            (KeyCode::Char('g'), false) => self.pending = Some(Pending::G),
            (KeyCode::Char('y'), false) => match self.visual.take() {
                Some(v) => {
                    let here = (cur, self.col.min(self.last_col(cur, split)));
                    let (a, b) = if (v.row, v.col) <= here {
                        ((v.row, v.col), here)
                    } else {
                        (here, (v.row, v.col))
                    };
                    let cols = (!v.line).then_some((a.1, b.1));
                    self.yank(a.0, b.0, cols, split);
                    // As vim: the cursor goes to the start of what was copied.
                    self.cursor.set(a.0);
                    self.col = if v.line {
                        self.col.min(self.last_col(a.0, split))
                    } else {
                        a.1
                    };
                }
                None => {
                    self.count = count;
                    self.pending = Some(Pending::Y);
                }
            },
            (KeyCode::End | KeyCode::Char('G'), _) => {
                let row = count.map_or(last, |line| self.row_of_line(line, split));
                self.set_row(row, split);
            }
            (KeyCode::Char('H'), false) => self.set_row(self.scroll.get().min(last), split),
            (KeyCode::Char('L'), false) => self.set_row(self.last_visible(), split),
            (KeyCode::Char('M'), false) => {
                let (top, bottom) = (self.scroll.get().min(last), self.last_visible());
                self.set_row(top + (bottom - top) / 2, split);
            }
            (KeyCode::Left | KeyCode::Char('h'), _) => {
                self.col = self.col.min(self.last_col(cur, split)).saturating_sub(n);
                self.want = self.col;
            }
            (KeyCode::Right | KeyCode::Char('l'), _) => {
                self.col = (self.col + n).min(self.last_col(cur, split));
                self.want = self.col;
            }
            (KeyCode::Char('0'), false) => (self.col, self.want) = (0, 0),
            (KeyCode::Char('^'), false) => {
                let body = self.row_body(cur, split);
                self.col = body.chars().take_while(|c| c.is_whitespace()).count();
                self.col = self.col.min(self.last_col(cur, split));
                self.want = self.col;
            }
            (KeyCode::Char('$'), false) => {
                self.col = self.last_col(cur, split);
                self.want = usize::MAX;
            }
            (KeyCode::Char(c @ ('w' | 'b' | 'e')), false) => {
                for _ in 0..n {
                    self.word(c, split);
                }
                self.want = self.col;
            }
            (KeyCode::Char('n'), false) if self.pattern.is_some() => {
                let at = self.find(cur, true, n);
                self.jump(at, split);
            }
            (KeyCode::Char('N'), false) => {
                let at = self.find(cur, false, n);
                self.jump(at, split);
            }
            (KeyCode::Char('n' | '}'), false) => {
                let after = self.hunk_starts(split).into_iter().filter(|&h| h > cur);
                if let Some(h) = after.take(n).last() {
                    self.set_row(h, split);
                    self.scroll.set(h.min(self.max_scroll()));
                }
            }
            (KeyCode::Char('p' | '{'), false) => {
                let mut before: Vec<usize> = self
                    .hunk_starts(split)
                    .into_iter()
                    .filter(|&h| h < cur)
                    .collect();
                before.reverse();
                let h = before.get(n - 1).copied().unwrap_or(0);
                self.set_row(h, split);
                self.scroll.set(h.min(self.max_scroll()));
            }
            (KeyCode::Char(c @ ('v' | 'V')), false) => {
                let line = c == 'V';
                self.visual.set(match self.visual.get() {
                    Some(v) if v.line == line => None,
                    Some(v) => Some(Visual { line, ..v }),
                    None => Some(Visual {
                        row: cur,
                        col: self.col.min(self.last_col(cur, split)),
                        line,
                    }),
                });
            }
            (KeyCode::Char(' ' | '\\'), false) => self.pending = Some(Pending::Leader),
            (KeyCode::Char('/'), false) => self.typing = Some(String::new()),
            (KeyCode::Char('s'), false) => self.force = Some(!split),
            (KeyCode::Char('f'), false) => self.toggle_full(),
            _ => {}
        }
        self.follow();
        false
    }

    /// Puts the cursor on `row`, at the column it keeps going back to.
    fn set_row(&mut self, row: usize, split: bool) {
        let row = row.min(self.len(split).saturating_sub(1));
        self.cursor.set(row);
        self.col = self.want.min(self.last_col(row, split));
    }

    /// The cursor on row `at` (a search's match), at the match.
    fn jump(&mut self, at: usize, split: bool) {
        self.set_row(at, split);
        if let Some(pattern) = &self.pattern {
            let body = self.row_body(at, split);
            let exact = pattern.chars().any(char::is_uppercase);
            let hay = if exact {
                body.clone()
            } else {
                body.to_lowercase()
            };
            let needle = if exact {
                pattern.clone()
            } else {
                pattern.to_lowercase()
            };
            if let Some(byte) = hay.find(&needle) {
                self.col = hay[..byte].chars().count().min(self.last_col(at, split));
                self.want = self.col;
            }
        }
        self.scroll.set(at.min(self.max_scroll()));
    }

    /// Ctrl-E / Ctrl-Y: the view moves, the cursor only as far as it has to stay in it.
    fn scroll_only(&mut self, rows: isize) {
        let s = self.scroll.get().min(self.max_scroll());
        self.scroll
            .set(s.saturating_add_signed(rows).min(self.max_scroll()));
        let split = self.view.get().split;
        let (top, bottom) = (self.scroll.get(), self.last_visible());
        let cur = self.cursor.get();
        if cur < top {
            self.set_row(top, split);
        } else if cur > bottom {
            self.set_row(bottom, split);
        }
    }

    /// The last row the view shows whole from its top row.
    fn last_visible(&self) -> usize {
        let v = self.view.get();
        let len = self.len(v.split);
        let top = self.scroll.get().min(len.saturating_sub(1));
        let mut used = 0;
        let mut at = top;
        for i in top..len {
            used += self.height(i, v.split);
            if used > v.rows.max(1) {
                break;
            }
            at = i;
        }
        at
    }

    /// Scrolls so the cursor is in view, its block of comments too; sideways to its column when
    /// long lines are cut.
    fn follow(&mut self) {
        let v = self.view.get();
        let cur = self.cursor.get();
        let s = self.scroll.get().min(self.max_scroll());
        self.scroll.set(if cur < s {
            cur
        } else {
            s.max(self.top_for(cur, v.split, v.rows))
        });
        if !self.wrap {
            let body = self.row_body(cur, v.split);
            let cell: usize = body
                .chars()
                .take(self.col)
                .map(|c| c.width().unwrap_or(0))
                .sum();
            let room = v.col_w.saturating_sub(2).max(1);
            if cell < self.hscroll {
                self.hscroll = cell;
            } else if cell >= self.hscroll + room {
                self.hscroll = cell + 1 - room;
            }
        }
    }

    /// The highest top row that still shows row `cur` whole (as far up as `rows` lines reach).
    fn top_for(&self, cur: usize, split: bool, rows: usize) -> usize {
        let mut used = 0;
        let mut top = cur;
        loop {
            used += self.height(top, split);
            if used > rows.max(1) {
                return (top + 1).min(cur);
            }
            if top == 0 {
                return 0;
            }
            top -= 1;
        }
    }

    /// The old and new line number of row `i` of a mode (a context line in the unified view
    /// only knows its new one).
    fn row_nos(&self, i: usize, split: bool) -> (Option<usize>, Option<usize>) {
        if split {
            return match self.aligned.rows.get(i) {
                Some(Row::Line { left, right, .. }) => {
                    (left.as_ref().map(|s| s.no), right.as_ref().map(|s| s.no))
                }
                _ => (None, None),
            };
        }
        let no = self.numbers.get(i).copied().flatten();
        match self.lines.get(i).map(|l| l.kind) {
            Some(Kind::Del) => (no, None),
            Some(Kind::Add | Kind::Context) => (None, no),
            _ => (None, None),
        }
    }

    /// The text the cursor moves in on row `i`: the line without its sign; in the split view
    /// the new side's when it has one, else the old one's.
    fn row_body(&self, i: usize, split: bool) -> String {
        if split {
            return match self.aligned.rows.get(i) {
                Some(Row::Line { left, right, .. }) => right
                    .as_ref()
                    .or(left.as_ref())
                    .map_or_else(String::new, |s| s.text.clone()),
                Some(Row::Hunk(t) | Row::Note(t)) => t.clone(),
                None => String::new(),
            };
        }
        match self.lines.get(i) {
            Some(l) if matches!(l.kind, Kind::Add | Kind::Del | Kind::Context) => {
                l.text.chars().skip(1).collect()
            }
            Some(l) => l.text.clone(),
            None => String::new(),
        }
    }

    /// The last column the cursor can be on in row `i` (0 for an empty one).
    fn last_col(&self, i: usize, split: bool) -> usize {
        self.row_body(i, split).chars().count().saturating_sub(1)
    }

    /// Row `row` of one mode in the other: the row of the same line, else the start of its
    /// hunk.
    fn convert_row(&self, row: usize, from_split: bool) -> usize {
        let (old, new) = self.row_nos(row, from_split);
        let to = !from_split;
        let same = (0..self.len(to)).find(|&j| {
            let (o, n) = self.row_nos(j, to);
            match new {
                Some(new) => n == Some(new),
                None => old.is_some() && n.is_none() && o == old,
            }
        });
        same.unwrap_or_else(|| self.convert(row, from_split))
    }

    /// `w`, `b` or `e` once, from the cursor: to the start of the next word, the start of this
    /// or the last one, the end of this or the next one; across lines as vim goes.
    fn word(&mut self, motion: char, split: bool) {
        let class = |c: char| {
            if c.is_whitespace() {
                0
            } else if c.is_alphanumeric() || c == '_' {
                1
            } else {
                2
            }
        };
        let len = self.len(split);
        let chars = |row: usize| self.row_body(row, split).chars().collect::<Vec<char>>();
        let (mut row, mut col) = (self.cursor.get(), self.col);
        let mut line = chars(row);
        match motion {
            'w' => {
                let start = line.get(col).map(|&c| class(c));
                if let Some(k) = start.filter(|&k| k != 0) {
                    while line.get(col).is_some_and(|&c| class(c) == k) {
                        col += 1;
                    }
                }
                loop {
                    while line.get(col).is_some_and(|&c| class(c) == 0) {
                        col += 1;
                    }
                    if col < line.len() || row + 1 >= len {
                        break;
                    }
                    row += 1;
                    col = 0;
                    line = chars(row);
                    // An empty line is a word of its own.
                    if line.is_empty() {
                        break;
                    }
                }
            }
            'e' => {
                col += 1;
                loop {
                    while line.get(col).is_some_and(|&c| class(c) == 0) {
                        col += 1;
                    }
                    if col < line.len() || row + 1 >= len {
                        break;
                    }
                    row += 1;
                    col = 0;
                    line = chars(row);
                }
                if let Some(k) = line.get(col).map(|&c| class(c)) {
                    while line.get(col + 1).is_some_and(|&c| class(c) == k) {
                        col += 1;
                    }
                }
            }
            _ => loop {
                if col == 0 {
                    if row == 0 {
                        break;
                    }
                    row -= 1;
                    line = chars(row);
                    col = line.len();
                    if line.is_empty() {
                        break;
                    }
                    continue;
                }
                col -= 1;
                if class(line[col]) == 0 {
                    continue;
                }
                let k = class(line[col]);
                while col > 0 && class(line[col - 1]) == k {
                    col -= 1;
                }
                break;
            },
        }
        self.cursor.set(row);
        self.col = col.min(line.len().saturating_sub(1));
    }

    /// `<leader>c`: writes a comment on the selection's lines, or the cursor's; on a line a
    /// comment is about (no selection), edits that one.
    fn start_comment(&mut self) {
        if self.req.is_none() {
            self.message = Some("no file here to comment on".into());
            return;
        }
        let split = self.view.get().split;
        let cur = self.cursor.get();
        let (a, b) = match self.visual.take() {
            Some(v) => (v.row.min(cur), v.row.max(cur)),
            None => {
                let (old, new) = self.row_nos(cur, split);
                let on = |c: &Comment| {
                    let no = if c.old {
                        old.filter(|_| new.is_none())
                    } else {
                        new
                    };
                    no.is_some_and(|no| (c.start..=c.end).contains(&no))
                };
                if let Some(i) = self.comments.iter().position(on) {
                    let c = &self.comments[i];
                    self.writing = Some(Writing {
                        start: c.start,
                        end: c.end,
                        old: c.old,
                        text: c.text.clone(),
                        editing: Some(i),
                    });
                    self.comments_gen += 1;
                    return;
                }
                (cur, cur)
            }
        };
        let (mut news, mut olds) = (Vec::new(), Vec::new());
        for row in a..=b {
            match self.row_nos(row, split) {
                (_, Some(n)) => news.push(n),
                (Some(o), None) => olds.push(o),
                _ => {}
            }
        }
        let (nos, old) = if news.is_empty() {
            (olds, true)
        } else {
            (news, false)
        };
        let (Some(&start), Some(&end)) = (nos.iter().min(), nos.iter().max()) else {
            self.message = Some("no line here to comment on".into());
            return;
        };
        self.writing = Some(Writing {
            start,
            end,
            old,
            text: String::new(),
            editing: None,
        });
        self.comments_gen += 1;
    }

    /// A key while a comment is written: Enter keeps it (empty: deletes it), Esc gives up.
    fn write_key(&mut self, key: KeyEvent) {
        let Some(w) = self.writing.as_mut() else {
            return;
        };
        self.comments_gen += 1;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.writing = None,
            KeyCode::Backspace => {
                w.text.pop();
            }
            KeyCode::Char('u') if ctrl => w.text.clear(),
            KeyCode::Char('w') if ctrl => {
                let kept = w.text.trim_end().len();
                let start = w.text[..kept]
                    .rfind(char::is_whitespace)
                    .map_or(0, |at| at + 1);
                w.text.truncate(start);
            }
            KeyCode::Char(c) if !ctrl => w.text.push(c),
            KeyCode::Enter => {
                let Some(w) = self.writing.take() else {
                    return;
                };
                let rel = self.req.as_ref().map_or("", |r| r.rel.as_str()).to_string();
                let text = w.text.split_whitespace().collect::<Vec<_>>().join(" ");
                let before = w.editing.and_then(|i| self.comments.get(i).cloned());
                let after = (!text.is_empty()).then_some(Comment {
                    start: w.start,
                    end: w.end,
                    old: w.old,
                    text,
                });
                match (w.editing, &after) {
                    (Some(i), Some(c)) => self.comments[i] = c.clone(),
                    (Some(i), None) => {
                        self.comments.remove(i);
                    }
                    (None, Some(c)) => self.comments.push(c.clone()),
                    (None, None) => return,
                }
                let mention = |c: &Comment| c.mention(&rel);
                self.changes
                    .push((before.as_ref().map(mention), after.as_ref().map(mention)));
            }
            _ => {}
        }
    }

    /// Copies rows `a..=b` as the cursor reads them (without the signs): whole, or from char
    /// `cols.0` of the first to char `cols.1` of the last.
    fn yank(&mut self, a: usize, b: usize, cols: Option<(usize, usize)>, split: bool) {
        let lines: Vec<String> = (a..=b)
            .map(|row| {
                let body: Vec<char> = self.row_body(row, split).chars().collect();
                let from = cols.filter(|_| row == a).map_or(0, |c| c.0).min(body.len());
                let to = cols
                    .filter(|_| row == b)
                    .map_or(body.len(), |c| c.1 + 1)
                    .clamp(from, body.len());
                body[from..to].iter().collect()
            })
            .collect();
        self.copy = Some(lines.join("\n"));
    }

    /// What `y` copied, once.
    pub(crate) fn take_copy(&mut self) -> Option<String> {
        self.copy.take()
    }

    /// What the comments changed since the last call: each as the prompt had it and has it now.
    pub(crate) fn take_comment_changes(&mut self) -> Vec<(Option<String>, Option<String>)> {
        std::mem::take(&mut self.changes)
    }

    /// The comments, to keep while the view is closed.
    pub(crate) fn take_comments(&mut self) -> Vec<Comment> {
        std::mem::take(&mut self.comments)
    }

    /// The comments written on this file before.
    pub(crate) fn with_comments(mut self, comments: Vec<Comment>) -> DiffView {
        self.comments = comments;
        self.comments_gen += 1;
        self
    }

    /// The path the view shows, as the prompt names it.
    pub(crate) fn rel(&self) -> Option<&str> {
        self.req.as_ref().map(|r| r.rel.as_str())
    }

    /// The cursor and selection on row `i`, for drawing.
    fn marks(&self, i: usize, split: bool) -> Marks {
        if self.writing.is_some() {
            return Marks::default();
        }
        let cur = self.cursor.get();
        let len = self.row_body(i, split).chars().count();
        let mut m = Marks::default();
        if i == cur {
            m.cursor = Some(self.col.min(len.saturating_sub(1)));
        }
        if let Some(v) = self.visual.get() {
            let here = (cur, self.col.min(self.last_col(cur, split)));
            let (a, b) = if (v.row, v.col) <= here {
                ((v.row, v.col), here)
            } else {
                (here, (v.row, v.col))
            };
            if (a.0..=b.0).contains(&i) {
                let whole = len.max(1);
                m.sel = Some(if v.line {
                    (0, whole)
                } else {
                    let from = if i == a.0 { a.1 } else { 0 };
                    let to = if i == b.0 { b.1 + 1 } else { whole };
                    (from, to.max(from + 1))
                });
            }
        }
        m
    }

    /// The comment blocks above row `i` of a mode, at the view's width.
    fn blocks_at(&self, i: usize, split: bool) -> Vec<Vec<Line<'static>>> {
        let width = self.view.get().width;
        let key = (split, width, self.comments_gen);
        let colors = self.colors.get();
        let mut cache = self.blocks.borrow_mut();
        if cache.key != Some(key) {
            cache.key = Some(key);
            cache.at.clear();
            let rel = self.rel().unwrap_or("");
            let editing = self.writing.as_ref().and_then(|w| w.editing);
            let mut put = |start: usize, old: bool, block: Vec<Line<'static>>| {
                let row = (0..self.len(split)).find(|&r| {
                    let (o, n) = self.row_nos(r, split);
                    if old {
                        n.is_none() && o == Some(start)
                    } else {
                        n == Some(start)
                    }
                });
                if let Some(row) = row {
                    cache.at.entry(row).or_default().push(block);
                }
            };
            for (i, c) in self.comments.iter().enumerate() {
                if Some(i) != editing {
                    let head = c.mention(rel);
                    let head = head.split_once(' ').map_or(head.as_str(), |(at, _)| at);
                    put(c.start, c.old, block(head, &c.text, width, false, colors));
                }
            }
            if let Some(w) = &self.writing {
                let c = Comment {
                    start: w.start,
                    end: w.end,
                    old: w.old,
                    text: String::new(),
                };
                let head = c.mention(rel);
                put(
                    w.start,
                    w.old,
                    block(head.trim_end(), &w.text, width, true, colors),
                );
            }
        }
        cache.at.get(&i).cloned().unwrap_or_default()
    }

    /// A key while `/` is typed: Enter searches, Esc gives up.
    fn search_key(&mut self, key: KeyEvent) {
        let Some(text) = self.typing.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.typing = None,
            KeyCode::Backspace if text.is_empty() => self.typing = None,
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                let text = self.typing.take().unwrap_or_default();
                if !text.is_empty() {
                    self.pattern = Some(text);
                }
                let split = self.view.get().split;
                let at = self.find(self.cursor.get(), true, 1);
                if self.message.is_none() {
                    self.jump(at, split);
                    self.follow();
                }
            }
            _ => {}
        }
    }

    /// The row of the `n`-th match of the search after (or before) row `from`, wrapping around;
    /// `from` itself when there is none (and the title says so).
    fn find(&mut self, from: usize, forward: bool, n: usize) -> usize {
        let Some(pattern) = self.pattern.clone() else {
            return from;
        };
        // Smart case: an upper-case letter makes it exact.
        let exact = pattern.chars().any(char::is_uppercase);
        let needle = if exact {
            pattern.clone()
        } else {
            pattern.to_lowercase()
        };
        let split = self.view.get().split;
        let len = self.len(split);
        let hit = |i: usize| {
            let text = self.row_text(i, split);
            if exact {
                text.contains(&needle)
            } else {
                text.to_lowercase().contains(&needle)
            }
        };
        let mut at = from;
        for _ in 0..n {
            let order: Box<dyn Iterator<Item = usize>> = if forward {
                Box::new((at + 1..len).chain(0..=at.min(len.saturating_sub(1))))
            } else {
                Box::new((0..at).rev().chain((at..len).rev()))
            };
            match order.into_iter().find(|&i| hit(i)) {
                Some(i) => at = i,
                None => {
                    self.message = Some(format!("no match for {pattern}"));
                    return from;
                }
            }
        }
        at
    }

    /// The text of row `i` of the shown mode, for searching.
    fn row_text(&self, i: usize, split: bool) -> String {
        if !split {
            return self
                .lines
                .get(i)
                .map_or_else(String::new, |l| l.text.clone());
        }
        match self.aligned.rows.get(i) {
            Some(Row::Line { left, right, .. }) => {
                fn side(s: &Option<Side>) -> &str {
                    s.as_ref().map_or("", |s| s.text.as_str())
                }
                format!("{}\n{}", side(left), side(right))
            }
            Some(Row::Hunk(t) | Row::Note(t)) => t.clone(),
            None => String::new(),
        }
    }

    /// The row where line `line` of the file (the new version's) is, or the nearest after it.
    fn row_of_line(&self, line: usize, split: bool) -> usize {
        if split {
            return self
                .aligned
                .rows
                .iter()
                .position(|r| matches!(r, Row::Line { right: Some(s), .. } if s.no >= line))
                .unwrap_or(self.aligned.rows.len().saturating_sub(1));
        }
        (0..self.lines.len())
            .find(|&i| {
                self.lines[i].kind != Kind::Del && self.numbers[i].is_some_and(|no| no >= line)
            })
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// The file `e` asked to edit, and the line, once.
    pub(crate) fn take_edit(&mut self) -> Option<(PathBuf, Option<usize>)> {
        self.edit.take()
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
        let col_w = geom.map_or(w.saturating_sub(1 + self.uni_gutter(w)), |g| {
            g.left - g.gutter - 3
        });
        self.colors.set((look.accent, palette.muted));
        if self.to_change.take()
            && let (Some(at), Some(change)) =
                (self.change_position(split), self.first_change(split))
        {
            self.scroll.set(at);
            self.cursor.set(change);
            self.scroll_split.set(split);
        }
        // A resize (or `s`) changed the mode: land on the same hunk, the cursor on its line.
        if self.scroll_split.get() != split {
            let from = self.scroll_split.get();
            let pos = self.convert(self.scroll.get(), from);
            self.scroll.set(pos);
            self.cursor.set(self.convert_row(self.cursor.get(), from));
            if let Some(v) = self.visual.get() {
                let row = self.convert_row(v.row, from);
                self.visual.set(Some(Visual { row, ..v }));
            }
            self.scroll_split.set(split);
        }
        self.view.set(View {
            rows,
            col_w,
            split,
            width: w,
        });
        // The cursor stays in view however the size changed.
        let cur = self.cursor.get();
        if cur < self.scroll.get().min(self.max_scroll()) || cur > self.last_visible() {
            self.scroll.set(self.top_for(cur, split, rows));
        }
        let n = self.len(split);
        let first = self.scroll.get().min(self.max_scroll());
        let hscroll = self.hscroll.min(self.max_hscroll());

        // Highlighted: what is on screen, and a screen above and below it for scrolling.
        let lazy = if split {
            &self.lit_split
        } else {
            &self.lit_uni
        };
        if let Some(lazy) = lazy {
            lazy.want(first.saturating_sub(rows), first + 2 * rows.max(1));
        }
        let (body, shown) = match &geom {
            Some(g) => self.split_lines(g, first, rows, hscroll, &look),
            None => self.unified_lines(w, first, rows, hscroll, &look),
        };
        let mut lines = vec![self.title_line(w, first, shown, n, &look)];
        if let Some(g) = &geom {
            lines.push(self.column_header(g, &look));
        }
        lines.extend(body);
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
        // A search being typed takes the line.
        if let Some(text) = &self.typing {
            let shown = truncate_start(text, w.saturating_sub(2).max(1));
            return Line::from(vec![
                Span::styled("/", Style::new().fg(look.accent)),
                Span::raw(shown),
                Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
            ]);
        }
        let mut title = vec![Span::styled(
            truncate_start(&self.title, w.saturating_sub(14).max(1)),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(message) = &self.message {
            title.push(Span::styled(
                format!("  {message}"),
                Style::new().fg(look.danger),
            ));
        }
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

    /// `rows` screen lines of the split view from row `first`, each exactly as wide as the
    /// overlay, and how many rows they show (a wrapped row takes several lines).
    fn split_lines(
        &self,
        g: &Geom,
        first: usize,
        rows: usize,
        hscroll: usize,
        look: &Look,
    ) -> (Vec<Line<'static>>, usize) {
        let total = g.left + 1 + g.right;
        let mut out = Vec::with_capacity(rows);
        let mut shown = 0;
        for (index, row) in self.aligned.rows.iter().enumerate().skip(first) {
            if out.len() >= rows {
                break;
            }
            shown += 1;
            for block in self.blocks_at(index, true) {
                out.extend(block);
            }
            let marks = self.marks(index, true);
            // A separator or a note across both sides; the cursor can be on it.
            let across = |text: &str, offset: usize, style: Style| {
                let mut chars: Vec<(char, usize)> = text.chars().map(|c| (c, 0)).collect();
                let mut styles = vec![style; 3];
                let extra = paint(&mut chars, &mut styles, marks, offset, look.selection);
                let cells = width(text) + extra;
                Line::from(windowed(chars.into_iter(), cells, 0, total, &styles, true))
            };
            match row {
                Row::Hunk(text) => {
                    out.push(across(
                        &format!("⋯ {text}"),
                        2,
                        with_bg(look.muted, look.band),
                    ));
                }
                Row::Note(text) => out.push(across(text, 0, look.muted)),
                Row::Line {
                    left,
                    right,
                    changed,
                } => {
                    let (lc, rc) = match self.lit_split.as_ref().and_then(|l| l.get(index)) {
                        Some((l, r)) => (l.as_slice(), r.as_slice()),
                        None => (&[][..], &[][..]),
                    };
                    let (lt, rt) = if *changed {
                        (Tone::Del, Tone::Add)
                    } else {
                        (Tone::Ctx, Tone::Ctx)
                    };
                    let (gut, wrap) = (g.gutter, self.wrap);
                    // The cursor is on the new side, or the old one when only it has a line.
                    let (lm, rm) = if right.is_some() {
                        (Marks::default(), marks)
                    } else {
                        (marks, Marks::default())
                    };
                    let (lw, rw) = ((gut, g.left, hscroll, wrap), (gut, g.right, hscroll, wrap));
                    let l = side_rows(left.as_ref(), lc, lt, lw, lm, look);
                    let r = side_rows(right.as_ref(), rc, rt, rw, rm, look);
                    // The shorter side goes on blank, in its tint, as long as the other wraps.
                    for i in 0..l.len().max(r.len()) {
                        let mut spans = l
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| side_filler(left.is_some(), lt, g.left, look));
                        spans.push(Span::styled("│", look.border));
                        spans.extend(
                            r.get(i)
                                .cloned()
                                .unwrap_or_else(|| side_filler(right.is_some(), rt, g.right, look)),
                        );
                        out.push(Line::from(spans));
                    }
                }
            }
        }
        out.truncate(rows);
        // Below the last row the columns go on.
        while out.len() < rows {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(g.left)),
                Span::styled("│", look.border),
                Span::raw(" ".repeat(g.right)),
            ]));
        }
        (out, shown)
    }

    /// Cells the line numbers of the unified view take, with the space after them; none when
    /// no line has one (a note) or `w` leaves too little room for the text.
    fn uni_gutter(&self, w: usize) -> usize {
        let gutter = self.aligned.gutter.max(3) + 1;
        if self.numbers.iter().any(Option::is_some) && w >= gutter + MIN_TEXT {
            gutter
        } else {
            0
        }
    }

    /// `rows` screen lines of the unified view from line `first`, and how many lines they show
    /// (a wrapped line takes several rows).
    fn unified_lines(
        &self,
        w: usize,
        first: usize,
        rows: usize,
        hscroll: usize,
        look: &Look,
    ) -> (Vec<Line<'static>>, usize) {
        let mut out = Vec::with_capacity(rows);
        let mut shown = 0;
        for (index, l) in self.lines.iter().enumerate().skip(first) {
            if out.len() >= rows {
                break;
            }
            shown += 1;
            for block in self.blocks_at(index, false) {
                out.extend(block);
            }
            out.extend(self.unified_rows(index, l, w, hscroll, look));
        }
        out.truncate(rows);
        (out, shown)
    }

    /// The screen rows of unified line `index`: one, or more when it wraps.
    fn unified_rows(
        &self,
        index: usize,
        l: &DiffLine,
        w: usize,
        hscroll: usize,
        look: &Look,
    ) -> Vec<Line<'static>> {
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
        let gutter = self.uni_gutter(w);
        let avail = w.saturating_sub(width(mark) + gutter);
        // A removed line is the old version's; the rest the new one's.
        let classes = match self.lit_uni.as_ref().and_then(|l| l.get(index)) {
            Some((old, _)) if l.kind == Kind::Del => old.as_slice(),
            Some((_, new)) => new.as_slice(),
            None => &[][..],
        };
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
        let mut chars: Vec<(char, usize)> = body
            .chars()
            .enumerate()
            .map(|(i, c)| match look.code(classes.get(i).copied()) {
                Some(code) => (c, style_id(&mut styles, base.patch(code))),
                None => (c, 0),
            })
            .collect();
        let marks = self.marks(index, false);
        let extra = paint(&mut chars, &mut styles, marks, 0, look.selection);
        let pad = lit && tone != Tone::Ctx;
        let total = width(body) + extra;
        let text = fitted(chars, total, hscroll, avail, &styles, pad, self.wrap);
        text.into_iter()
            .enumerate()
            .map(|(i, text)| {
                // The number and the sign on the first row; under them the line goes on.
                let mut spans = Vec::new();
                if i == 0 {
                    if gutter > 0 {
                        let no = self.numbers.get(index).copied().flatten();
                        let no = no.map_or_else(String::new, |n| n.to_string());
                        spans.push(Span::styled(format!("{no:>0$} ", gutter - 1), look.muted));
                    }
                    spans.push(Span::styled(mark.to_string(), style));
                } else {
                    spans.push(Span::styled(" ".repeat(gutter + width(mark)), base));
                }
                spans.extend(text);
                Line::from(spans)
            })
            .collect()
    }

    /// Screen rows row `i` of the shown mode takes: one, or more when it wraps.
    fn height(&self, i: usize, split: bool) -> usize {
        let blocks: usize = if self.comments.is_empty() && self.writing.is_none() {
            0
        } else {
            self.blocks_at(i, split).iter().map(Vec::len).sum()
        };
        blocks + self.text_height(i, split)
    }

    /// Screen rows the text of row `i` takes, without the comments above it.
    fn text_height(&self, i: usize, split: bool) -> usize {
        let v = self.view.get();
        if !self.wrap {
            return 1;
        }
        if split {
            return match self.aligned.rows.get(i) {
                Some(Row::Line { left, right, .. }) => {
                    let side = |s: &Option<Side>| {
                        s.as_ref().map_or(1, |s| {
                            let eol = if s.no_eol { width(NO_EOL) } else { 0 };
                            wrapped_rows(width(&s.text) + eol, v.col_w)
                        })
                    };
                    side(left).max(side(right))
                }
                _ => 1,
            };
        }
        self.lines.get(i).map_or(1, |l| {
            wrapped_rows(width(&l.text).saturating_sub(1), v.col_w)
        })
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
    accent: Color,
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
    syntax: Option<[Style; syntax::CLASSES]>,
    /// What `v` / `V` selected: the accent mixed in, underlined without colors to mix.
    selection: Style,
}

impl Look {
    fn new(p: &Palette) -> Look {
        Look {
            muted: Style::new().fg(p.muted),
            accent: p.accent,
            border: Style::new().fg(p.border),
            success: p.success,
            danger: p.danger,
            del_bg: blend(p.bg, p.danger, 16),
            del_hl: blend(p.bg, p.danger, 38),
            add_bg: blend(p.bg, p.success, 16),
            add_hl: blend(p.bg, p.success, 38),
            band: matches!(p.surface, Color::Rgb(..)).then_some(p.surface),
            syntax: code_colors(p),
            selection: match blend(p.bg, p.accent, 35) {
                Some(c) => Style::new().bg(c),
                None => Style::new().add_modifier(Modifier::UNDERLINED),
            },
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

/// What the highlighting worker parses, for the unified lines and for the split rows, and the
/// grammar it starts with: the one for `path` (a pull request's diff: the path in each file's
/// `+++ b/` header). Old and new lines are parsed apart, each in order, and a hunk starts the
/// parse over.
fn steps(
    path: Option<&str>,
    lines: &[DiffLine],
    aligned: &Aligned,
) -> (Option<&'static SyntaxReference>, Vec<Step>, Vec<Step>) {
    let header_path = |l: &DiffLine| l.text.strip_prefix("+++ b/").map(str::to_string);
    let first = path
        .map(str::to_string)
        .or_else(|| lines.iter().find_map(header_path));
    let grammar = first.as_deref().and_then(syntax_for);
    let mut current = grammar;
    let uni = lines
        .iter()
        .map(|l| {
            let body = l
                .text
                .get(l.text.chars().next().map_or(0, char::len_utf8)..)
                .unwrap_or("")
                .to_string();
            match l.kind {
                Kind::Header => match header_path(l).filter(|_| path.is_none()) {
                    Some(p) => {
                        current = syntax_for(&p);
                        Step::Reset(current)
                    }
                    None => Step::Skip,
                },
                Kind::Hunk => Step::Reset(current),
                Kind::Context => Step::Line {
                    old: Some(body.clone()),
                    new: Some(body),
                },
                Kind::Add => Step::Line {
                    old: None,
                    new: Some(body),
                },
                Kind::Del => Step::Line {
                    old: Some(body),
                    new: None,
                },
                Kind::Note => Step::Skip,
            }
        })
        .collect();
    let split = aligned
        .rows
        .iter()
        .map(|row| match row {
            Row::Hunk(_) => Step::Reset(grammar),
            Row::Line { left, right, .. } => Step::Line {
                old: left.as_ref().map(|s| s.text.clone()),
                new: right.as_ref().map(|s| s.text.clone()),
            },
            Row::Note(_) => Step::Skip,
        })
        .collect();
    (grammar, uni, split)
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

/// Cells a text takes as rows of `avail` cells when wrapped (at least one).
fn wrapped_rows(cells: usize, avail: usize) -> usize {
    cells.div_ceil(avail.max(1)).max(1)
}

/// A line of `(char, style index)` as screen rows of exactly `avail` cells when `pad`: wrapped
/// at the edge when `wrap` (a wide character that doesn't fit goes to the next row), else one
/// row from cell `skip` on, cut with a `…`.
fn fitted(
    chars: Vec<(char, usize)>,
    total: usize,
    skip: usize,
    avail: usize,
    styles: &[Style],
    pad: bool,
    wrap: bool,
) -> Vec<Vec<Span<'static>>> {
    if !wrap || avail == 0 {
        return vec![windowed(chars.into_iter(), total, skip, avail, styles, pad)];
    }
    let mut rows: Vec<Vec<(char, usize)>> = vec![Vec::new()];
    let mut used = 0;
    for (c, id) in chars {
        let w = c.width().unwrap_or(0);
        if used + w > avail && used > 0 {
            rows.push(Vec::new());
            used = 0;
        }
        rows.last_mut().expect("never empty").push((c, id));
        used += w;
    }
    rows.into_iter()
        .map(|row| {
            let w = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
            windowed(row.into_iter(), w, 0, avail, styles, pad)
        })
        .collect()
}

/// A column of a split row where its side has no more to show, `col` cells: the row's tint, or
/// the band when the side has no line at all.
fn side_filler(present: bool, tone: Tone, col: usize, look: &Look) -> Vec<Span<'static>> {
    let bg = if present { look.bg(tone) } else { look.band };
    vec![Span::styled(" ".repeat(col), with_bg(Style::new(), bg))]
}

/// One column of a split row, each screen row exactly `col` cells: the line number, a sign, the
/// text (wrapped, or from cell `skip` on) with the cursor and selection on it; a blank filler
/// when this side has no line. `cols` is the gutter, the column's width, `skip` and `wrap`.
fn side_rows(
    side: Option<&Side>,
    classes: &[Class],
    tone: Tone,
    cols: (usize, usize, usize, bool),
    marks: Marks,
    look: &Look,
) -> Vec<Vec<Span<'static>>> {
    let (gutter, col, skip, wrap) = cols;
    let Some(side) = side else {
        return vec![side_filler(false, tone, col, look)];
    };
    let base = look.base(tone);
    let bg = with_bg(Style::new(), look.bg(tone));
    let sign = match tone {
        Tone::Ctx => " ",
        Tone::Del => "-",
        Tone::Add => "+",
    };
    let avail = col.saturating_sub(gutter + 3);
    let hl = side.hl.filter(|_| tone != Tone::Ctx);
    // Highlighted code keeps the change's color as a background tint.
    let mut styles = vec![base, look.changed(tone), bg.patch(look.muted)];
    let mut chars: Vec<(char, usize)> = side
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
    let extra = paint(&mut chars, &mut styles, marks, 0, look.selection);
    let total = width(&side.text) + extra + if side.no_eol { width(NO_EOL) } else { 0 };
    let chars = chars.into_iter().chain(eol.map(|c| (c, 2))).collect();
    fitted(chars, total, skip, avail, &styles, true, wrap)
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            // The number and the sign on the first row; under them the text goes on.
            let mut spans = if i == 0 {
                vec![
                    Span::styled(format!("{:>gutter$} ", side.no), bg.patch(look.muted)),
                    Span::styled(sign, base),
                    Span::styled(" ", bg),
                ]
            } else {
                vec![Span::styled(" ".repeat(gutter + 3), bg)]
            };
            spans.extend(text);
            spans
        })
        .collect()
}

/// Paints the selection and the cursor of `m` on a row's chars, whose text starts at char
/// `offset`; a cursor or selection past the end gets a blank after the text. The cells added.
fn paint(
    chars: &mut Vec<(char, usize)>,
    styles: &mut Vec<Style>,
    m: Marks,
    offset: usize,
    selection: Style,
) -> usize {
    if m == Marks::default() {
        return 0;
    }
    let len = chars.len().saturating_sub(offset);
    let need = m
        .cursor
        .map_or(0, |c| c + 1)
        .max(m.sel.map_or(0, |(_, b)| b));
    let extra = usize::from(need > len);
    if extra == 1 {
        chars.push((' ', 0));
    }
    let end = len + extra;
    let mut over = |at: usize, patch: Style| {
        if let Some(ch) = chars.get_mut(offset + at) {
            let style = styles[ch.1].patch(patch);
            ch.1 = style_id(styles, style);
        }
    };
    if let Some((a, b)) = m.sel {
        for at in a..b.min(end) {
            over(at, selection);
        }
    }
    if let Some(c) = m.cursor {
        over(
            c.min(end.saturating_sub(1)),
            Style::new().add_modifier(Modifier::REVERSED),
        );
    }
    extra
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

/// The line number of every unified line, counted from the hunk headers: the new file's for
/// context and additions, the old file's for removals.
fn line_numbers(lines: &[DiffLine]) -> Vec<Option<usize>> {
    let (mut old, mut new) = (1, 1);
    lines
        .iter()
        .map(|l| match l.kind {
            Kind::Hunk => {
                (old, new) = hunk_start(&l.text);
                None
            }
            Kind::Context => {
                old += 1;
                new += 1;
                Some(new - 1)
            }
            Kind::Add => {
                new += 1;
                Some(new - 1)
            }
            Kind::Del => {
                old += 1;
                Some(old - 1)
            }
            Kind::Header | Kind::Note => None,
        })
        .collect()
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
    let out = cmd
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    // --no-index exits 1 when the files differ.
    let ok = out.status.success() || (no_index && out.status.code() == Some(1));
    if ok {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    // What git said, on one line: the view shows it in place of the diff.
    let said = String::from_utf8_lossy(&out.stderr);
    let said = said.lines().map(str::trim).find(|l| !l.is_empty());
    Err(said.map_or_else(|| "git diff failed".into(), clean_line))
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
    if req.mark == Mark::Committed {
        // What the last commit that touched it changed. Asked where the file is: one outside the
        // session's repository (../other/a.rs) is in a repository of its own.
        let log = [
            "log",
            "-1",
            "--format=",
            "-p",
            "--no-color",
            "--no-ext-diff",
        ];
        let path = cwd.join(&req.rel);
        let (dir, name) = match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) if dir.is_dir() => (dir.to_path_buf(), PathBuf::from(name)),
            _ => (cwd.to_path_buf(), PathBuf::from(&req.rel)),
        };
        let out = run(
            git(&dir).args(log).arg(&context).arg("--").arg(&name),
            false,
        )?;
        return if out.trim().is_empty() {
            Err("no change left: it was changed back".into())
        } else {
            Ok(out)
        };
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
    fn long_lines_wrap_under_their_number_and_w_cuts_them_instead() {
        let line = "0123456789abcdefghijklmnopqrstuvwxyz";
        let mut v = DiffView::from_text("f", &format!("@@ -1,2 +1,2 @@\n+{line}\n ok\n"));
        let mut t = Terminal::new(TestBackend::new(20, 6)).unwrap();
        let out = draw_to(&v, &mut t);
        let rows: Vec<&str> = out.lines().collect();
        // The gutter is 4 cells, the sign 1: 15 cells of text a row.
        assert_eq!(rows[2], "  1 +0123456789abcde");
        assert_eq!(rows[3], "     fghijklmnopqrst");
        assert_eq!(rows[4], "     uvwxyz");
        assert_eq!(rows[5], "  2  ok");
        // The title counts lines of the diff, not rows of the screen.
        assert!(rows[0].ends_with("1–3/3"), "{}", rows[0]);
        // Nothing to scroll sideways while wrapping; `<leader>w` cuts again.
        ch(&mut v, 'j');
        ch(&mut v, '$');
        assert_eq!(v.hscroll, 0);
        ch(&mut v, '0');
        ch(&mut v, ' ');
        ch(&mut v, 'w');
        let cut = draw_to(&v, &mut t);
        assert_eq!(cut.lines().nth(2).unwrap(), "  1 +0123456789abcd…");
    }

    #[test]
    fn wrapped_rows_still_let_the_end_fill_the_screen() {
        let mut text = String::from("@@ -1,20 +1,20 @@\n");
        for i in 0..20 {
            text.push_str(&format!(" {i:02} {}\n", "x".repeat(30)));
        }
        let mut v = DiffView::from_text("f", &text);
        let mut t = Terminal::new(TestBackend::new(24, 9)).unwrap();
        draw_to(&v, &mut t);
        ch(&mut v, 'G');
        let out = draw_to(&v, &mut t);
        // Each line takes two rows: the last four lines fill the 8 rows.
        assert!(out.lines().nth(1).unwrap().contains(" 16 "), "{out}");
        assert!(
            out.lines().last().unwrap().trim_start().starts_with('x'),
            "{out}"
        );
    }

    #[test]
    fn draws_the_unified_view_when_narrow() {
        let mut v = DiffView::from_text("src/a.ts", SAMPLE);
        v.wrap = false;
        let mut t = Terminal::new(TestBackend::new(24, 8)).unwrap();
        let out = draw_to(&v, &mut t);
        let first = out.lines().next().unwrap();
        assert_eq!(first, "src/a.ts  +1 −2   1–7/10");
        // Headers get an empty gutter, so the code lines up under them.
        assert_eq!(out.lines().nth(1).unwrap(), "    diff --git a/src/a.…");
        assert_eq!(out.lines().count(), 8);
        // Colors.
        let p = Palette::default();
        let buf = t.backend().buffer();
        assert_eq!(buf[(0, 5)].fg, p.muted); // @@
        assert_eq!(buf[(0, 7)].fg, p.muted); // its line number
        assert_eq!(buf[(4, 7)].fg, p.danger); // -old line
        assert_eq!(buf[(10, 0)].fg, p.success); // +1 in the title
    }

    #[test]
    fn unified_view_scrolls_sideways_with_the_mark_in_place() {
        let mut v = DiffView::from_text("f", "@@ -1 +1 @@\n+0123456789abcdef\n");
        v.wrap = false;
        let mut t = Terminal::new(TestBackend::new(10, 4)).unwrap();
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
        // The view follows the cursor sideways, the sign staying put.
        ch(&mut v, 'j');
        for _ in 0..6 {
            ch(&mut v, 'l');
        }
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
        ch(&mut v, 'l');
        assert_ne!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
        // Not past the end.
        ch(&mut v, '$');
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+89abcdef");
        ch(&mut v, 'l');
        assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+89abcdef");
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
        /// Waits until rows `from..to` of both views are highlighted.
        fn wait_for_syntax(&mut self, from: usize, to: usize) {
            for lazy in [&mut self.lit_uni, &mut self.lit_split]
                .into_iter()
                .flatten()
            {
                lazy.wait(from, to);
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
        v.wait_for_syntax(0, 10);
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
        assert_eq!(buf[(4, 6)].fg, Palette::default().success);
    }

    #[test]
    fn a_huge_file_is_highlighted_where_it_is_looked_at() {
        let n = 200_000;
        let mut text = format!("--- a/big.rs\n+++ b/big.rs\n@@ -1,{n} +1,{n} @@\n");
        for i in 0..n {
            text.push_str(&format!(
                " fn f{i}(x: u32) -> u32 {{ x * {i} + \"s\".len() as u32 }} // {i}\n"
            ));
        }
        let mut v = DiffView::from_text("big.rs", &text);
        assert_eq!(v.lines.len(), n + 3, "nothing cut");
        let mut t = Terminal::new(TestBackend::new(70, 12)).unwrap();
        draw_to(&v, &mut t);
        // The end, far from where any parse is.
        ch(&mut v, 'G');
        draw_to(&v, &mut t);
        let started = std::time::Instant::now();
        v.wait_for_syntax(n - 10, n + 3);
        let took = started.elapsed();
        eprintln!("the end of {n} lines highlighted in {took:?}");
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
        let lazy = v.lit_uni.as_ref().unwrap();
        assert!(lazy.get(n).unwrap().1.contains(&Class::Keyword));
        assert!(lazy.get(n / 2).is_none(), "the middle was never looked at");
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
            panel: Color::Reset,
            raised: Color::Reset,
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
        // Down moves the cursor; the view only once it would leave it.
        assert!(!press(&mut v, KeyCode::Down));
        let out = draw_to(&v, &mut t);
        assert_eq!(v.cursor.get(), 1);
        assert!(line(&out, 0).ends_with("lines 1–10 of 41"));
        for _ in 0..9 {
            press(&mut v, KeyCode::Down);
        }
        let out = draw_to(&v, &mut t);
        assert!(line(&out, 1).starts_with("  1 +line 0"));
        assert!(line(&out, 0).ends_with("lines 2–11 of 41"));
        press(&mut v, KeyCode::Char('G'));
        let out = draw_to(&v, &mut t);
        assert!(out.lines().last().unwrap().starts_with(" 40 +line 39"));
        assert!(line(&out, 0).ends_with("lines 32–41 of 41"));
        press(&mut v, KeyCode::Down);
        assert_eq!(draw_to(&v, &mut t).lines().last().unwrap(), " 40 +line 39");
        press(&mut v, KeyCode::PageUp);
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with(" 31 +line 30")
        );
        v.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with(" 26 +line 25")
        );
        v.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with(" 31 +line 30")
        );
        press(&mut v, KeyCode::PageDown);
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .last()
                .unwrap()
                .starts_with(" 40 +line 39")
        );
        ch(&mut v, 'g');
        ch(&mut v, 'g');
        assert!(line(&draw_to(&v, &mut t), 1).starts_with("    @@"));
        ch(&mut v, 'j');
        ch(&mut v, 'j');
        ch(&mut v, 'k');
        assert_eq!(v.cursor.get(), 1);
        assert!(line(&draw_to(&v, &mut t), 1).starts_with("    @@"));
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

    fn ctrl(v: &mut DiffView, c: char) -> bool {
        v.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    #[test]
    fn vim_keys_counts_and_line_jumps() {
        let mut v = spread();
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap(); // unified, 9 rows
        draw_to(&v, &mut t);
        assert!(!v.view.get().split);
        let at = |v: &DiffView| (v.cursor.get(), v.scroll.get());
        ch(&mut v, '5');
        ch(&mut v, 'j');
        assert_eq!(at(&v), (5, 0));
        // Ctrl-E / Ctrl-Y move the view, not the cursor.
        ctrl(&mut v, 'e');
        assert_eq!(at(&v), (5, 1));
        ctrl(&mut v, 'y');
        ctrl(&mut v, 'y');
        assert_eq!(at(&v), (5, 0));
        // Half a page and a page move both.
        ctrl(&mut v, 'd');
        assert_eq!(at(&v), (9, 4));
        ctrl(&mut v, 'u');
        assert_eq!(at(&v), (5, 0));
        ctrl(&mut v, 'f');
        assert_eq!(at(&v), (13, 8));
        ctrl(&mut v, 'b');
        assert_eq!(at(&v), (5, 0));
        // H / M / L: the top, middle and bottom of the view.
        ch(&mut v, 'L');
        assert_eq!(at(&v), (8, 0));
        ch(&mut v, 'M');
        assert_eq!(at(&v), (4, 0));
        ch(&mut v, 'H');
        assert_eq!(at(&v), (0, 0));
        // `}` / `{` between hunks (each 13 unified rows: its `@@` and 12 lines); a count goes
        // further.
        assert_eq!(v.hunk_starts(false), [0, 13, 26]);
        ch(&mut v, '}');
        assert_eq!(at(&v), (13, 13));
        ch(&mut v, 'g');
        ch(&mut v, 'g');
        assert_eq!(v.cursor.get(), 0);
        ch(&mut v, '2');
        ch(&mut v, '}');
        assert_eq!(v.cursor.get(), 26);
        ch(&mut v, '{');
        assert_eq!(v.cursor.get(), 13);
        // `23G`: the row of the file's line 23 (in the second hunk, which starts at line 21).
        ch(&mut v, '2');
        ch(&mut v, '3');
        ch(&mut v, 'G');
        assert_eq!(v.numbers[v.cursor.get()], Some(23));
        // In a line: `$` to its end, `0` back; w / e / b by words, on into the next line.
        ch(&mut v, 'g');
        ch(&mut v, 'g');
        ch(&mut v, 'j');
        assert_eq!(v.row_body(1, false), "ctx 0.0");
        ch(&mut v, '$');
        assert_eq!(v.col, 6);
        ch(&mut v, '0');
        assert_eq!(v.col, 0);
        let mut word = |c: char| {
            ch(&mut v, c);
            (v.cursor.get(), v.col)
        };
        assert_eq!(word('w'), (1, 4));
        assert_eq!(word('w'), (1, 5));
        assert_eq!(word('w'), (1, 6));
        assert_eq!(word('w'), (2, 0));
        assert_eq!(word('e'), (2, 2));
        assert_eq!(word('b'), (2, 0));
        assert_eq!(word('b'), (1, 6));
    }

    #[test]
    fn slash_searches_and_n_goes_on_with_smart_case() {
        let mut v = spread();
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        draw_to(&v, &mut t);
        ch(&mut v, '/');
        for c in "new".chars() {
            ch(&mut v, c);
        }
        assert!(draw_to(&v, &mut t).starts_with("/new"));
        press(&mut v, KeyCode::Enter);
        let at = v.scroll.get();
        assert!(v.lines[at].text.contains("new 0"), "{}", v.lines[at].text);
        ch(&mut v, 'n');
        assert!(v.lines[v.scroll.get()].text.contains("new 1"));
        ch(&mut v, 'N');
        assert!(v.lines[v.scroll.get()].text.contains("new 0"));
        // Upper case makes it exact: nothing, and the title says so.
        ch(&mut v, '/');
        for c in "NEW".chars() {
            ch(&mut v, c);
        }
        press(&mut v, KeyCode::Enter);
        assert!(v.lines[v.scroll.get()].text.contains("new 0"));
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .next()
                .unwrap()
                .contains("no match for NEW")
        );
        // Esc while typing gives up without closing.
        ch(&mut v, '/');
        assert!(!press(&mut v, KeyCode::Esc));
        assert!(v.typing.is_none());
    }

    #[test]
    fn leader_e_asks_for_the_file_in_the_editor_at_the_cursor() {
        let dir = std::env::temp_dir();
        let mut v = file_view(&dir, "@@ -10,3 +10,3 @@\n a\n-b\n+B\n c\n");
        // On the `@@`: the first line in view.
        ch(&mut v, ' ');
        ch(&mut v, 'e');
        assert_eq!(v.take_edit(), Some((dir.join("src/a.rs"), Some(10))));
        assert_eq!(v.take_edit(), None);
        // On `+B`, line 11; `\` is the leader too.
        for _ in 0..3 {
            ch(&mut v, 'j');
        }
        ch(&mut v, '\\');
        ch(&mut v, 'e');
        assert_eq!(v.take_edit(), Some((dir.join("src/a.rs"), Some(11))));
        // A pull request's diff has no file here.
        let mut pr = DiffView::from_text("pr", "@@ -1 +1 @@\n-a\n+b\n");
        ch(&mut pr, ' ');
        ch(&mut pr, 'e');
        assert_eq!(pr.take_edit(), None);
        assert_eq!(pr.message.as_deref(), Some("not a file here"));
    }

    /// The diff of `src/a.rs` in `dir`, as the files panel opens it.
    fn file_view(dir: &Path, diff: &str) -> DiffView {
        let mut v = DiffView::blank("src/a.rs".into());
        v.cwd = dir.to_path_buf();
        v.req = Some(DiffRequest {
            rel: "src/a.rs".into(),
            orig: None,
            mark: Mark::Modified,
            in_repo: true,
        });
        v.finish(Ok(diff.into()));
        v
    }

    fn typed(v: &mut DiffView, text: &str) {
        for c in text.chars() {
            ch(v, c);
        }
    }

    #[test]
    fn leader_c_comments_on_a_line_and_the_prompt_gets_it() {
        let mut v = file_view(&std::env::temp_dir(), "@@ -10,3 +10,3 @@\n a\n-b\n+B\n c\n");
        let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
        draw_to(&v, &mut t);
        // On `+B` (line 11): the block opens above it, the file split there.
        for _ in 0..3 {
            ch(&mut v, 'j');
        }
        typed(&mut v, " c");
        typed(&mut v, "use a const");
        let out = draw_to(&v, &mut t);
        let rows: Vec<&str> = out.lines().collect();
        let head = rows
            .iter()
            .position(|r| r.contains("╭ @src/a.rs:11"))
            .expect(&out);
        assert!(rows[head + 1].starts_with("│ use a const"), "{out}");
        assert!(rows[head + 2].contains("enter keeps it"), "{out}");
        assert!(rows[head + 3].contains("+B"), "{out}");
        // Keys go to the comment while it is written: j is a letter.
        ch(&mut v, 'j');
        press(&mut v, KeyCode::Backspace);
        press(&mut v, KeyCode::Enter);
        assert_eq!(
            v.take_comment_changes(),
            [(None, Some("@src/a.rs:11 use a const".to_string()))]
        );
        let out = draw_to(&v, &mut t);
        assert!(out.contains("╭ @src/a.rs:11"), "{out}");
        assert!(out.contains("│ use a const"), "{out}");
        assert!(!out.contains("enter keeps it"), "{out}");
        // On the commented line again: edits it; the prompt's copy is replaced.
        typed(&mut v, " c");
        assert_eq!(
            v.writing.as_ref().map(|w| w.text.as_str()),
            Some("use a const")
        );
        typed(&mut v, "ant");
        press(&mut v, KeyCode::Enter);
        assert_eq!(
            v.take_comment_changes(),
            [(
                Some("@src/a.rs:11 use a const".to_string()),
                Some("@src/a.rs:11 use a constant".to_string())
            )]
        );
        // Emptied: it goes.
        typed(&mut v, " c");
        ctrl(&mut v, 'u');
        press(&mut v, KeyCode::Enter);
        assert_eq!(
            v.take_comment_changes(),
            [(Some("@src/a.rs:11 use a constant".to_string()), None)]
        );
        assert!(v.comments.is_empty());
        // Esc gives up and leaves nothing.
        typed(&mut v, " cnever mind");
        assert!(!press(&mut v, KeyCode::Esc), "the view stays open");
        assert!(v.writing.is_none());
        assert!(v.take_comment_changes().is_empty());
    }

    #[test]
    fn y_copies_the_selection_or_lines_without_their_signs() {
        let mut v = file_view(
            &std::env::temp_dir(),
            "@@ -1,3 +1,3 @@\n alpha\n-beta\n+gamma one\n",
        );
        let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
        draw_to(&v, &mut t);
        // `V` and `y`: whole lines; the cursor goes back to the first.
        typed(&mut v, "jVjjy");
        assert_eq!(v.take_copy().as_deref(), Some("alpha\nbeta\ngamma one"));
        assert!(v.visual.get().is_none());
        assert_eq!(v.cursor.get(), 1);
        // `v` and `y`: from the char it started on to the cursor's, both in.
        typed(&mut v, "lvjjey");
        assert_eq!(v.take_copy().as_deref(), Some("lpha\nbeta\ngamma"));
        assert_eq!((v.cursor.get(), v.col), (1, 1));
        // `yy`: the cursor's line; `2yy` two of them.
        typed(&mut v, "yy");
        assert_eq!(v.take_copy().as_deref(), Some("alpha"));
        typed(&mut v, "2yy");
        assert_eq!(v.take_copy().as_deref(), Some("alpha\nbeta"));
        assert_eq!(v.take_copy(), None);
    }

    #[test]
    fn visual_selections_comment_on_their_lines() {
        let mut v = file_view(
            &std::env::temp_dir(),
            "@@ -10,4 +10,4 @@\n a\n-b\n+B\n c\n d\n",
        );
        let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
        draw_to(&v, &mut t);
        // `V` from `a` (10) down to `c` (12): the removed line has no new number.
        ch(&mut v, 'j');
        ch(&mut v, 'V');
        typed(&mut v, "jjj");
        assert_eq!(v.marks(2, false).sel, Some((0, 1)));
        typed(&mut v, " c");
        assert!(v.visual.get().is_none(), "the comment ends the selection");
        typed(&mut v, "why");
        press(&mut v, KeyCode::Enter);
        assert_eq!(
            v.take_comment_changes(),
            [(None, Some("@src/a.rs:10-12 why".to_string()))]
        );
        // `v` selects characters: from the cursor's to the one it moves to.
        ch(&mut v, 'G');
        ch(&mut v, '0');
        ch(&mut v, 'v');
        assert_eq!(v.marks(5, false).sel, Some((0, 1)));
        ch(&mut v, 'k');
        assert_eq!(v.marks(4, false).sel, Some((0, 1)));
        assert_eq!(v.marks(5, false).sel, Some((0, 1)));
        // Esc leaves visual mode first, then the view.
        assert!(!press(&mut v, KeyCode::Esc));
        assert!(v.visual.get().is_none());
        assert!(press(&mut v, KeyCode::Esc));
        // Only the old side has a removed line: its comment counts in the old numbers.
        let mut w = file_view(&std::env::temp_dir(), "@@ -10,2 +10,1 @@\n a\n-b\n");
        draw_to(&w, &mut t);
        typed(&mut w, "jj cgone?");
        press(&mut w, KeyCode::Enter);
        assert!(w.comments[0].old);
        assert_eq!(w.comments[0].start, 11);
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
        // The mouse wheel is Ctrl-Y / Ctrl-E: the view moves, the cursor (at the end) with it
        // only as far as it has to.
        assert_eq!(v.cursor.get(), 35);
        ctrl(&mut v, 'y');
        ctrl(&mut v, 'y');
        assert_eq!(v.scroll.get(), 26);
        assert_eq!(v.cursor.get(), 33);
        press(&mut v, KeyCode::End);
        assert_eq!(v.scroll.get(), 28);
        press(&mut v, KeyCode::Home);
        assert_eq!(v.scroll.get(), 0);
    }

    #[test]
    fn scrolls_both_sides_sideways() {
        let mut v = DiffView::from_text("src/app.ts", BIG);
        v.wrap = false;
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let out = draw_to(&v, &mut t);
        let at = |out: &str, needle: &str| out.lines().position(|l| l.contains(needle)).unwrap();
        let long = at(&out, "export const long");
        let cut = out.lines().nth(long).unwrap();
        assert_eq!(cut.matches('…').count(), 2, "{cut}");
        assert!(!cut.contains("sideways"));
        // The cursor on the long line (the new side's): `$` takes both sides to its end.
        ch(&mut v, '/');
        for c in "const long".chars() {
            ch(&mut v, c);
        }
        press(&mut v, KeyCode::Enter);
        ch(&mut v, '$');
        let out = draw_to(&v, &mut t);
        let cut = out.lines().nth(long).unwrap();
        // The end of both lines is in view, and no `…` is left.
        assert_eq!(cut.matches("sideways").count(), 2, "{cut}");
        assert!(cut.trim_end().ends_with("sideways!\";"), "{cut}");
        assert!(v.hscroll > 0);
        // The line numbers stay.
        assert!(out.lines().nth(2).unwrap().starts_with("⋯"));
        // Back a few columns stays where it is; `0` goes back to the left edge.
        let at = v.hscroll;
        press(&mut v, KeyCode::Left);
        assert_eq!(v.hscroll, at);
        ch(&mut v, '0');
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
                .starts_with("    @@ -41,12 +41,12 @@ section 2"),
            "{out}"
        );
        ch(&mut v, 'p');
        assert!(
            draw_to(&v, &mut t)
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("    @@ -21,12")
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
        for (force, wrap) in [None, Some(true), Some(false)]
            .into_iter()
            .flat_map(|f| [(f, true), (f, false)])
        {
            v.force = force;
            v.wrap = wrap;
            for w in 1..=140usize {
                for h in [1u16, 3, 9] {
                    for hscroll in [0, 3, 21] {
                        v.hscroll = hscroll;
                        // The rows themselves.
                        match v.split_geom(w) {
                            Some(g) => {
                                let (rows, _) = v.split_lines(&g, 0, 12, hscroll, &look);
                                for l in rows {
                                    assert_eq!(l.width(), w, "split {w} {hscroll}: {l:?}");
                                }
                            }
                            None => {
                                let (rows, _) = v.unified_lines(w, 0, 12, hscroll, &look);
                                for l in rows {
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
            std::env::temp_dir().join(format!("neo-term-diff-{}-{nanos}", std::process::id()));
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

        // Opened whole (from the files view), it starts five lines above the change.
        let req = DiffRequest {
            rel: "f.txt".into(),
            orig: None,
            mark: Mark::Modified,
            in_repo: true,
        };
        let mut v = DiffView::open_file(dir.clone(), req);
        wait(&mut v);
        let out = draw_to(&v, &mut t);
        // Unified: one header row.
        let first = out.lines().nth(1).unwrap();
        assert!(first.contains("line 25"), "{out}");
        assert!(out.contains("THIRTY"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
