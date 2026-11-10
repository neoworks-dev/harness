//! A reply opened from the chat as its markdown, in place of the chat, or plain text (a check's
//! log): a cursor that moves with vim's keys, `v` / `V` select, `y` copies, `/` searches and
//! `n` / `N` go between the matches.

use neo_renderer_ratatui::Palette;
use neo_renderer_ratatui::syntax::{self, Class, Highlighter};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthChar;

use crate::util::width;
use crate::widgets::truncate_start;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Visual,
    VisualLine,
}

/// A place in the text: a line and a char of it.
type Pos = (usize, usize);

/// How far a motion's range reaches, for `y` with one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Whole lines (`j`, `G`, `}`).
    Lines,
    /// Up to the char before the end (`w`, `b`, `0`).
    Exclusive,
    /// Up to and with the end (`e`, `$`).
    Inclusive,
}

pub enum ReaderAction {
    Redraw,
    Close,
}

pub struct Reader {
    title: String,
    lines: Vec<Vec<char>>,
    /// The syntax class of each char, as markdown.
    classes: Vec<Vec<Class>>,
    cur: Pos,
    /// The column `j` / `k` aim for: where the cursor last moved sideways (`usize::MAX` after
    /// `$`, the end of every line).
    want: usize,
    mode: Mode,
    anchor: Pos,
    /// The first line in view.
    top: usize,
    /// The text's area as drawn last.
    height: usize,
    width: usize,
    count: usize,
    /// `g` or `y` waiting for the rest of the command.
    pending: Option<char>,
    copy: Option<String>,
    /// What the title line says about the keys.
    hint: String,
    /// The next draw shows the end (it knows how tall the view is).
    end: bool,
    /// `/` is being typed: the pattern so far.
    typing: Option<String>,
    /// The last search, for `n` / `N` and to mark its matches.
    pattern: Option<String>,
    /// Said in the title line until the next key (`no match for x`).
    message: Option<String>,
}

impl Reader {
    pub fn new(title: &str, markdown: &str) -> Reader {
        let lines: Vec<Vec<char>> = markdown.split('\n').map(|l| l.chars().collect()).collect();
        let classes = match syntax::syntax_for("reply.md") {
            Some(md) => {
                let mut h = Highlighter::new(md);
                lines
                    .iter()
                    .map(|l| h.line(&l.iter().collect::<String>()))
                    .collect()
            }
            None => lines.iter().map(|l| vec![Class::Plain; l.len()]).collect(),
        };
        Reader::with(title, lines, classes, "markdown · / search · v/V select · y copy · q close")
    }

    /// Plain text (a log), with `hint` for the keys in the title line.
    pub fn plain(title: &str, text: &str, hint: &str) -> Reader {
        let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        let classes = lines.iter().map(|l| vec![Class::Plain; l.len()]).collect();
        Reader::with(title, lines, classes, hint)
    }

    /// Puts the cursor on the last line, the end of the text in view.
    pub fn to_end(&mut self) {
        self.cur = (self.lines.len().saturating_sub(1), 0);
        self.end = true;
    }

    fn with(title: &str, lines: Vec<Vec<char>>, classes: Vec<Vec<Class>>, hint: &str) -> Reader {
        Reader {
            hint: hint.to_string(),
            end: false,
            typing: None,
            pattern: None,
            message: None,
            title: title.to_string(),
            lines,
            classes,
            cur: (0, 0),
            want: 0,
            mode: Mode::Normal,
            anchor: (0, 0),
            top: 0,
            height: 1,
            width: 1,
            count: 0,
            pending: None,
            copy: None,
        }
    }

    /// What `y` copied, once.
    pub fn take_copy(&mut self) -> Option<String> {
        self.copy.take()
    }

    pub fn wheel(&mut self, down: bool) {
        self.scroll(if down { 3 } else { -3 });
    }

