//! Writing to GitHub from the detail, as the user `gh` is logged in as: a comment, a reaction,
//! closing an issue (completed, not planned or duplicate) and reopening, editing text. Each key
//! works on the block in focus ([`allowed`] says which where), and is confirmed in the panel at
//! the bottom; the work runs on a thread like everything else here.
//!
//! - The header: `x` close or reopen, `S` the title and `m` the other edits (see `edit.rs`).
//! - The description or a comment: `E` edit it, `+` react to it, `>` quote it.
//! - A review or a comment on the diff: `>` quote it.
//!
//! The input under the detail is always there: Tab or `c` types in it (Enter is a new line,
//! Ctrl-S posts), Tab or Esc goes back to reading and keeps what was typed.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::doc::Part;
use super::gh::{CloseReason, ReactOn};
use super::model::{Comment, Detail, Kind, REACTIONS, State};
use super::{Github, Mode, Msg};
use crate::editor::Editor;
use crate::style::Styles;
use crate::util::width;
use neo_github::fetch;

/// Most rows of the comment box on screen.
const BOX_ROWS: usize = 8;

/// What the keys of the detail are in the middle of.
pub(super) enum Writing {
    /// Typing the comment in `Github::draft`, which stays when the box is left with Esc.
    Comment,
    /// Choosing a reaction.
    React,
    /// Choosing why to close; with `dup`, typing the number of the issue it duplicates.
    Close { dup: Option<String> },
    /// Asking whether to reopen.
    Reopen,
    /// Editing the description or a comment in `Github::body_edit`; Esc drops the changes.
    Body(BodyOf),
    /// Showing the keys that edit (see `edit.rs`).
    Edit,
    /// Waiting for what a list needs from GitHub.
    Loading(super::edit::What),
    /// Choosing from a list.
    Pick(super::edit::Picker),
    /// Typing a value.
    Input(super::edit::Input),
}

/// Whose text is being edited.
#[derive(Clone, Copy)]
pub(super) enum BodyOf {
    /// The description of the issue or pull request.
    Item,
    /// A comment, by the number in its link.
    Comment(u64),
}

/// A key that types into an editor: true when it was one.
fn type_key(editor: &mut Editor, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter => editor.newline(),
        KeyCode::Backspace => editor.backspace(),
        KeyCode::Delete => editor.delete(),
        KeyCode::Left => editor.left(),
        KeyCode::Right => editor.right(),
        KeyCode::Up => {
            editor.up();
        }
        KeyCode::Down => {
            editor.down();
        }
        KeyCode::Home => editor.line_start(),
        KeyCode::End => editor.line_end(),
        KeyCode::Char(c) if !ctrl && !alt => editor.insert_char(c),
        _ => return false,
    }
    true
}

