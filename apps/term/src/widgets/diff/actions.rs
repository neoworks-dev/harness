use super::*;

impl DiffView {
    /// `c`: writes a comment on the selection's lines, or the cursor's; on a line a
    /// comment is about (no selection), edits that one.
    pub(super) fn start_comment(&mut self) {
        // A file of git's here, or one from elsewhere whose maker takes the comments (a pull
        // request's, where they become review comments).
        if self.req.is_none() && self.remote.is_none() {
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
    pub(super) fn write_key(&mut self, key: KeyEvent) {
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
    pub(super) fn yank(&mut self, a: usize, b: usize, cols: Option<(usize, usize)>, split: bool) {
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

    /// How many comments are written on it.
    pub(crate) fn comment_count(&self) -> usize {
        self.comments.len()
    }

    /// A key started something the next keys belong to: a comment or a search being written,
    /// a selection, a count or the second key of `gg`, `yy` or the leader's.
    pub(crate) fn busy(&self) -> bool {
        self.writing.is_some()
            || self.typing.is_some()
            || self.visual.get().is_some()
            || self.pending.is_some()
            || self.count.is_some()
    }

    /// The comments written on this file before.
    pub(crate) fn with_comments(mut self, comments: Vec<Comment>) -> DiffView {
        self.comments = comments;
        self.comments_gen += 1;
        self
    }

    /// The path the view shows, as the prompt names it.
    pub(crate) fn rel(&self) -> Option<&str> {
        self.req.as_ref().map(|r| r.rel.as_str()).or(self.lang.as_deref())
    }

    /// A comment back on its lines (one that could not be posted), to be tried again.
    pub(crate) fn restore_comment(&mut self, c: Comment) {
        self.comments.push(c);
        self.comments_gen += 1;
    }

    /// Says something in the title line until the next key.
    pub(crate) fn tell(&mut self, text: &str) {
        self.message = Some(text.to_string());
    }

    /// The cursor and selection on row `i`, for drawing.
    pub(super) fn marks(&self, i: usize, split: bool) -> Marks {
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
    pub(super) fn blocks_at(&self, i: usize, split: bool) -> Vec<Vec<Line<'static>>> {
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
                let row = (0..self.len(split))
                    .find(|&r| {
                        let (o, n) = self.row_nos(r, split);
                        if old {
                            n.is_none() && o == Some(start)
                        } else {
                            n == Some(start)
                        }
                    })
                    // An old line that is in both versions (context).
                    .or_else(|| {
                        (0..self.len(split))
                            .find(|&r| old && self.row_nos(r, split).0 == Some(start))
                    });
                if let Some(row) = row {
                    cache.at.entry(row).or_default().push(block);
                }
            };
            let (_, muted) = colors;
            for r in &self.remarks {
                put(r.line, r.old, block(&r.head, &r.text, width, false, (muted, muted)));
            }
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
    pub(super) fn search_key(&mut self, key: KeyEvent) {
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
    pub(super) fn find(&mut self, from: usize, forward: bool, n: usize) -> usize {
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
    pub(super) fn row_text(&self, i: usize, split: bool) -> String {
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
    pub(super) fn row_of_line(&self, line: usize, split: bool) -> usize {
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
}