    /// A search is being typed: every key is the search's.
    pub fn is_typing(&self) -> bool {
        self.typing.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> ReaderAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.message = None;
        if self.typing.is_some() {
            self.search_key(key);
            return ReaderAction::Redraw;
        }
        if let Some(p) = self.pending.take() {
            return self.pending_key(p, key);
        }
        if let KeyCode::Char(c @ '0'..='9') = key.code
            && !ctrl
            && (c != '0' || self.count > 0)
        {
            self.count = self
                .count
                .saturating_mul(10)
                .saturating_add(c as usize - '0' as usize);
            return ReaderAction::Redraw;
        }
        let raw = std::mem::take(&mut self.count);
        let n = raw.max(1);
        let half = (self.height / 2).max(1) as isize;
        let page = self.height.saturating_sub(2).max(1) as isize;
        match key.code {
            KeyCode::Char('c') if ctrl => return ReaderAction::Close,
            KeyCode::Char('d') if ctrl => self.scroll_with_cursor(half),
            KeyCode::Char('u') if ctrl => self.scroll_with_cursor(-half),
            KeyCode::Char('f') if ctrl => self.scroll_with_cursor(page),
            KeyCode::Char('b') if ctrl => self.scroll_with_cursor(-page),
            KeyCode::PageDown => self.scroll_with_cursor(page),
            KeyCode::PageUp => self.scroll_with_cursor(-page),
            KeyCode::Char('e') if ctrl => self.scroll(n as isize),
            KeyCode::Char('y') if ctrl => self.scroll(-(n as isize)),
            _ if ctrl => {}
            KeyCode::Esc if self.mode != Mode::Normal => self.mode = Mode::Normal,
            KeyCode::Esc | KeyCode::Char('q') => return ReaderAction::Close,
            KeyCode::Char('v') => self.toggle(Mode::Visual),
            KeyCode::Char('V') => self.toggle(Mode::VisualLine),
            KeyCode::Char('o') if self.mode != Mode::Normal => {
                std::mem::swap(&mut self.cur, &mut self.anchor);
                self.want = self.cur.1;
            }
            KeyCode::Char('y') if self.mode != Mode::Normal => {
                let (a, b) = self.ordered(self.anchor, self.cur);
                let reach = if self.mode == Mode::VisualLine {
                    Reach::Lines
                } else {
                    Reach::Inclusive
                };
                self.yank(a, b, reach);
                self.mode = Mode::Normal;
                self.go(a, false);
            }
            KeyCode::Char('Y') => {
                let last = (self.cur.0 + n - 1).min(self.lines.len() - 1);
                self.yank(self.cur, (last, 0), Reach::Lines);
            }
            KeyCode::Char(c @ ('g' | 'y')) => {
                self.pending = Some(c);
                self.count = raw;
            }
            KeyCode::Char('/') => self.typing = Some(String::new()),
            KeyCode::Char(c @ ('n' | 'N')) => {
                if let Some(to) = self.find(c == 'n', n) {
                    self.go(to, false);
                }
            }
            _ => {
                if let Some((to, _)) = self.motion(key, n, raw > 0) {
                    self.go(to, is_vertical(key.code));
                }
            }
        }
        ReaderAction::Redraw
    }

    /// The key after `g` or `y` (`Y` stands for `yg`).
    fn pending_key(&mut self, p: char, key: KeyEvent) -> ReaderAction {
        let raw = std::mem::take(&mut self.count);
        let n = raw.max(1);
        match (p, key.code) {
            ('g', KeyCode::Char('g')) => self.go((n.min(self.lines.len()) - 1, 0), false),
            ('y', KeyCode::Char('y')) => {
                let last = (self.cur.0 + n - 1).min(self.lines.len() - 1);
                self.yank(self.cur, (last, 0), Reach::Lines);
            }
            ('y', KeyCode::Char('g')) => {
                self.pending = Some('Y');
                self.count = raw;
            }
            ('Y', KeyCode::Char('g')) => {
                let to = (n.min(self.lines.len()) - 1, 0);
                let (a, b) = self.ordered(self.cur, to);
                self.yank(a, b, Reach::Lines);
                self.cur.0 = a.0;
            }
            ('y', _) => {
                if let Some((to, reach)) = self.motion(key, n, raw > 0) {
                    let (a, b) = self.ordered(self.cur, to);
                    self.yank(a, b, reach);
                    // The cursor goes to the start of what was copied, as in vim.
                    match reach {
                        Reach::Lines => self.go((a.0, self.cur.1), true),
                        _ => self.go(a, false),
                    }
                }
            }
            _ => {}
        }
        ReaderAction::Redraw
    }