/// The lines of a box to type in: a title, then the rows around the cursor.
fn editor_panel(
    editor: &mut Editor,
    title: Vec<Span<'static>>,
    w: usize,
    h: usize,
    row: &dyn Fn(Vec<Span<'static>>) -> Line<'static>,
) -> Panel {
    editor.set_width(w.saturating_sub(2).max(1));
    let rows = editor.rows();
    let room = BOX_ROWS.min(h.saturating_sub(2)).max(1);
    let (cursor_row, cursor_col) = editor.cursor_pos();
    let top = (cursor_row + 1).saturating_sub(room);
    let mut lines = vec![row(title)];
    for r in rows.iter().skip(top).take(room) {
        lines.push(row(vec![Span::raw(editor.text()[r.start..r.end].to_string())]));
    }
    Panel {
        cursor: Some((1 + cursor_row - top, 1 + cursor_col)),
        lines,
    }
}

/// A panel line: one space in from the left, filled to `w` on `bg`.
pub(super) fn row_line(spans: Vec<Span<'static>>, w: usize, bg: Style) -> Line<'static> {
    let mut line = Line::from(spans);
    line.spans.insert(0, Span::raw(" "));
    let pad = w.saturating_sub(line.width());
    line.spans.push(Span::raw(" ".repeat(pad)));
    for span in &mut line.spans {
        span.style = bg.patch(span.style);
    }
    line
}

/// The bottom of the detail while writing: its lines and where the cursor is in them.
pub(super) struct Panel {
    pub lines: Vec<Line<'static>>,
    /// (line, column) of the cursor, when there is one to show.
    pub cursor: Option<(usize, usize)>,
}

impl Github {
    fn target(&self) -> Option<(Kind, u64)> {
        match &self.mode {
            Mode::Detail(v) => Some((v.kind, v.number)),
            Mode::List => None,
        }
    }

    fn shown(&self) -> Option<&Detail> {
        let (kind, number) = self.target()?;
        self.details.get(&(kind, number))?.detail.as_ref()
    }

    /// The block in focus; the header until the lines are built.
    pub(super) fn part(&self) -> Part {
        match &self.mode {
            Mode::Detail(v) => v.focus,
            Mode::List => Part::Head,
        }
    }

    /// The comment the keys act on, when one is in focus.
    fn picked(&self) -> Option<&Comment> {
        let Part::Comment(i) = self.part() else {
            return None;
        };
        self.shown()?.comments.get(i)
    }

    /// Whether a key may act on the block in focus. When it acts somewhere else, the footer
    /// says where and the key does nothing.
    pub(super) fn in_scope(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT)
        {
            return true;
        }
        let KeyCode::Char(c) = key.code else {
            return true;
        };
        let pr = matches!(&self.mode, Mode::Detail(v) if v.kind == Kind::Pr);
        match allowed(self.part(), c, pr) {
            Some(true) | None => true,
            Some(false) => {
                let place = match c {
                    'E' | '+' => "the description or a comment",
                    '>' => "the description, a comment or a review",
                    'd' => "the header or the files",
                    _ => "the header",
                };
                self.note = Some(format!("{c} works on {place}: j k to move there"));
                false
            }
        }
    }

    /// Whom the picked comment, or the item, is by.
    fn picked_author(&self) -> String {
        match self.picked() {
            Some(c) => format!("{}'s comment", c.author),
            None => self.shown().map_or(String::new(), |d| format!("#{}", d.number)),
        }
    }

    /// The keys of the detail that write or pick; true when it was one of them.
    pub(super) fn write_hotkey(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT)
        {
            return false;
        }
        let KeyCode::Char(c) = key.code else {
            return false;
        };
        if matches!(c, 'm' | 'A' | 'L' | 'M' | 'T' | 'F' | 'P' | 'R' | 'S') {
            return self.edit_hotkey(c);
        }
        if !matches!(c, '+' | 'x' | 'E' | '>') {
            return false;
        }
        if self.shown().is_none() {
            return true;
        }
        match c {
            '>' => {
                if let Some(text) = self.focused_text() {
                    self.add_to_message(&text);
                }
            }
            // The comment in focus, or the description.
            'E' => {
                let (target, text) = match self.picked() {
                    Some(c) if c.id == 0 => {
                        self.note = Some("this comment has no number to edit it by".into());
                        return true;
                    }
                    Some(c) => (BodyOf::Comment(c.id), c.body.clone()),
                    None => (
                        BodyOf::Item,
                        self.shown().map(|d| d.body.clone()).unwrap_or_default(),
                    ),
                };
                self.body_edit.set_text(&text);
                self.writing = Some(Writing::Body(target));
            }
            '+' => self.writing = Some(Writing::React),
            _ => {
                let open = self.shown().is_some_and(|d| d.state != State::Closed);
                let merged = self.shown().is_some_and(|d| d.state == State::Merged);
                if merged {
                    self.note = Some("a merged pull request can't be closed or reopened".into());
                } else if open {
                    self.writing = Some(Writing::Close { dup: None });
                } else {
                    self.writing = Some(Writing::Reopen);
                }
            }
        }
        true
    }

    /// The markdown of the block in focus, to quote whole.
    fn focused_text(&self) -> Option<String> {
        let d = self.shown()?;
        let text = match self.part() {
            Part::Description => &d.body,
            Part::Comment(i) => &d.comments.get(i)?.body,
            Part::Review(i) => &d.reviews.get(i)?.body,
            Part::DiffComment(i) => &d.review_comments.get(i)?.body,
            Part::Head | Part::Checks | Part::Files => return None,
        };
        (!text.trim().is_empty()).then(|| text.clone())
    }

    /// Adds text to the message being written, as a quote, and opens the box on it: after what
    /// is there already, a blank line between.
    pub(super) fn add_to_message(&mut self, text: &str) {
        // What the lines carry for looks: the space in from the edge, the bar of a comment.
        let cleaned: Vec<String> = text
            .lines()
            .map(|l| {
                let l = l.strip_prefix(' ').unwrap_or(l);
                let l = l.strip_prefix("▎ ").unwrap_or(l);
                l.trim_end().to_string()
            })
            .collect();
        let quoted = quote(cleaned.join("\n").trim_matches('\n'));
        let have = self.draft.text().to_string();
        let mut joined = have.trim_end_matches('\n').to_string();
        if !joined.is_empty() {
            joined.push_str("\n\n");
        }
        joined.push_str(&quoted);
        self.draft.set_text(&joined);
        self.writing = Some(Writing::Comment);
    }

    /// A key while writing.
    pub(super) fn writing_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let Some(writing) = self.writing.take() else {
            return;
        };
        if self.sending {
            // One thing at a time: it is on its way.
            self.writing = Some(writing);
            return;
        }
        match writing {
            Writing::Comment => {
                let editor = &mut self.draft;
                match key.code {
                    // Back to reading; what was typed waits in the draft.
                    KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => return,
                    KeyCode::Char('d') if ctrl => {
                        editor.clear();
                        return;
                    }
                    KeyCode::Char('s') if ctrl => {
                        let body = editor.text().trim().to_string();
                        if body.is_empty() {
                            self.note = Some("a comment needs some text".into());
                        } else if let Some((kind, number)) = self.target()
                            && let Some(repo) = self.repo_name().map(str::to_string)
                        {
                            // Out of the draft; back into it if posting fails.
                            self.draft.clear();
                            self.unsent = Some(body.clone());
                            self.send(
                                "comment posted",
                                "could not post the comment",
                                move |gh, cwd| fetch::comment(gh, cwd, kind, &repo, number, &body),
                            );
                            return;
                        }
                    }
                    KeyCode::Enter => editor.newline(),
                    KeyCode::Backspace => editor.backspace(),
                    KeyCode::Delete => editor.delete(),
                    KeyCode::Left => editor.left(),
                    KeyCode::Right => editor.right(),
                    KeyCode::Up => {
                        editor.up();
                    }
                    KeyCode::Down => {
                        editor.down();
                    }
                    KeyCode::Home => editor.line_start(),
                    KeyCode::End => editor.line_end(),
                    KeyCode::Char(c) if !ctrl && !alt => editor.insert_char(c),
                    _ => {}
                }
                self.writing = Some(Writing::Comment);
            }
            Writing::React => match key.code {
                KeyCode::Esc => {}
                KeyCode::Char(c) if ('1'..='8').contains(&c) => {
                    let (_, rest, _) = REACTIONS[c as usize - '1' as usize];
                    self.react(rest);
                }
                _ => self.writing = Some(Writing::React),
            },
            Writing::Close { dup: None } => {
                let Some((kind, number)) = self.target() else {
                    return;
                };
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Char('1') => self.close_item(kind, number, CloseReason::Completed),
                    KeyCode::Char('2') if kind == Kind::Issue => {
                        self.close_item(kind, number, CloseReason::NotPlanned);
                    }
                    KeyCode::Char('3') if kind == Kind::Issue => {
                        self.writing = Some(Writing::Close {
                            dup: Some(String::new()),
                        });
                    }
                    _ => self.writing = Some(Writing::Close { dup: None }),
                }
            }
            Writing::Close { dup: Some(mut typed) } => {
                let Some((kind, number)) = self.target() else {
                    return;
                };
                match key.code {
                    KeyCode::Esc => self.writing = Some(Writing::Close { dup: None }),
                    KeyCode::Backspace => {
                        typed.pop();
                        self.writing = Some(Writing::Close { dup: Some(typed) });
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        typed.push(c);
                        self.writing = Some(Writing::Close { dup: Some(typed) });
                    }
                    KeyCode::Enter => match typed.parse::<u64>() {
                        Ok(of) if of > 0 && of != number => {
                            self.close_item(kind, number, CloseReason::Duplicate(of));
                        }
                        _ => {
                            self.note = Some("the number of the issue it duplicates".into());
                            self.writing = Some(Writing::Close { dup: Some(typed) });
                        }
                    },
                    _ => self.writing = Some(Writing::Close { dup: Some(typed) }),
                }
            }
            Writing::Body(target) => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Char('s') if ctrl => {
                        let text = self.body_edit.text().to_string();
                        if matches!(target, BodyOf::Comment(_)) && text.trim().is_empty() {
                            self.note = Some("a comment needs some text".into());
                        } else if let Some((kind, number)) = self.target()
                            && let Some(repo) = self.repo_name().map(str::to_string)
                        {
                            match target {
                                BodyOf::Item => self.send(
                                    "description saved",
                                    "could not save the description",
                                    move |gh, cwd| fetch::edit_body(gh, cwd, kind, &repo, number, &text),
                                ),
                                BodyOf::Comment(id) => self.send(
                                    "comment saved",
                                    "could not save the comment",
                                    move |gh, cwd| fetch::edit_comment(gh, cwd, &repo, id, &text),
                                ),
                            }
                            return;
                        }
                    }
                    _ => {
                        type_key(&mut self.body_edit, key);
                    }
                }
                self.writing = Some(Writing::Body(target));
            }
            w @ (Writing::Edit | Writing::Loading(_) | Writing::Pick(_) | Writing::Input(_)) => {
                self.edit_key(w, key);
            }
            Writing::Reopen => match key.code {
                KeyCode::Esc => {}
                KeyCode::Char('1' | 'y') => {
                    if let Some((kind, number)) = self.target()
                        && let Some(repo) = self.repo_name().map(str::to_string)
                    {
                        self.send("reopened", "could not reopen", move |gh, cwd| {
                            fetch::reopen(gh, cwd, kind, &repo, number)
                        });
                    }
                }
                _ => self.writing = Some(Writing::Reopen),
            },
        }
    }

    /// Text pasted while a comment is typed goes into it.
    pub(super) fn paste_into_comment(&mut self, text: &str) -> bool {
        match &self.writing {
            Some(Writing::Comment) => {
                self.draft.insert_str(text);
                true
            }
            Some(Writing::Body(_)) => {
                self.body_edit.insert_str(text);
                true
            }
            _ => false,
        }
    }

    fn react(&mut self, rest: &'static str) {
        let Some((_, number)) = self.target() else {
            return;
        };
        let on = match self.picked() {
            Some(c) if c.id == 0 => {
                self.note = Some("this comment has no number to react to".into());
                return;
            }
            Some(c) => ReactOn::Comment(c.id),
            None => ReactOn::Item(number),
        };
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        // Giving the same reaction again takes it back.
        self.send_text("could not react", move |gh, cwd| {
            fetch::toggle_reaction(gh, cwd, &repo, on, rest).map(|given| {
                if given { "reacted" } else { "reaction taken back" }.to_string()
            })
        });
    }

    fn close_item(&mut self, kind: Kind, number: u64, reason: CloseReason) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.send("closed", "could not close", move |gh, cwd| {
            fetch::close(gh, cwd, kind, &repo, number, reason)
        });
    }

    /// Runs a write on a thread; `done` is what the footer says when it worked, `failed` the
    /// start of what it says when it did not.
    pub(super) fn send(
        &mut self,
        done: &str,
        failed: &str,
        job: impl FnOnce(&dyn super::Gh, &std::path::Path) -> Result<(), super::GhError> + Send + 'static,
    ) {
        let done = done.to_string();
        self.send_text(failed, move |gh, cwd| job(gh, cwd).map(|()| done));
    }

    /// Like [`Github::send`] for a write that says itself what it did.
    fn send_text(
        &mut self,
        failed: &str,
        job: impl FnOnce(&dyn super::Gh, &std::path::Path) -> Result<String, super::GhError> + Send + 'static,
    ) {
        self.sending = true;
        self.info = Some("sending…".into());
        let failed = failed.to_string();
        self.spawn(move |gh, cwd| Msg::Wrote {
            failed,
            result: job(gh, cwd),
        });
    }

    /// What the footer and the detail show after a write: the item is fetched again, and the
    /// list under it (its state and counts may have changed).
    pub(super) fn wrote(&mut self, failed: String, result: Result<String, super::GhError>) {
        self.sending = false;
        self.line_comment_done(&result);
        match result {
            Ok(done) => {
                self.unsent = None;
                self.info = Some(done);
                if let Some((kind, number)) = self.target() {
                    self.load_detail(kind, number);
                }
                self.reload_list();
            }
            Err(e) => {
                self.info = None;
                // A comment that did not go is back in the draft, not lost.
                if let Some(text) = self.unsent.take() {
                    self.draft.set_text(&text);
                }
                self.note = Some(format!("{failed}: {}", e.message()));
            }
        }
    }

    /// The input under the detail, always there: the comment being written, from one row up
    /// to [`BOX_ROWS`]. Its cursor shows while it has the keys.
    fn input(&mut self, w: usize, h: usize, st: &Styles) -> Panel {
        let typing = matches!(self.writing, Some(Writing::Comment));
        let number = self.target().map_or(0, |t| t.1);
        let bg = if st.raised == Color::Reset {
            Style::new()
        } else {
            Style::new().bg(st.raised)
        };
        // A space in, then the prompt mark or its room.
        const IN: usize = 3;
        self.draft.set_width(w.saturating_sub(IN + 1).max(1));
        if self.draft.is_empty() && !typing {
            let hint = format!("tab to comment on #{number}");
            return Panel {
                lines: vec![row_line(
                    vec![Span::styled("› ", st.muted), Span::styled(hint, st.muted)],
                    w,
                    bg,
                )],
                cursor: None,
            };
        }
        let rows = self.draft.rows();
        let room = BOX_ROWS.min(h.saturating_sub(1)).max(1);
        let (cursor_row, cursor_col) = self.draft.cursor_pos();
        let top = if typing { (cursor_row + 1).saturating_sub(room) } else { 0 };
        let mark = if typing { st.accent } else { st.muted };
        let text = if typing { Style::new() } else { st.muted };
        let mut lines = Vec::new();
        for (i, r) in rows.iter().skip(top).take(room).enumerate() {
            let lead = if i == 0 && top == 0 { "› " } else { "  " };
            lines.push(row_line(
                vec![
                    Span::styled(lead, mark),
                    Span::styled(self.draft.text()[r.start..r.end].to_string(), text),
                ],
                w,
                bg,
            ));
        }
        if lines.is_empty() {
            lines.push(row_line(vec![Span::styled("› ", mark)], w, bg));
        }
        Panel {
            cursor: typing.then(|| (cursor_row - top, IN + cursor_col)),
            lines,
        }
    }

    /// The panel at the bottom of the detail, `h` rows of the body at most: the input, or what
    /// a key asked for in its place.
    pub(super) fn panel(&mut self, w: usize, h: usize, st: &Styles) -> Panel {
        let who = self.picked_author();
        let number = self.target().map_or(0, |t| t.1);
        let kind = self.target().map_or(Kind::Issue, |t| t.0);
        let bg = if st.raised == Color::Reset {
            Style::new()
        } else {
            Style::new().bg(st.raised)
        };
        let row = |spans: Vec<Span<'static>>| {
            let mut line = Line::from(spans);
            line.spans.insert(0, Span::raw(" "));
            let pad = w.saturating_sub(line.width());
            line.spans.push(Span::raw(" ".repeat(pad)));
            for span in &mut line.spans {
                span.style = bg.patch(span.style);
            }
            line
        };
        let bold = Style::new().add_modifier(Modifier::BOLD);
        // The lists and typed values of editing are drawn by `edit.rs`.
        if matches!(
            self.writing,
            Some(Writing::Edit | Writing::Loading(_) | Writing::Pick(_) | Writing::Input(_))
        ) && let Some(writing) = self.writing.take()
        {
            let panel = self.edit_panel(&writing, number, w, h, st, bg);
            self.writing = Some(writing);
            if let Some(panel) = panel {
                return panel;
            }
        }
        // Reading or typing a comment: the input.
        let Some(writing) = self.writing.as_ref().filter(|w| !matches!(w, Writing::Comment))
        else {
            return self.input(w, h, st);
        };
        match writing {
            Writing::Comment => unreachable!("the input, above"),
            Writing::Body(target) => {
                let what = match target {
                    BodyOf::Item => format!("Edit the description of #{number}"),
                    BodyOf::Comment(_) => "Edit the comment".to_string(),
                };
                editor_panel(
                    &mut self.body_edit,
                    vec![
                        Span::styled(what, bold),
                        Span::styled("  ctrl-s save · esc drop the changes", st.muted),
                    ],
                    w,
                    h,
                    &row,
                )
            }
            Writing::React => {
                let mut options = Vec::new();
                for (i, (_, _, emoji)) in REACTIONS.iter().enumerate() {
                    if i > 0 {
                        options.push(Span::raw("  "));
                    }
                    options.push(Span::styled(format!("{}", i + 1), st.accent));
                    options.push(Span::raw(format!(" {emoji}")));
                }
                options.push(Span::styled("   esc cancel", st.muted));
                Panel {
                    lines: vec![
                        row(vec![
                            Span::styled(format!("React to {who}"), bold),
                            Span::styled("  the same again takes it back", st.muted),
                        ]),
                        row(options),
                    ],
                    cursor: None,
                }
            }
            Writing::Close { dup: None } => {
                let key = |k: &str, what: &str| {
                    vec![
                        Span::styled(k.to_string(), st.accent),
                        Span::raw(format!(" {what}")),
                    ]
                };
                let mut options = key("1", if kind == Kind::Issue { "completed" } else { "close" });
                if kind == Kind::Issue {
                    options.push(Span::raw("   "));
                    options.extend(key("2", "not planned"));
                    options.push(Span::raw("   "));
                    options.extend(key("3", "duplicate"));
                }
                options.push(Span::styled("   esc cancel", st.muted));
                let noun = if kind == Kind::Issue { "Close issue" } else { "Close pull request" };
                Panel {
                    lines: vec![row(vec![Span::styled(format!("{noun} #{number} as"), bold)]), row(options)],
                    cursor: None,
                }
            }
            Writing::Close { dup: Some(typed) } => {
                let shown = format!("Duplicate of #{typed}");
                let col = 1 + width(&shown);
                Panel {
                    lines: vec![
                        row(vec![Span::styled(format!("Close #{number} as a duplicate"), bold)]),
                        row(vec![
                            Span::raw(shown),
                            Span::styled("   enter close · esc back", st.muted),
                        ]),
                    ],
                    cursor: Some((1, col)),
                }
            }
            Writing::Edit | Writing::Loading(_) | Writing::Pick(_) | Writing::Input(_) => Panel {
                lines: Vec::new(),
                cursor: None,
            },
            Writing::Reopen => Panel {
                lines: vec![
                    row(vec![Span::styled(format!("Reopen #{number}?"), bold)]),
                    row(vec![
                        Span::styled("1", st.accent),
                        Span::raw(" reopen"),
                        Span::styled("   esc cancel", st.muted),
                    ]),
                ],
                cursor: None,
            },
        }
    }
}

/// Whether the key `c` acts on a block that is `part`; None when it is not a key that belongs
/// to blocks (it works everywhere, or is nobody's).
pub(super) fn allowed(part: Part, c: char, pr: bool) -> Option<bool> {
    let text = matches!(part, Part::Description | Part::Comment(_));
    Some(match c {
        'x' | 'S' | 'm' | 'A' | 'L' | 'M' | 'T' | 'F' | 'P' | 'R' => part == Part::Head,
        'E' | '+' => text,
        '>' => text || matches!(part, Part::Review(_) | Part::DiffComment(_)),
        'd' if pr => matches!(part, Part::Head | Part::Files),
        _ => return None,
    })
}

/// The text as a markdown quote, with the empty line after it that a reply starts on.
fn quote(text: &str) -> String {
    let mut out: String = text
        .trim_end()
        .lines()
        .map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") })
        .collect::<Vec<_>>()
        .join("\n");
    out.push_str("\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn a_quote_marks_every_line_and_leaves_room_to_reply() {
        assert_eq!(quote("one\n\ntwo\n"), "> one\n>\n> two\n\n");
    }
}
