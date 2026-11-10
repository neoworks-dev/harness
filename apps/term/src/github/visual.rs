//! A cursor that walks the detail's text with vim's keys, the whole detail (description and
//! comments) being one buffer, and visual mode to select in it. Pure state over the lines the
//! detail shows; `view.rs` draws the cursor and the selection, `write.rs` adds a selection to
//! the comment being written.
//!
//! - **Move**: `h j k l` (and the arrows), `w b e`, `0 ^ $`, `gg G` and `5G`, Ctrl-D / Ctrl-U,
//!   PageUp / PageDown, all with a count (`3w`).
//! - **Select**: `v` characters, `V` lines, `o` swaps the ends, Esc leaves visual mode.
//! - **Add**: `a`, `y` or Enter with something selected hand the selection over to be quoted.
//!
//! Every other key is passed on, so the detail's own (`d`, `s`, `r`, Esc…) keep working.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a key did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Keyed {
    /// Not one of the cursor's keys.
    Pass,
    Handled,
    /// Add this text (the selection) to the message.
    Add(String),
}

type Pos = (usize, usize);

pub(super) struct Cursor {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// Where visual mode started.
    anchor: Option<Pos>,
    linewise: bool,
    /// A count being typed (`12j`).
    count: String,
    /// `g` is waiting for its second key.
    g: bool,
    /// A cursor key was used: the cursor is drawn from then on.
    pub active: bool,
}