    /// A key while `/` is typed: Enter searches from the cursor, Esc gives up.
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
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.typing = None;
            }
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                let text = self.typing.take().unwrap_or_default();
                if !text.is_empty() {
                    self.pattern = Some(text);
                }
                if let Some(to) = self.find(true, 1) {
                    self.go(to, false);
                }
            }
            _ => {}
        }
    }

    /// The pattern as searched and whether it is exact: smart case, an upper-case letter makes
    /// it exact.
    fn needle(&self) -> Option<(Vec<char>, bool)> {
        let pattern = self.pattern.as_ref().filter(|p| !p.is_empty())?;
        let exact = pattern.chars().any(char::is_uppercase);
        let chars = if exact {
            pattern.chars().collect()
        } else {
            pattern.to_lowercase().chars().collect()
        };
        Some((chars, exact))
    }

    /// Where the matches in line `row` start, by char.
    fn matches_in(&self, row: usize, needle: &[char], exact: bool) -> Vec<usize> {
        let line: Vec<char> = if exact {
            self.lines[row].clone()
        } else {
            // Lower-casing one char at a time keeps the positions (a char that lower-cases to
            // more than one keeps its first).
            self.lines[row]
                .iter()
                .map(|c| c.to_lowercase().next().unwrap_or(*c))
                .collect()
        };
        if needle.is_empty() || line.len() < needle.len() {
            return Vec::new();
        }
        (0..=line.len() - needle.len())
            .filter(|&i| line[i..i + needle.len()] == *needle)
            .collect()
    }

    /// The `n`-th match after (`forward`) or before the cursor, round the ends; None (and the
    /// title says why) when there is none.
    fn find(&mut self, forward: bool, n: usize) -> Option<Pos> {
        let Some((needle, exact)) = self.needle() else {
            self.message = Some("no search yet: / to search".into());
            return None;
        };
        let all: Vec<Pos> = (0..self.lines.len())
            .flat_map(|r| {
                self.matches_in(r, &needle, exact)
                    .into_iter()
                    .map(move |c| (r, c))
            })
            .collect();
        if all.is_empty() {
            let pattern = self.pattern.clone().unwrap_or_default();
            self.message = Some(format!("no match for {pattern}"));
            return None;
        }
        let mut at = self.cur;
        let mut wrapped = false;
        for _ in 0..n {
            let next = if forward {
                all.iter().find(|&&p| p > at).copied().or_else(|| {
                    wrapped = true;
                    all.first().copied()
                })
            } else {
                all.iter().rev().find(|&&p| p < at).copied().or_else(|| {
                    wrapped = true;
                    all.last().copied()
                })
            };
            at = next?;
        }
        if wrapped {
            self.message = Some(
                if forward { "search went past the end, on from the top" } else { "search went past the top, on from the end" }
                    .into(),
            );
        }
        Some(at)
    }

    fn toggle(&mut self, mode: Mode) {
        if self.mode == mode {
            self.mode = Mode::Normal;
        } else {
            if self.mode == Mode::Normal {
                self.anchor = self.cur;
            }
            self.mode = mode;
        }
    }

    /// Where a motion key goes from the cursor, `n` times, and what it reaches for `y`.
    /// `counted`: a count was typed (`5G` vs `G`).
    fn motion(&self, key: KeyEvent, n: usize, counted: bool) -> Option<(Pos, Reach)> {
        let last = self.lines.len() - 1;
        let (row, col) = self.cur;
        let repeat = |f: &dyn Fn(Pos) -> Pos| (0..n).fold(self.cur, |p, _| f(p));
        let at_want = |row: usize| (row, self.want.min(self.lines[row].len().saturating_sub(1)));
        Some(match key.code {
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                ((row, col.saturating_sub(n)), Reach::Exclusive)
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') => {
                ((row, col + n), Reach::Exclusive)
            }
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => {
                (at_want((row + n).min(last)), Reach::Lines)
            }
            KeyCode::Char('k') | KeyCode::Up => (at_want(row.saturating_sub(n)), Reach::Lines),
            KeyCode::Char('0') | KeyCode::Home => ((row, 0), Reach::Exclusive),
            KeyCode::Char('^') => {
                let first = self.lines[row].iter().position(|c| !c.is_whitespace());
                ((row, first.unwrap_or(0)), Reach::Exclusive)
            }
            KeyCode::Char('$') | KeyCode::End => {
                let row = (row + n - 1).min(last);
                ((row, usize::MAX), Reach::Inclusive)
            }
            KeyCode::Char('G') => {
                let to = if counted { n.min(last + 1) - 1 } else { last };
                ((to, 0), Reach::Lines)
            }
            KeyCode::Char('H') => ((self.top, 0), Reach::Lines),
            KeyCode::Char('L') => ((self.last_visible(), 0), Reach::Lines),
            KeyCode::Char('M') => (((self.top + self.last_visible()) / 2, 0), Reach::Lines),
            KeyCode::Char('w') => (repeat(&|p| self.word_start(p, false)), Reach::Exclusive),
            KeyCode::Char('W') => (repeat(&|p| self.word_start(p, true)), Reach::Exclusive),
            KeyCode::Char('e') => (repeat(&|p| self.word_end(p, false)), Reach::Inclusive),
            KeyCode::Char('E') => (repeat(&|p| self.word_end(p, true)), Reach::Inclusive),
            KeyCode::Char('b') => (repeat(&|p| self.word_back(p, false)), Reach::Exclusive),
            KeyCode::Char('B') => (repeat(&|p| self.word_back(p, true)), Reach::Exclusive),
            KeyCode::Char('}') => (repeat(&|p| self.paragraph(p, true)), Reach::Exclusive),
            KeyCode::Char('{') => (repeat(&|p| self.paragraph(p, false)), Reach::Exclusive),
            _ => return None,
        })
    }

    /// Moves the cursor; `vertical` (`j`, `k`) keeps the column it aims for.
    fn go(&mut self, to: Pos, vertical: bool) {
        self.cur = to;
        self.clamp();
        if to.1 == usize::MAX {
            self.want = usize::MAX;
        } else if !vertical {
            self.want = self.cur.1;
        }
    }

    /// The cursor on a char of its line (on the line's start when it is empty).
    fn clamp(&mut self) {
        self.cur.0 = self.cur.0.min(self.lines.len() - 1);
        self.cur.1 = self
            .cur
            .1
            .min(self.lines[self.cur.0].len().saturating_sub(1));
    }

    fn ordered(&self, a: Pos, b: Pos) -> (Pos, Pos) {
        if a <= b { (a, b) } else { (b, a) }
    }

    fn yank(&mut self, a: Pos, b: Pos, reach: Reach) {
        let text = match reach {
            Reach::Lines => self.lines[a.0..=b.0]
                .iter()
                .map(|l| l.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("\n"),
            Reach::Inclusive => self.chars_between(a, b),
            // Nothing between them: nothing to copy.
            Reach::Exclusive if a == b => return,
            // Up to the start of a line: up to the end of the one before.
            Reach::Exclusive if b.1 == 0 => {
                self.chars_between(a, (b.0 - 1, self.lines[b.0 - 1].len()))
            }
            Reach::Exclusive => self.chars_between(a, (b.0, b.1 - 1)),
        };
        self.copy = Some(text);
    }

    /// The text from `a` to `b`, both included (`b`'s column may be past its line: to its
    /// end).
    fn chars_between(&self, a: Pos, b: Pos) -> String {
        let mut out = String::new();
        for row in a.0..=b.0 {
            let line = &self.lines[row];
            let from = if row == a.0 { a.1.min(line.len()) } else { 0 };
            let to = if row == b.0 {
                b.1.saturating_add(1).min(line.len())
            } else {
                line.len()
            };
            if row > a.0 {
                out.push('\n');
            }
            out.extend(&line[from..to.max(from)]);
        }
        out
    }

    // ---- words and paragraphs ----

    /// The char at a place; the end of a line (and an empty line) reads as a newline.
    fn char_at(&self, (row, col): Pos) -> char {
        self.lines[row].get(col).copied().unwrap_or('\n')
    }

    fn next(&self, (row, col): Pos) -> Option<Pos> {
        if col + 1 < self.lines[row].len() {
            Some((row, col + 1))
        } else if row + 1 < self.lines.len() {
            Some((row + 1, 0))
        } else {
            None
        }
    }

    fn prev(&self, (row, col): Pos) -> Option<Pos> {
        if col > 0 {
            Some((row, col - 1))
        } else if row > 0 {
            Some((row - 1, self.lines[row - 1].len().saturating_sub(1)))
        } else {
            None
        }
    }

    /// 0 for blanks, 1 for word chars, 2 for punctuation; with `big`, anything not blank is 1.
    fn class(c: char, big: bool) -> u8 {
        if c.is_whitespace() {
            0
        } else if big || c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    }

    fn is_empty(&self, row: usize) -> bool {
        self.lines[row].is_empty()
    }

    /// `w`: the start of the next word; an empty line counts as one.
    fn word_start(&self, from: Pos, big: bool) -> Pos {
        let mut p = from;
        let start = Self::class(self.char_at(p), big);
        if start != 0 {
            loop {
                let Some(n) = self.next(p) else {
                    return (p.0, self.lines[p.0].len().saturating_sub(1));
                };
                let crossed = n.0 != p.0;
                p = n;
                if crossed || Self::class(self.char_at(p), big) != start {
                    break;
                }
            }
        }
        while Self::class(self.char_at(p), big) == 0 {
            if self.is_empty(p.0) && p != from {
                break;
            }
            match self.next(p) {
                Some(n) => p = n,
                None => break,
            }
        }
        p
    }

    /// `e`: the end of this word, or of the next one when at the end already.
    fn word_end(&self, from: Pos, big: bool) -> Pos {
        let Some(mut p) = self.next(from) else {
            return from;
        };
        while Self::class(self.char_at(p), big) == 0 {
            match self.next(p) {
                Some(n) => p = n,
                None => return p,
            }
        }
        let class = Self::class(self.char_at(p), big);
        while let Some(n) = self.next(p) {
            if n.0 != p.0 || Self::class(self.char_at(n), big) != class {
                break;
            }
            p = n;
        }
        p
    }

    /// `b`: the start of this word, or of the one before when at its start already.
    fn word_back(&self, from: Pos, big: bool) -> Pos {
        let Some(mut p) = self.prev(from) else {
            return from;
        };
        while Self::class(self.char_at(p), big) == 0 && !(self.is_empty(p.0) && p.0 != from.0) {
            match self.prev(p) {
                Some(n) => p = n,
                None => return p,
            }
        }
        let class = Self::class(self.char_at(p), big);
        while let Some(n) = self.prev(p) {
            if n.0 != p.0 || Self::class(self.char_at(n), big) != class {
                break;
            }
            p = n;
        }
        p
    }

    /// `}` / `{`: the next empty line after (before) the text, or the last (first) line.
    fn paragraph(&self, (row, _): Pos, down: bool) -> Pos {
        let n = self.lines.len();
        let mut r = row;
        // Off the empty lines first, then on to the next one.
        while self.is_empty(r) {
            let next = if down { r + 1 } else { r.wrapping_sub(1) };
            if next >= n {
                break;
            }
            r = next;
        }
        loop {
            let next = if down { r + 1 } else { r.wrapping_sub(1) };
            if next >= n {
                return if down { (n - 1, usize::MAX) } else { (0, 0) };
            }
            r = next;
            if self.is_empty(r) {
                return (r, 0);
            }
        }
    }

    // ---- view ----

    /// Rows a line takes, wrapped at the text's width.
    fn rows(&self, row: usize) -> usize {
        let cells: usize = self.lines[row].iter().map(|c| c.width().unwrap_or(0)).sum();
        cells.div_ceil(self.width.max(1)).max(1)
    }

    /// The last line that starts in view.
    fn last_visible(&self) -> usize {
        let mut used = 0;
        let mut row = self.top;
        while row < self.lines.len() {
            used += self.rows(row);
            if used > self.height && row > self.top {
                return row - 1;
            }
            row += 1;
        }
        self.lines.len() - 1
    }

    /// Ctrl-E / Ctrl-Y, the wheel: the view moves, the cursor only as far as it has to.
    fn scroll(&mut self, by: isize) {
        let max = self.lines.len() - 1;
        self.top = self.top.saturating_add_signed(by).min(max);
        let last = self.last_visible();
        let row = self.cur.0.clamp(self.top, last);
        if row != self.cur.0 {
            self.cur.0 = row;
            self.cur.1 = self.want;
            self.clamp();
        }
    }

    /// Ctrl-D / Ctrl-U and the pages: view and cursor move together.
    fn scroll_with_cursor(&mut self, by: isize) {
        let max = self.lines.len() - 1;
        self.top = self.top.saturating_add_signed(by).min(max);
        self.cur.0 = self.cur.0.saturating_add_signed(by).min(max);
        self.cur.1 = self.want;
        self.clamp();
    }

    /// Scrolls just enough for the cursor's line to be in view.
    fn reveal(&mut self) {
        if self.cur.0 < self.top {
            self.top = self.cur.0;
        }
        while self.top < self.cur.0 && self.last_visible() < self.cur.0 {
            self.top += 1;
        }
    }

    fn selected(&self, p: Pos) -> bool {
        let (a, b) = self.ordered(self.anchor, self.cur);
        match self.mode {
            Mode::Normal => false,
            Mode::VisualLine => (a.0..=b.0).contains(&p.0),
            Mode::Visual => a <= p && p <= b,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, p: &Palette) {
        if area.height < 2 || area.width == 0 {
            return;
        }
        self.width = usize::from(area.width);
        self.height = usize::from(area.height) - 1;
        if std::mem::take(&mut self.end) {
            self.top = self.lines.len().saturating_sub(self.height);
        }
        self.reveal();
        let muted = Style::new().fg(p.muted);
        let colors = syntax::block_colors(p);
        let selection = match crate::chat::mix(p.bg, p.accent, 0.35) {
            Some(c) => Style::new().bg(c),
            None => Style::new().add_modifier(Modifier::UNDERLINED),
        };
        let cursor = Style::new().add_modifier(Modifier::REVERSED);
        let found = match crate::chat::mix(p.bg, p.warning, 0.4) {
            Some(c) => Style::new().bg(c),
            None => Style::new().add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
        };
        let needle = self.needle();

        let mut out = vec![self.title_line(muted, p)];
        let mut row = self.top;
        while out.len() <= self.height && row < self.lines.len() {
            let line = &self.lines[row];
            // The chars of this line that are part of a match of the search.
            let marked: Vec<bool> = match &needle {
                Some((n, exact)) => {
                    let mut marked = vec![false; line.len()];
                    for at in self.matches_in(row, n, *exact) {
                        marked[at..at + n.len()].iter_mut().for_each(|m| *m = true);
                    }
                    marked
                }
                None => Vec::new(),
            };
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut used = 0;
            let push =
                |spans: &mut Vec<Span<'static>>, c: char, style: Style| match spans.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(c),
                    _ => spans.push(Span::styled(c.to_string(), style)),
                };
            let style_at = |col: usize| {
                let class = self.classes[row].get(col).copied().unwrap_or_default();
                let mut style = colors[class as usize];
                if marked.get(col).copied().unwrap_or(false) {
                    style = style.patch(found);
                }
                if self.selected((row, col)) {
                    style = style.patch(selection);
                }
                if self.cur == (row, col) {
                    style = style.patch(cursor);
                }
                style
            };
            for (col, &c) in line.iter().enumerate() {
                let w = c.width().unwrap_or(0);
                if used + w > self.width && used > 0 {
                    out.push(Line::from(std::mem::take(&mut spans)));
                    used = 0;
                    if out.len() > self.height {
                        break;
                    }
                }
                push(&mut spans, c, style_at(col));
                used += w;
            }
            // The cursor on an empty line, and a selected line break.
            let filler = line.is_empty()
                || (self.mode == Mode::VisualLine && self.selected((row, line.len())));
            if filler && used < self.width {
                push(&mut spans, ' ', style_at(line.len()));
            }
            if out.len() <= self.height {
                out.push(Line::from(spans));
            }
            row += 1;
        }
        out.truncate(self.height + 1);
        frame.render_widget(Paragraph::new(out), area);
    }

    /// `reply · markdown`, the keys, and the mode or where the cursor is on the right.
    fn title_line(&self, muted: Style, p: &Palette) -> Line<'static> {
        let w = self.width;
        // The search being typed has the line to itself.
        if let Some(text) = &self.typing {
            return crate::util::fit_line(
                Line::from(vec![
                    Span::styled("/", Style::new().fg(p.accent)),
                    Span::raw(text.clone()),
                    Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
                    Span::styled("  enter search · esc cancel", muted),
                ]),
                w,
            );
        }
        let right = match self.mode {
            Mode::Visual => "-- VISUAL --".to_string(),
            Mode::VisualLine => "-- VISUAL LINE --".to_string(),
            Mode::Normal => format!("{}/{}", self.cur.0 + 1, self.lines.len()),
        };
        // A word about the last key (no match…) in place of the keys, until the next one.
        let hint = match &self.message {
            Some(m) => format!("  {m}"),
            None if self.pattern.is_some() => format!("  {} · n N next", self.hint),
            None => format!("  {}", self.hint),
        };
        // The keys give way first: the title keeps a little, where the cursor is stays.
        let keep = width(&self.title).min(12);
        let hint = crate::util::truncate(&hint, w.saturating_sub(width(&right) + 2 + keep));
        let room = w.saturating_sub(width(&right) + 2 + width(&hint)).max(1);
        let title = truncate_start(&self.title, room);
        let said = if self.message.is_some() {
            Style::new().fg(p.warning)
        } else {
            muted
        };
        let mut spans = vec![
            Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(hint, said),
        ];
        let used: usize = spans.iter().map(|s| width(&s.content)).sum();
        if used + 2 + width(&right) <= w {
            spans.push(Span::raw(" ".repeat(w - used - width(&right))));
            spans.push(Span::styled(right, muted));
        }
        crate::util::fit_line(Line::from(spans), w)
    }
}

