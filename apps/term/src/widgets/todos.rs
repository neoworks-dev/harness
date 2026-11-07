//! The todos panel: the session's shared todo list, which the user and every agent change. It
//! follows the newest `todos` event; what the keys do goes back to the daemon (`neo.todos`), and
//! the list here changes when that event comes.

use neo_protocol::{TodoItem, Todos};
use neo_renderer_ratatui::Palette;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::text::wrap;

/// What a key in the focused panel asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum TodoKey {
    Handled,
    /// Back to the editor.
    Leave,
    /// Write a new one in the editor (`a`).
    Add,
    /// `neo.todos` with these args: check off, open again or remove the selected one.
    Change(serde_json::Value),
}

/// A row the keys can be on: an item (its index) or the row the finished ones fold into.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Row {
    Item(usize),
    Done,
}

#[derive(Debug, Default)]
pub(crate) struct TodosState {
    items: Vec<TodoItem>,
    /// Position in [`TodosState::rows`].
    selected: usize,
    /// The finished items are shown under their row.
    show_done: bool,
}

impl TodosState {
    pub(crate) fn apply(&mut self, todos: &Todos) {
        self.items = todos.items.clone();
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
    }

    pub(crate) fn has_any(&self) -> bool {
        !self.items.is_empty()
    }

    /// The open items, then the row of the finished ones and, opened, those items.
    fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = (0..self.items.len())
            .filter(|&i| !self.items[i].done)
            .map(Row::Item)
            .collect();
        let done: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.items[i].done)
            .collect();
        if !done.is_empty() {
            rows.push(Row::Done);
            if self.show_done {
                rows.extend(done.into_iter().map(Row::Item));
            }
        }
        rows
    }

    /// One block per row, wrapped in `w` columns after its box.
    fn blocks(&self, w: usize, focused: bool, p: &Palette) -> Vec<Vec<Line<'static>>> {
        let done = self.items.iter().filter(|i| i.done).count();
        self.rows()
            .into_iter()
            .enumerate()
            .map(|(at, row)| {
                let selected = focused && at == self.selected;
                let i = match row {
                    Row::Item(i) => i,
                    Row::Done => {
                        let arrow = if self.show_done { "▾ " } else { "▸ " };
                        let mut style = Style::new().fg(p.muted);
                        if selected {
                            style = style.add_modifier(Modifier::REVERSED);
                        }
                        return vec![Line::from(vec![
                            Span::styled(arrow, Style::new().fg(p.success)),
                            Span::styled(format!("{done} done"), style),
                        ])];
                    }
                };
                let item = &self.items[i];
                let (mark, style) = if item.done {
                    (
                        "[x] ",
                        Style::new().fg(p.muted).add_modifier(Modifier::CROSSED_OUT),
                    )
                } else {
                    ("[ ] ", Style::new())
                };
                let style = if selected {
                    style.add_modifier(Modifier::REVERSED)
                } else {
                    style
                };
                let mut lines = Vec::new();
                for (row, text) in wrap(&item.text, w.saturating_sub(4).max(1))
                    .into_iter()
                    .enumerate()
                {
                    let lead = if row == 0 { mark } else { "    " };
                    let lead_style = if item.done {
                        Style::new().fg(p.success)
                    } else {
                        Style::new().fg(p.muted)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(lead, lead_style),
                        Span::styled(text, style),
                    ]));
                }
                lines
            })
            .collect()
    }

    /// Rows its items would like.
    pub(crate) fn natural_height(&self, w: usize, p: &Palette) -> usize {
        self.blocks(w, false, p).iter().map(Vec::len).sum()
    }

    /// `h` rows: the items from where the selection stays in sight.
    pub(crate) fn render(
        &self,
        w: usize,
        h: usize,
        focused: bool,
        p: &Palette,
    ) -> Vec<Line<'static>> {
        if self.items.is_empty() {
            return Vec::new();
        }
        let blocks = self.blocks(w, focused, p);
        // From the first block that keeps the selected one in view.
        let mut start = 0;
        let rows_to =
            |from: usize, to: usize| -> usize { blocks[from..=to].iter().map(Vec::len).sum() };
        while start < self.selected && rows_to(start, self.selected) > h {
            start += 1;
        }
        blocks[start..].iter().flatten().take(h).cloned().collect()
    }

    pub(crate) fn key(&mut self, key: KeyEvent) -> TodoKey {
        let rows = self.rows();
        let row = rows.get(self.selected).copied();
        let selected = match row {
            Some(Row::Item(i)) => self.items.get(i),
            _ => None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => TodoKey::Leave,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                TodoKey::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(rows.len().saturating_sub(1));
                TodoKey::Handled
            }
            KeyCode::Char('a') => TodoKey::Add,
            // The finished ones open and close.
            KeyCode::Char(' ') | KeyCode::Enter if row == Some(Row::Done) => {
                self.show_done = !self.show_done;
                TodoKey::Handled
            }
            KeyCode::Char(' ') | KeyCode::Enter => match selected {
                Some(item) => {
                    let action = if item.done { "undone" } else { "done" };
                    TodoKey::Change(serde_json::json!({ "action": action, "id": item.id }))
                }
                None => TodoKey::Handled,
            },
            KeyCode::Char('x') | KeyCode::Delete => match selected {
                Some(item) => {
                    TodoKey::Change(serde_json::json!({ "action": "remove", "id": item.id }))
                }
                None => TodoKey::Handled,
            },
            _ => TodoKey::Handled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_renderer_ratatui::plain_text;
    use serde_json::json;

    fn todos() -> Todos {
        serde_json::from_value(json!({"type": "todos", "items": [
            {"id": "t1", "text": "Make the header sticky", "done": true, "by": "user"},
            {"id": "t2", "text": "Check the login form on a phone", "done": false}
        ]}))
        .unwrap()
    }

    #[test]
    fn items_show_with_their_box_and_the_keys_change_them() {
        let mut t = TodosState::default();
        let p = Palette::default();
        assert!(t.render(30, 5, false, &p).is_empty());
        t.apply(&todos());
        // The open ones first; the finished ones fold into a row under them.
        assert_eq!(
            plain_text(&t.render(20, 5, true, &p)),
            ["[ ] Check the login", "    form on a phone", "▸ 1 done"]
        );
        let key = |c| KeyEvent::from(c);
        assert_eq!(
            t.key(key(KeyCode::Char(' '))),
            TodoKey::Change(json!({"action": "done", "id": "t2"}))
        );
        // On the fold, Enter opens it.
        t.key(key(KeyCode::Down));
        assert_eq!(t.key(key(KeyCode::Enter)), TodoKey::Handled);
        assert_eq!(
            plain_text(&t.render(20, 5, false, &p)),
            [
                "[ ] Check the login",
                "    form on a phone",
                "▾ 1 done",
                "[x] Make the header",
                "    sticky"
            ]
        );
        t.key(key(KeyCode::Down));
        assert_eq!(
            t.key(key(KeyCode::Char(' '))),
            TodoKey::Change(json!({"action": "undone", "id": "t1"}))
        );
        assert_eq!(
            t.key(key(KeyCode::Char('x'))),
            TodoKey::Change(json!({"action": "remove", "id": "t1"}))
        );
        assert_eq!(t.key(key(KeyCode::Char('a'))), TodoKey::Add);
        // Two rows: the selected one stays in sight.
        assert_eq!(
            plain_text(&t.render(20, 2, true, &p)),
            ["[x] Make the header", "    sticky"]
        );
    }
}
