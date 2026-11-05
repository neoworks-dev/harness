use super::*;

impl App {
    /// Tab: editor, the chat's files and replies, sessions, terminal, plan, files (the ones on
    /// screen), editor.
    /// Tab (`delta` 1) or Shift-Tab (-1): the next or previous pane on screen, the editor past
    /// either end.
    pub(super) fn cycle_focus(&mut self, delta: isize) {
        let order = [
            (self.chat.has_picks(), Focus::Chat),
            (self.sessions_shown, Focus::Sessions),
            // The side column top to bottom.
            (
                self.sidebar_shown && self.widgets.has_todos(),
                Focus::Panel(Panel::Todos),
            ),
            (self.sidebar_shown, Focus::Panel(Panel::Plan)),
            (self.sidebar_shown, Focus::Panel(Panel::Files)),
            (self.terminal_area.is_some(), Focus::Panel(Panel::Terminal)),
        ];
        let mut on_screen: Vec<Focus> = order
            .iter()
            .filter(|(shown, _)| *shown)
            .map(|(_, f)| *f)
            .collect();
        if delta < 0 {
            on_screen.reverse();
        }
        let mut on_screen = on_screen.into_iter();
        self.focus = match self.focus {
            None => on_screen.next(),
            Some(current) => on_screen.skip_while(|f| *f != current).nth(1),
        };
        self.enter_focus();
    }

    /// Ctrl-G: the sessions sidebar gets the keys (it is switched on first when hidden), or gives
    /// them back.
    pub(super) fn toggle_sessions_focus(&mut self) {
        if self.focus == Some(Focus::Sessions) {
            self.focus = None;
            return;
        }
        if self.sessions.is_empty() {
            self.notify("no sessions yet", false);
        } else if self.sessions_on && self.sessions_shown {
            self.focus = Some(Focus::Sessions);
            self.enter_focus();
        } else {
            // Switched on now: the next draw knows whether it fits, and focuses it or says why not.
            self.sessions_on = true;
            self.sessions_wanted = true;
        }
    }

    pub(super) fn enter_focus(&mut self) {
        match self.focus {
            Some(Focus::Sessions) => self.sessions.focus(&self.session),
            // The latest file or reply, unless one is picked already.
            Some(Focus::Chat) => self.chat.move_pick(0),
            _ => {}
        }
    }

    /// `/files`: the files panel gets the keys.
    pub(super) fn focus_files(&mut self) {
        if self.sidebar_shown {
            self.focus = Some(Focus::Panel(Panel::Files));
        } else if self.widgets.wants_sidebar(u16::MAX) {
            self.notify("terminal too narrow for the files panel", false);
        } else {
            self.notify("no changed files", false);
        }
    }

    /// `/terminal`: show or hide the agent's commands next to the chat.
    pub(super) fn toggle_terminal(&mut self) {
        self.terminal_on = !self.terminal_on;
        if self.terminal_on && !self.widgets.has_commands() {
            self.notify(
                "no commands yet; they show here once the agent runs one",
                false,
            );
        }
    }

    /// `/sidebar`: show or hide the sessions sidebar.
    pub(super) fn toggle_sessions(&mut self) {
        self.sessions_on = !self.sessions_on;
        if !self.sessions_on && self.focus == Some(Focus::Sessions) {
            self.focus = None;
        }
    }