fn is_vertical(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down | KeyCode::Enter
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# Title\n\nSome words here, and more.\nnext line\n\nlast para";

    fn reader() -> Reader {
        let mut r = Reader::new("reply", TEXT);
        r.height = 10;
        r.width = 40;
        r
    }

    fn keys(r: &mut Reader, keys: &str) {
        for c in keys.chars() {
            r.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn esc(r: &mut Reader) {
        r.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }

    fn enter(r: &mut Reader) {
        r.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn slash_searches_and_n_and_shift_n_go_between_the_matches_round_the_ends() {
        let mut r = Reader::plain("log", "ok\nerror: one\nfine\nERROR two\nlast", "");
        keys(&mut r, "/error");
        assert!(r.is_typing());
        // Typed keys are the search's, not motions.
        assert_eq!(r.cur, (0, 0));
        enter(&mut r);
        assert!(!r.is_typing());
        assert_eq!(r.cur, (1, 0));
        // No upper-case letter: any case.
        keys(&mut r, "n");
        assert_eq!(r.cur, (3, 0));
        keys(&mut r, "n");
        assert_eq!(r.cur, (1, 0), "round to the top");
        assert!(r.message.as_deref().is_some_and(|m| m.contains("from the top")));
        keys(&mut r, "N");
        assert_eq!(r.cur, (3, 0), "back round to the end");
        // An upper-case letter makes it exact; a count jumps that many.
        keys(&mut r, "gg/ERROR");
        enter(&mut r);
        assert_eq!(r.cur, (3, 0));
        // From the top: the o of `error`, then of `one`, `ERROR` (any case) and `two`.
        keys(&mut r, "gg/o");
        enter(&mut r);
        assert_eq!(r.cur, (1, 3));
        keys(&mut r, "2n");
        assert_eq!(r.cur, (3, 3));
    }

    #[test]
    fn a_search_with_no_match_says_so_and_esc_gives_up_typing() {
        let mut r = Reader::plain("log", "alpha\nbeta", "");
        keys(&mut r, "j/zzz");
        enter(&mut r);
        assert_eq!(r.cur, (1, 0), "stays");
        assert!(r.message.as_deref().is_some_and(|m| m.contains("no match for zzz")));
        keys(&mut r, "/al");
        esc(&mut r);
        assert!(!r.is_typing());
        assert_eq!(r.pattern.as_deref(), Some("zzz"), "the last search is kept");
        // n before any search says how to start one.
        let mut r = Reader::plain("log", "x", "");
        keys(&mut r, "n");
        assert!(r.message.is_some());
    }

    #[test]
    fn moves_by_lines_words_and_paragraphs() {
        let mut r = reader();
        keys(&mut r, "jj");
        assert_eq!(r.cur, (2, 0));
        keys(&mut r, "w");
        assert_eq!(r.cur, (2, 5));
        keys(&mut r, "e");
        assert_eq!(r.cur, (2, 9));
        keys(&mut r, "w");
        assert_eq!(r.cur, (2, 11), "on to the next word");
        keys(&mut r, "w");
        assert_eq!(r.cur, (2, 15), "punctuation is a word of its own");
        keys(&mut r, "b");
        assert_eq!(r.cur, (2, 11));
        keys(&mut r, "$");
        assert_eq!(r.cur, (2, 25));
        keys(&mut r, "j");
        assert_eq!(r.cur, (3, 8), "$ sticks to the ends of lines");
        keys(&mut r, "0}");
        assert_eq!(r.cur, (4, 0));
        keys(&mut r, "{");
        assert_eq!(r.cur, (1, 0));
        keys(&mut r, "G");
        assert_eq!(r.cur, (5, 0));
        keys(&mut r, "gg");
        assert_eq!(r.cur, (0, 0));
        keys(&mut r, "3G");
        assert_eq!(r.cur, (2, 0));
        keys(&mut r, "2w");
        assert_eq!(r.cur, (2, 11));
    }

    #[test]
    fn yy_and_y_motions_copy() {
        let mut r = reader();
        keys(&mut r, "jjyy");
        assert_eq!(r.take_copy().as_deref(), Some("Some words here, and more."));
        keys(&mut r, "2yy");
        assert_eq!(
            r.take_copy().as_deref(),
            Some("Some words here, and more.\nnext line")
        );
        keys(&mut r, "yw");
        assert_eq!(r.take_copy().as_deref(), Some("Some "));
        keys(&mut r, "ye");
        assert_eq!(r.take_copy().as_deref(), Some("Some"));
        keys(&mut r, "wy$");
        assert_eq!(r.take_copy().as_deref(), Some("words here, and more."));
        keys(&mut r, "ygg");
        assert_eq!(
            r.take_copy().as_deref(),
            Some("# Title\n\nSome words here, and more.")
        );
        assert_eq!(r.cur.0, 0);
    }

    #[test]
    fn visual_modes_select_and_y_copies_them() {
        let mut r = reader();
        keys(&mut r, "jjwvey");
        assert_eq!(r.take_copy().as_deref(), Some("words"));
        assert_eq!(r.mode, Mode::Normal);
        assert_eq!(r.cur, (2, 5), "back to the selection's start");
        keys(&mut r, "vjy");
        assert_eq!(
            r.take_copy().as_deref(),
            Some("words here, and more.\nnext l")
        );
        keys(&mut r, "Vjy");
        assert_eq!(
            r.take_copy().as_deref(),
            Some("Some words here, and more.\nnext line")
        );
        keys(&mut r, "v");
        esc(&mut r);
        assert_eq!(r.mode, Mode::Normal);
        assert!(matches!(
            r.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ReaderAction::Close
        ));
    }

    #[test]
    fn the_view_follows_the_cursor() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut r = Reader::new(
            "reply",
            &(0..30)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let mut t = Terminal::new(TestBackend::new(60, 6)).unwrap();
        keys(&mut r, "G");
        t.draw(|f| r.draw(f, f.area(), &Palette::default()))
            .unwrap();
        let buf = t.backend().buffer().clone();
        let row = |y: u16| {
            (0..60)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert_eq!(row(5).trim_end(), "line 29");
        assert_eq!(row(1).trim_end(), "line 25");
        assert!(row(0).contains("30/30"));
    }
}
