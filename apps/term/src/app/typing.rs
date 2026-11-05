use super::*;

impl App {
    /// Keys while a prompt has taken the editor's place: they answer it, and nothing else
    /// reaches the editor (and its draft).
    pub(super) fn prompt_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        if let Some(r) = self.active_form().map(str::to_string) {
            return self.form_key(&r, key, fx);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char(c) if ctrl => match c {
                'c' => self.ctrl_c(fx),
                'u' if self.filtering() => self.edit_filter(String::clear),
                'p' => {
                    self.select_prompt(-1);
                }
                'n' => {
                    self.select_prompt(1);
                }
                'o' | 't' | 's' | 'l' | 'g' | 'x' => {
                    self.ctrl_key(c, fx);
                }
                _ => {}
            },
            // On a long choice every character is for the filter, digits too.
            KeyCode::Char(c) if self.filtering() => {
                if !key.modifiers.contains(KeyModifiers::ALT) {
                    self.edit_filter(|f| f.push(c));
                }
            }
            KeyCode::Backspace if self.filtering() => {
                self.edit_filter(|f| {
                    f.pop();
                });
            }
            // A setting changed in place: the card stays, drawn again as it is now.
            KeyCode::Char(' ') if self.focused_button().is_some_and(|b| b.stays) => {
                self.press_staying(fx);
            }
            KeyCode::Char(c) => {
                if let Some(n) = c.to_digit(10).filter(|n| (1..=9).contains(n))
                    && (n as usize) <= self.active_buttons().len()
                {
                    self.press(n as usize - 1, fx);
                }
            }
            KeyCode::Enter => {
                // With nothing matching there is nothing to pick.
                if !self.filtering() || self.shown_buttons().contains(&self.prompt.button) {
                    self.press(self.prompt.button, fx);
                }
            }
            KeyCode::Tab => self.tab_on_prompt(),
            KeyCode::BackTab => self.cycle_approvals(fx),
            KeyCode::Esc => self.esc(fx),
            KeyCode::Up if shift => self.chat.scroll_up(1),
            KeyCode::Down if shift => self.chat.scroll_down(1),
            KeyCode::Left | KeyCode::Up => self.move_button(-1),
            KeyCode::Right | KeyCode::Down => self.move_button(1),
            KeyCode::PageUp => self.chat.page_up(),
            KeyCode::PageDown => self.chat.page_down(),
            KeyCode::Home => self.chat.to_top(),
            KeyCode::End => self.chat.follow(),
            _ => {}
        }
    }

    pub(super) fn editor_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        if self.prompt.engaged {
            return self.prompt_key(key, fx);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let empty = self.editor.is_empty();
        // The `@` file list (or `/cd`'s folders) takes the arrows, Tab, Enter and Esc while it
        // is open. Enter on a `/cd` that names a folder already runs it, unless the arrows picked
        // one.
        let runs_cd = || {
            self.mention_sel == 0
                && mention::at_cursor(self.editor.text(), self.editor.cursor())
                    .is_some_and(|m| m.complete())
        };
        if !ctrl && !alt && !shift && self.mention_visible() {
            match key.code {
                KeyCode::Up => return self.move_mention(-1),
                KeyCode::Down => return self.move_mention(1),
                KeyCode::Enter if runs_cd() => {}
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_mention();
                    self.prompt.typed_ms = self.now_ms;
                    return;
                }
                KeyCode::Esc => {
                    self.mention_closed_for = Some(self.editor.text().to_string());
                    return;
                }
                _ => {}
            }
        }
        let mut edited = true;
        match key.code {
            KeyCode::Char(c) if ctrl => edited = self.ctrl_key(c, fx),
            KeyCode::Char(c) if alt => match c {
                'b' => self.editor.word_left(),
                'f' => self.editor.word_right(),
                'd' => self.editor.delete_word_forward(),
                _ => edited = false,
            },
            KeyCode::Char(' ') if !shift => edited = self.space(key.kind, fx),
            KeyCode::Char(c) => {
                if c == '?' && empty {
                    self.overlay = Overlay::Help;
                    return;
                }
                self.editor.insert_char(c);
            }
            KeyCode::Enter if shift => self.editor.newline(),
            // Ctrl-Enter or Alt-Enter (which every terminal tells apart from Enter): into the
            // running turn at once, which goes on with it, not after it.
            KeyCode::Enter if ctrl || alt => {
                let from = fx.len();
                if self.enter(fx) {
                    self.histories.of(&self.session).reset();
                    edited = false;
                }
                if self.running {
                    for effect in &mut fx[from..] {
                        if let Effect::Input { steer, .. } = effect {
                            *steer = true;
                        }
                    }
                }
            }
            KeyCode::Enter => {
                if self.enter(fx) {
                    // Sent: the typing is over (see send), not just paused.
                    self.histories.of(&self.session).reset();
                    edited = false;
                }
            }
            KeyCode::BackTab => {
                self.cycle_approvals(fx);
                edited = false;
            }
            KeyCode::Tab => {
                edited = false;
                if self.menu_visible() {
                    self.complete_menu();
                } else {
                    self.cycle_focus(1);
                }
            }
            KeyCode::Esc => {
                self.esc(fx);
                edited = false;
            }
            KeyCode::Backspace if alt => self.editor.delete_word_back(),
            // A pasted image's marker goes as a whole (and the image with it).
            KeyCode::Backspace => {
                match attach::marker_before(self.editor.text(), self.editor.cursor()) {
                    Some(len) => self.editor.delete_before(len),
                    None => self.editor.backspace(),
                }
            }
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Left if ctrl || alt => self.editor.word_left(),
            KeyCode::Right if ctrl || alt => self.editor.word_right(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right => self.editor.right(),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.chat.scroll_up(1);
                edited = false;
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.chat.scroll_down(1);
                edited = false;
            }
            KeyCode::Up => {
                edited = false;
                if self.menu_visible() {
                    self.move_menu(-1);
                } else if !self.editor.up()
                    && !(empty && !self.histories.of(&self.session).browsing() && self.unqueue(fx))
                {
                    self.history_prev();
                }
            }
            KeyCode::Down => {
                edited = false;
                if self.menu_visible() {
                    self.move_menu(1);
                } else if !self.editor.down() {
                    // Past the end of an empty draft, Down goes on to the subagents list, then
                    // the background commands.
                    let below = empty && !self.histories.of(&self.session).browsing();
                    if below && self.agents.visible(self.now_ms) {
                        self.focus_agents();
                    } else if below && !self.widgets.jobs(self.now_ms).is_empty() {
                        self.focus_jobs();
                    } else {
                        self.history_next();
                    }
                }
            }
            KeyCode::Home if ctrl || empty => {
                self.chat.to_top();
                edited = false;
            }
            KeyCode::End if ctrl || empty => {
                self.chat.follow();
                edited = false;
            }
            KeyCode::Home => self.editor.line_start(),
            KeyCode::End => self.editor.line_end(),
            KeyCode::PageUp => {
                self.chat.page_up();
                edited = false;
            }
            KeyCode::PageDown => {
                self.chat.page_down();
                edited = false;
            }
            _ => edited = false,
        }
        if edited {
            self.histories.of(&self.session).reset();
            self.prompt.typed_ms = self.now_ms;
            self.mention_sel = 0;
        }
    }

    /// Returns whether the editor text was edited.
    pub(super) fn ctrl_key(&mut self, c: char, fx: &mut Vec<Effect>) -> bool {
        let empty = self.editor.is_empty();
        match c {
            'c' => self.ctrl_c(fx),
            'd' if empty => {
                if self.chat.following() {
                    fx.push(Effect::Quit);
                } else {
                    self.chat.half_page_down();
                }
                return false;
            }
            'd' => self.editor.delete(),
            'v' => {
                fx.push(Effect::PasteImage);
                return false;
            }
            // While a command offers it (see backgroundable); else the cursor goes left.
            'b' if let Some(id) = self.backgroundable() => {
                fx.push(Effect::Action {
                    session: self.session.clone(),
                    item: format!("c-{id}"),
                    action: "neo.background".into(),
                    args: Some(serde_json::json!({ "toolCallId": id })),
                });
                self.notify("sent to the background: it runs on under the prompt", false);
                return false;
            }
            // Putting the draft aside makes room: a waiting prompt takes it at once.
            's' => {
                self.swap_stash();
                self.prompt.typed_ms = 0;
                self.update_engaged();
                return false;
            }
            'r' => {
                self.github.open();
                return false;
            }
            'o' => {
                self.toggle_expanded();
                return false;
            }
            't' => {
                self.show_thoughts = !self.show_thoughts;
                return false;
            }
            'l' => {
                self.clear = true;
                return false;
            }
            'g' => {
                self.toggle_sessions_focus();
                return false;
            }
            'x' => {
                self.focus_agents();
                return false;
            }
            'p' | 'n' => {
                let delta = if c == 'p' { -1 } else { 1 };
                if self.menu_visible() {
                    self.move_menu(delta);
                } else if self.prompt_waits() {
                    // The prompt held back by the draft is the one asked for.
                    self.prompt.engaged = true;
                    self.dirty = true;
                } else if !self.select_prompt(delta) {
                    if delta < 0 {
                        self.history_prev();
                    } else {
                        self.history_next();
                    }
                }
                return false;
            }
            'a' => self.editor.line_start(),
            'b' => self.editor.left(),
            'f' => self.editor.right(),
            'w' => self.editor.delete_word_back(),
            'h' => self.editor.backspace(),
            'u' if empty => {
                self.chat.half_page_up();
                return false;
            }
            'u' => self.editor.delete_to_line_start(),
            'k' => self.editor.delete_to_line_end(),
            'j' => self.editor.newline(),
            _ => return false,
        }
        true
    }

    /// Space in the editor: typed, or, held down, recording speech until it is let go (see
    /// voice.rs). Returns whether the draft changed.
    pub(super) fn space(&mut self, kind: KeyEventKind, fx: &mut Vec<Effect>) -> bool {
        if matches!(self.voice, Voice::Recording(_)) {
            self.spaces.held(self.now_ms);
            return false;
        }
        // Held again while the last one is written out: only typed Spaces go in.
        let hold = self.spaces.press(self.now_ms, kind == KeyEventKind::Repeat);
        match hold {
            SpaceKey::Type => self.editor.insert_char(' '),
            SpaceKey::Hold { .. } if self.voice == Voice::Transcribing => return false,
            SpaceKey::Hold { typed } => {
                let before = &self.editor.text()[..self.editor.cursor()];
                if typed > 0 && before.ends_with(&" ".repeat(typed)) {
                    self.editor.delete_before(typed);
                }
                self.voice = Voice::Recording(self.now_ms);
                fx.push(Effect::VoiceStart);
            }
        }
        true
    }

    /// Space was let go: the recording goes to be written out.
    pub(super) fn stop_voice(&mut self, fx: &mut Vec<Effect>) {
        if matches!(self.voice, Voice::Recording(_)) {
            self.voice = Voice::Transcribing;
            self.dirty = true;
            fx.push(Effect::VoiceStop);
        }
    }

    /// What was said goes in at the cursor, a space apart from the text around it.
    pub(super) fn dictate(&mut self, text: &str) {
        let before = &self.editor.text()[..self.editor.cursor()];
        if before.chars().last().is_some_and(|c| !c.is_whitespace()) {
            self.editor.insert_char(' ');
        }
        self.editor.insert_str(text);
        self.dirty = true;
    }

    pub(super) fn history_prev(&mut self) {
        let current = self.editor.text().to_string();
        if let Some(text) = self.histories.of(&self.session).older(&current) {
            self.recall(&text);
        }
    }

    pub(super) fn history_next(&mut self) {
        if let Some(text) = self.histories.of(&self.session).newer() {
            self.recall(&text);
        }
    }

    /// A recalled `/command` keeps the menu closed, or the next Up/Down would move in it
    /// instead of going on through the history.
    pub(super) fn recall(&mut self, text: &str) {
        self.editor.set_text(text);
        self.menu_closed_for = Some(text.to_string());
    }
}