    pub(super) fn switcher_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('c') if ctrl => self.overlay = Overlay::None,
            KeyCode::Up => self.switcher.move_by(-1),
            KeyCode::Down => self.switcher.move_by(1),
            KeyCode::Char('p') if ctrl => self.switcher.move_by(-1),
            KeyCode::Char('n') if ctrl => self.switcher.move_by(1),
            KeyCode::Backspace => self.switcher.backspace(),
            KeyCode::Char('u') if ctrl => {
                while !self.switcher.filter.is_empty() {
                    self.switcher.backspace();
                }
            }
            KeyCode::Enter => match self.switcher.selected_entry() {
                Some(Entry::New) => {
                    self.overlay = Overlay::None;
                    fx.push(Effect::CreateSession {
                        cwd: self.cwd.clone(),
                    });
                }
                Some(Entry::Session(i)) => {
                    let picked = self
                        .switcher
                        .sessions
                        .as_ref()
                        .and_then(|s| s.get(i))
                        .cloned();
                    self.overlay = Overlay::None;
                    if let Some(info) = picked {
                        self.open_session(info, fx);
                    }
                }
                None => {}
            },
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.switcher.type_char(c);
            }
            _ => {}
        }
    }

    /// Shows another session and brings the agent to this app (the switcher and the sidebar).
    pub(super) fn open_session(&mut self, info: SessionInfo, fx: &mut Vec<Effect>) {
        if info.id == self.session {
            return;
        }
        let from = self.session.clone();
        self.cancel_secret(fx);
        self.begin_session(&info.id, &info.cwd);
        self.awaiting = Some((info.id.clone(), self.now_ms + 3000));
        fx.push(Effect::Switch {
            from: Some(from),
            to: info.id,
            cwd: info.cwd,
            move_agent: true,
        });
    }

    /// Ctrl-O: tool results in full, or back. Scrolled up, the view stays on what it showed.
    pub(super) fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
        self.chat.keep_place();
        if let Some(t) = self.transcript.as_mut() {
            t.chat.keep_place();
        }
    }

    /// Marks the shown session (the last slot), or unmarks it.
    pub(super) fn toggle_mark(&mut self) {
        let said = match self.marks.iter().position(|m| *m == self.session) {
            Some(at) => {
                self.marks.remove(at);
                "unmarked".to_string()
            }
            None => {
                self.marks.push(self.session.clone());
                format!("marked: ctrl-{}", self.marks.len())
            }
        };
        self.save_marks(&said);
    }

    /// Keeps the marks in `term.json`, and says `said` (or why they weren't kept).
    pub(super) fn save_marks(&mut self, said: &str) {
        match self.prefs.set_marks(&self.marks) {
            Ok(()) => self.notify(said, false),
            Err(why) => self.notify(format!("{said} (not kept: {why})"), true),
        }
    }

    /// Shows the session marked in slot `n` (from 0).
    pub(super) fn jump_to_mark(&mut self, n: usize, fx: &mut Vec<Effect>) {
        let Some(id) = self.marks.get(n).cloned() else {
            self.notify(format!("nothing marked at {} (alt-m marks)", n + 1), false);
            return;
        };
        match self.sessions.get(&id).cloned() {
            Some(info) => {
                self.overlay = Overlay::None;
                self.open_session(info, fx);
            }
            None => {
                self.marks.remove(n);
                self.save_marks(&format!("the chat at {} is gone: unmarked", n + 1));
            }
        }
    }

    /// The marks list: j/k move, J/K move the mark itself, d unmarks, a marks or unmarks the
    /// shown chat, Enter (or its number) shows it, Esc (or Ctrl-E again) closes.
    pub(super) fn marks_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let len = self.marks.len();
        let at = self.marks_sel.min(len.saturating_sub(1));
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Char('\'') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.overlay = Overlay::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.marks_sel = (at + 1).min(len.saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.marks_sel = at.saturating_sub(1),
            KeyCode::Char('J') if at + 1 < len => {
                self.marks.swap(at, at + 1);
                self.marks_sel = at + 1;
                self.save_marks(&format!("moved to {}", at + 2));
            }
            KeyCode::Char('K') if at > 0 => {
                self.marks.swap(at, at - 1);
                self.marks_sel = at - 1;
                self.save_marks(&format!("moved to {at}"));
            }
            KeyCode::Char('d') | KeyCode::Delete if len > 0 => {
                self.marks.remove(at);
                self.marks_sel = at.min(self.marks.len().saturating_sub(1));
                self.save_marks("unmarked");
            }
            KeyCode::Enter if len > 0 => self.jump_to_mark(at, fx),
            KeyCode::Char('a') => {
                self.toggle_mark();
                self.marks_sel = self.marks.len().saturating_sub(1);
            }
            KeyCode::Char(d @ '1'..='9') => self.jump_to_mark(usize::from(d as u8 - b'1'), fx),
            _ => {}
        }
    }

    pub(super) fn open_switcher(&mut self, fx: &mut Vec<Effect>) {
        self.switcher = Switcher::new(&self.session);
        self.overlay = Overlay::Switcher;
        fx.push(Effect::ListSessions);
    }

    /// Shift-Tab: ask, review, edits, auto, ask…
    pub(super) fn cycle_approvals(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Input {
            session: self.session.clone(),
            text: format!("/approvals {}", self.approvals.next().word()),
            images: Vec::new(),
            steer: false,
        });
    }

    pub(super) fn ctrl_c(&mut self, fx: &mut Vec<Effect>) {
        if (!self.editor.is_empty() || !self.attachments.is_empty()) && !self.prompt.engaged {
            self.editor.clear();
            self.attachments.clear();
            self.histories.of(&self.session).reset();
        } else if self.quit_armed_until > self.now_ms {
            fx.push(Effect::Quit);
        } else {
            self.quit_armed_until = self.now_ms + QUIT_WINDOW_MS;
            self.notice = Some(Notice {
                text: "press ctrl-c again to quit".into(),
                error: false,
                until: self.quit_armed_until,
            });
        }
    }

    pub(super) fn esc(&mut self, fx: &mut Vec<Effect>) {
        if self.prompt.engaged && self.filtering() && !self.prompt.filter.is_empty() {
            self.edit_filter(String::clear);
        } else if self.menu_visible() {
            self.menu_closed_for = Some(self.editor.text().to_string());
        } else if let Some(r) = self.active_ref().map(str::to_string) {
            self.prompt.dismissed.push(r);
            self.prompt.selected = None;
        } else if self.running {
            fx.push(Effect::Cancel {
                session: self.session.clone(),
            });
            self.notify("cancelling…", false);
        } else if !self.chat.following() {
            self.chat.follow();
        }
    }
}
