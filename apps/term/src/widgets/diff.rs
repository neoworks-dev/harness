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

// The rest of `impl DiffView` (keys; comments, copies and searches; drawing), then the
// painting of rows, the reading of a diff and the asking of git.
mod actions;
mod draw;
mod fetch;
mod keys;
mod paint;
mod parse;
use fetch::*;
use paint::*;
use parse::*;
pub(crate) use parse::{align, parse_diff};

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

/// A comment on lines of the file, written in the view (`c`) and put in the prompt as
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
    /// An edit waiting for an answer, not git's diff: there is nothing to reload.
    proposed: bool,
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
            proposed: false,
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

    /// An edit a permission request asks about (or, in review mode, made), the whole file
    /// with it marked, unified: `lines` are the request's diff, the file is read from disk.
    pub(crate) fn proposed(cwd: PathBuf, rel: &str, lines: &[neo_protocol::DiffLine]) -> DiffView {
        let text = proposed_diff(&cwd, rel, lines);
        let mut v = DiffView {
            cwd,
            req: Some(DiffRequest {
                rel: rel.to_string(),
                orig: None,
                mark: Mark::Modified,
                in_repo: false,
            }),
            full: true,
            proposed: true,
            force: Some(false),
            ..DiffView::blank(rel.to_string())
        };
        v.finish(text);
        v
    }

    /// A diff that is already at hand (the GitHub view's pull request diffs).
    pub(crate) fn from_text(title: &str, text: &str) -> DiffView {
        let mut v = DiffView::blank(title.to_string());
        v.finish(Ok(text.to_string()));
        v
    }

    /// What a commit changed (`git show`, a merge against its first parent), fetched on a
    /// thread.
    pub(crate) fn commit(cwd: PathBuf, sha: &str, title: &str) -> DiffView {
        let short: String = sha.chars().take(7).collect();
        let (tx, rx) = mpsc::channel();
        let (dir, sha) = (cwd.clone(), sha.to_string());
        std::thread::spawn(move || {
            let _ = tx.send(git_show(&dir, &sha));
        });
        DiffView {
            cwd,
            rx: Some(rx),
            ..DiffView::blank(format!("{short} {title}").trim_end().to_string())
        }
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
        if self.proposed {
            self.message = Some("the edit is shown in its whole file".into());
            return;
        }
        self.full = !self.full;
        let split = self.view.get().split;
        self.anchor = self
            .row_nos(self.cursor.get(), split)
            .1
            .or_else(|| self.top_new_line());
        self.rx = Some(spawn(self.cwd.clone(), req, self.full));
        self.loaded = false;
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

#[cfg(test)]
mod tests;