fn class(c: char) -> u8 {
    if c == '\n' || c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

impl Cursor {
    pub fn new(lines: Vec<Vec<char>>) -> Cursor {
        let mut c = Cursor {
            lines: vec![Vec::new()],
            row: 0,
            col: 0,
            anchor: None,
            linewise: false,
            count: String::new(),
            g: false,
            active: false,
        };
        c.set_lines(lines);
        c
    }

    /// The text changed (a reload, another width): the cursor stays where it can.
    pub fn set_lines(&mut self, lines: Vec<Vec<char>>) {
        self.lines = if lines.is_empty() { vec![Vec::new()] } else { lines };
        self.clamp();
        if let Some(a) = &mut self.anchor {
            a.0 = a.0.min(self.lines.len() - 1);
            a.1 = a.1.min(self.lines[a.0].len().saturating_sub(1));
        }
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn col(&self) -> usize {
        self.col
    }

    /// Moves the cursor to a line (not into visual mode).
    pub fn go_to_row(&mut self, row: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.clamp();
        self.active = true;
    }

    /// Visual mode is on.
    pub fn visual(&self) -> bool {
        self.anchor.is_some()
    }

    #[cfg(test)]
    pub fn linewise(&self) -> bool {
        self.linewise
    }

    /// Whether the character at a position is part of the selection.
    pub fn selects(&self, r: usize, c: usize) -> bool {
        let Some((a, b)) = self.span() else {
            return false;
        };
        if self.linewise {
            r >= a.0 && r <= b.0
        } else {
            (r, c) >= a && (r, c) <= b
        }
    }

    /// The text at a position; the end of a line is a newline.
    fn at(&self, (r, c): Pos) -> char {
        self.lines[r].get(c).copied().unwrap_or('\n')
    }

    /// The next position, counting the end of each line as one.
    fn next(&self, (r, c): Pos) -> Option<Pos> {
        if c < self.lines[r].len() {
            Some((r, c + 1))
        } else if r + 1 < self.lines.len() {
            Some((r + 1, 0))
        } else {
            None
        }
    }

    fn prev(&self, (r, c): Pos) -> Option<Pos> {
        if c > 0 {
            Some((r, c - 1))
        } else if r > 0 {
            Some((r - 1, self.lines[r - 1].len()))
        } else {
            None
        }
    }

    fn set(&mut self, (r, c): Pos) {
        self.row = r;
        self.col = c;
        self.clamp();
    }

    /// The cursor sits on a character, not past the end of its line.
    fn clamp(&mut self) {
        self.row = self.row.min(self.lines.len() - 1);
        self.col = self.col.min(self.lines[self.row].len().saturating_sub(1));
    }

    fn word_forward(&mut self) {
        let mut p = (self.row, self.col);
        let start = class(self.at(p));
        if start != 0 {
            while let Some(n) = self.next(p) {
                p = n;
                if class(self.at(p)) != start {
                    break;
                }
            }
        }
        while class(self.at(p)) == 0 {
            match self.next(p) {
                Some(n) => p = n,
                None => break,
            }
        }
        self.set(p);
    }

    fn word_back(&mut self) {
        let Some(mut p) = self.prev((self.row, self.col)) else {
            return;
        };
        while class(self.at(p)) == 0 {
            match self.prev(p) {
                Some(n) => p = n,
                None => {
                    self.set(p);
                    return;
                }
            }
        }
        let cls = class(self.at(p));
        while let Some(n) = self.prev(p) {
            if class(self.at(n)) != cls {
                break;
            }
            p = n;
        }
        self.set(p);
    }

    fn word_end(&mut self) {
        let Some(mut p) = self.next((self.row, self.col)) else {
            return;
        };
        while class(self.at(p)) == 0 {
            match self.next(p) {
                Some(n) => p = n,
                None => break,
            }
        }
        let cls = class(self.at(p));
        while let Some(n) = self.next(p) {
            if class(self.at(n)) != cls {
                break;
            }
            p = n;
        }
        self.set(p);
    }

    /// The ends of the selection in order.
    fn span(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        let b = (self.row, self.col);
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// The selected text: whole lines, or the characters from end to end.
    pub fn selected(&self) -> Option<String> {
        let (a, b) = self.span()?;
        let mut out = String::new();
        for r in a.0..=b.0 {
            let line = &self.lines[r];
            let (from, to) = if self.linewise {
                (0, line.len())
            } else {
                (
                    if r == a.0 { a.1 } else { 0 },
                    if r == b.0 { b.1 + 1 } else { line.len() },
                )
            };
            out.extend(line.iter().take(to).skip(from));
            if r < b.0 {
                out.push('\n');
            }
        }
        Some(out)
    }

    /// `page` rows is what a screenful moves.
    pub fn key(&mut self, key: KeyEvent, page: usize) -> Keyed {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt || (ctrl && !matches!(key.code, KeyCode::Char('d' | 'u'))) {
            return self.pass();
        }
        let last = self.lines.len() - 1;
        // A count; `0` is a motion unless a count has begun.
        if !ctrl
            && let KeyCode::Char(d @ '0'..='9') = key.code
            && (d != '0' || !self.count.is_empty())
        {
            self.count.push(d);
            self.active = true;
            return Keyed::Handled;
        }
        let typed = std::mem::take(&mut self.count).parse::<usize>().ok();
        let n = typed.unwrap_or(1);
        if std::mem::take(&mut self.g) {
            if key.code == KeyCode::Char('g') {
                self.row = typed.map_or(0, |t| t.saturating_sub(1)).min(last);
                self.clamp();
                return Keyed::Handled;
            }
            return Keyed::Handled;
        }
        let half = (page / 2).max(1);
        let rows = |s: &mut Cursor, down: bool, by: usize| {
            s.row = if down {
                (s.row + by).min(last)
            } else {
                s.row.saturating_sub(by)
            };
            s.clamp();
        };
        match (ctrl, key.code) {
            (true, KeyCode::Char('d')) => rows(self, true, half * n),
            (true, KeyCode::Char('u')) => rows(self, false, half * n),
            (false, KeyCode::Char('h') | KeyCode::Left) => self.col = self.col.saturating_sub(n),
            (false, KeyCode::Char('l') | KeyCode::Right) => {
                self.col = (self.col + n).min(self.lines[self.row].len().saturating_sub(1));
            }
            (false, KeyCode::Char('j') | KeyCode::Down) => rows(self, true, n),
            (false, KeyCode::Char('k') | KeyCode::Up) => rows(self, false, n),
            (false, KeyCode::PageDown | KeyCode::Char(' ')) => rows(self, true, page * n),
            (false, KeyCode::PageUp) => rows(self, false, page * n),
            (false, KeyCode::Char('0') | KeyCode::Home) => self.col = 0,
            (false, KeyCode::Char('^')) => {
                self.col = self.lines[self.row]
                    .iter()
                    .position(|c| !c.is_whitespace())
                    .unwrap_or(0);
            }
            (false, KeyCode::Char('$') | KeyCode::End) => {
                self.col = self.lines[self.row].len().saturating_sub(1);
            }
            (false, KeyCode::Char('g')) => {
                self.g = true;
                self.count = typed.map(|t| t.to_string()).unwrap_or_default();
            }
            (false, KeyCode::Char('G')) => {
                self.row = typed.map_or(last, |t| t.saturating_sub(1).min(last));
                self.clamp();
            }
            (false, KeyCode::Char('w')) => (0..n).for_each(|_| self.word_forward()),
            (false, KeyCode::Char('b')) => (0..n).for_each(|_| self.word_back()),
            (false, KeyCode::Char('e')) => (0..n).for_each(|_| self.word_end()),
            (false, KeyCode::Char('v')) => self.toggle(false),
            (false, KeyCode::Char('V')) => self.toggle(true),
            (false, KeyCode::Char('o')) if self.anchor.is_some() => {
                if let Some(a) = self.anchor {
                    self.anchor = Some((self.row, self.col));
                    self.set(a);
                }
            }
            (false, KeyCode::Esc) if self.anchor.is_some() => self.anchor = None,
            (false, KeyCode::Char('a' | 'y') | KeyCode::Enter) if self.anchor.is_some() => {
                let text = self.selected().unwrap_or_default();
                self.anchor = None;
                return Keyed::Add(text);
            }
            _ => return self.pass(),
        }
        self.active = true;
        Keyed::Handled
    }

    fn pass(&mut self) -> Keyed {
        self.count.clear();
        self.g = false;
        Keyed::Pass
    }

    /// `v` or `V`: starts visual mode, switches between the two, or leaves it by the same key.
    fn toggle(&mut self, linewise: bool) {
        match self.anchor {
            Some(_) if self.linewise == linewise => self.anchor = None,
            Some(_) => self.linewise = linewise,
            None => {
                self.anchor = Some((self.row, self.col));
                self.linewise = linewise;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyEventKind, KeyEventState};

    const TEXT: &str = "The quick brown fox\njumps over the lazy dog.\n\nSecond paragraph here";

    fn cursor() -> Cursor {
        Cursor::new(TEXT.split('\n').map(|l| l.chars().collect()).collect())
    }

    fn press(c: &mut Cursor, keys: &str) -> Keyed {
        let mut out = Keyed::Handled;
        for ch in keys.chars() {
            let code = match ch {
                '\n' => KeyCode::Enter,
                '\u{1b}' => KeyCode::Esc,
                ch => KeyCode::Char(ch),
            };
            out = c.key(
                KeyEvent {
                    code,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                },
                10,
            );
        }
        out
    }

    fn added(out: Keyed) -> String {
        match out {
            Keyed::Add(text) => text,
            other => panic!("expected text to add, got {other:?}"),
        }
    }

    #[test]
    fn words_move_and_select_characters() {
        let mut c = cursor();
        // From `quick` to the end of `brown`.
        press(&mut c, "wve");
        press(&mut c, "e");
        assert_eq!(added(press(&mut c, "a")), "quick brown");
        // A count: two words from the start.
        let mut c = cursor();
        press(&mut c, "v2e");
        assert_eq!(added(press(&mut c, "y")), "The quick");
        // Back a word from the end of the line.
        let mut c = cursor();
        press(&mut c, "$bv$");
        assert_eq!(added(press(&mut c, "a")), "fox");
    }

    #[test]
    fn a_selection_runs_across_lines_and_either_way() {
        let mut c = cursor();
        press(&mut c, "wvj");
        assert_eq!(added(press(&mut c, "\n")), "quick brown fox\njumps");
        let mut c = cursor();
        press(&mut c, "jwwv");
        press(&mut c, "k");
        assert_eq!(added(press(&mut c, "a")), "rown fox\njumps over t");
    }

    #[test]
    fn visual_line_takes_whole_lines_and_o_swaps_the_ends() {
        let mut c = cursor();
        press(&mut c, "Vj");
        assert_eq!(added(press(&mut c, "a")), "The quick brown fox\njumps over the lazy dog.");
        let mut c = cursor();
        press(&mut c, "jvlo");
        press(&mut c, "k");
        assert_eq!(added(press(&mut c, "a")), "The quick brown fox\nju");
    }

    #[test]
    fn a_and_enter_only_add_in_visual_mode_otherwise_the_keys_pass() {
        let mut c = cursor();
        assert_eq!(press(&mut c, "a"), Keyed::Pass);
        assert_eq!(press(&mut c, "\n"), Keyed::Pass);
        assert_eq!(press(&mut c, "d"), Keyed::Pass);
        assert_eq!(press(&mut c, "\u{1b}"), Keyed::Pass);
        press(&mut c, "v");
        // Esc leaves visual mode first.
        assert_eq!(press(&mut c, "\u{1b}"), Keyed::Handled);
        assert!(!c.visual());
        assert_eq!(press(&mut c, "\u{1b}"), Keyed::Pass);
    }

    #[test]
    fn gg_and_g_jump_and_a_count_picks_the_line() {
        let mut c = cursor();
        press(&mut c, "G");
        assert_eq!(c.row(), 3);
        press(&mut c, "gg");
        assert_eq!(c.row(), 0);
        press(&mut c, "2G");
        assert_eq!(c.row(), 1);
        press(&mut c, "3gg");
        assert_eq!(c.row(), 2);
    }

    #[test]
    fn the_cursor_stays_on_a_character_and_pages_move_by_rows() {
        let mut c = cursor();
        press(&mut c, "$jj");
        assert_eq!((c.row(), c.col()), (2, 0));
        press(&mut c, "jll");
        assert_eq!((c.row(), c.col()), (3, 2));
        press(&mut c, "$");
        assert_eq!(c.col(), "Second paragraph here".len() - 1);
        // A page is as many rows as it is told, and stops at the ends.
        press(&mut c, "gg ");
        assert_eq!(c.row(), 3);
    }

    #[test]
    fn what_is_selected_is_asked_per_character() {
        let mut c = cursor();
        press(&mut c, "wve");
        assert!(c.selects(0, 4) && c.selects(0, 8));
        assert!(!c.selects(0, 3) && !c.selects(0, 9) && !c.selects(1, 4));
        press(&mut c, "V");
        assert!(c.linewise() && c.selects(0, 0) && !c.selects(1, 0));
    }
}
