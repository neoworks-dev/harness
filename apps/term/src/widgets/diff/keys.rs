use super::*;

impl DiffView {
    /// Moves the cursor, selects, comments, searches, asks for the editor; true when the key
    /// closes the overlay (Esc outside visual mode, q). Vim's keys: h/j/k/l, w/b/e, 0/^/$,
    /// gg/G, H/M/L; a count before a key repeats it (`5j`) or names a line (`120G`); Ctrl-D/U
    /// half a page, Ctrl-F/B and PgUp/PgDn a page, Ctrl-E/Y scroll a line; `/` searches (smart
    /// case), `n` / `N` go to the next / previous match (without a search, `n` / `p` go between
    /// hunks, as `}` / `{` always do); `v` and `V` select characters and lines; `c` comments on the
    /// line or the selection (on a commented line: edits the comment). The leader (`\` or
    /// Space) then `w` wraps or cuts long lines, `e` opens the file in the editor at the cursor.
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
            (KeyCode::Char('c'), false) => self.start_comment(),
            (KeyCode::Char('/'), false) => self.typing = Some(String::new()),
            (KeyCode::Char('s'), false) => self.force = Some(!split),
            (KeyCode::Char('f'), false) => self.toggle_full(),
            _ => {}
        }
        self.follow();
        false
    }

    /// Puts the cursor on `row`, at the column it keeps going back to.
    pub(super) fn set_row(&mut self, row: usize, split: bool) {
        let row = row.min(self.len(split).saturating_sub(1));
        self.cursor.set(row);
        self.col = self.want.min(self.last_col(row, split));
    }

    /// The cursor on row `at` (a search's match), at the match.
    pub(super) fn jump(&mut self, at: usize, split: bool) {
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
    pub(super) fn scroll_only(&mut self, rows: isize) {
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
    pub(super) fn last_visible(&self) -> usize {
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
    pub(super) fn follow(&mut self) {
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
    pub(super) fn top_for(&self, cur: usize, split: bool, rows: usize) -> usize {
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
    pub(super) fn row_nos(&self, i: usize, split: bool) -> (Option<usize>, Option<usize>) {
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
    pub(super) fn row_body(&self, i: usize, split: bool) -> String {
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
    pub(super) fn last_col(&self, i: usize, split: bool) -> usize {
        self.row_body(i, split).chars().count().saturating_sub(1)
    }

    /// Row `row` of one mode in the other: the row of the same line, else the start of its
    /// hunk.
    pub(super) fn convert_row(&self, row: usize, from_split: bool) -> usize {
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
    pub(super) fn word(&mut self, motion: char, split: bool) {
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
}
