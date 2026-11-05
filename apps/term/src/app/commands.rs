use super::*;

impl App {
    // ---- sending and commands ----

    /// The slash menu's commands: ours, and those of the harness the session runs on.
    pub(super) fn commands(&self) -> Vec<Item> {
        let listed = self
            .offered
            .get(&self.harness)
            .map_or(&[][..], Vec::as_slice);
        menu::items_for(&self.harness, listed)
    }

    pub(super) fn menu_matches(&self) -> Vec<Item> {
        let text = self.editor.text();
        if self.focus.is_some()
            || self.prompt.engaged
            || self.overlay != Overlay::None
            || !menu::is_slash_word(text)
            || self.menu_closed_for.as_deref() == Some(text)
        {
            return Vec::new();
        }
        menu::matching(text, &self.commands())
    }

    /// The files the `@word` at the cursor could be (none while anything else has the keys).
    pub(super) fn mention_matches(&self) -> Vec<Candidate> {
        let text = self.editor.text();
        if self.focus.is_some()
            || self.prompt.engaged
            || self.overlay != Overlay::None
            || self.mention_closed_for.as_deref() == Some(text)
        {
            return Vec::new();
        }
        let Some(m) = mention::at_cursor(text, self.editor.cursor()) else {
            return Vec::new();
        };
        let home = self.home.as_deref().map(std::path::Path::new);
        self.mentions
            .candidates(std::path::Path::new(&self.cwd), home, &m)
    }

    pub(super) fn mention_visible(&self) -> bool {
        !self.mention_matches().is_empty()
    }

    pub(super) fn move_mention(&mut self, delta: isize) {
        let n = self.mention_matches().len() as isize;
        if n > 0 {
            self.mention_sel = (self.mention_sel as isize + delta).rem_euclid(n) as usize;
        }
    }

    /// Puts the selected file in the `@word`'s place: a folder stays open to go on in, a file
    /// ends the word with a space.
    pub(super) fn accept_mention(&mut self) {
        let matches = self.mention_matches();
        let Some(pick) = matches.get(self.mention_sel.min(matches.len().saturating_sub(1))) else {
            return;
        };
        let text = self.editor.text().to_string();
        let cursor = self.editor.cursor();
        let Some(m) = mention::at_cursor(&text, cursor) else {
            return;
        };
        let word = format!(
            "{}{}{}",
            m.sigil(),
            pick.path,
            if pick.dir { "" } else { " " }
        );
        let after = &text[cursor..];
        let new = format!("{}{word}{after}", &text[..m.start]);
        self.editor.set_text(&new);
        // The cursor after the word: set_text leaves it at the end.
        for _ in 0..after.chars().count() {
            self.editor.left();
        }
        self.mention_sel = 0;
    }

    pub(super) fn menu_visible(&self) -> bool {
        !self.menu_matches().is_empty()
    }

    pub(super) fn move_menu(&mut self, delta: isize) {
        let n = self.menu_matches().len() as isize;
        if n > 0 {
            self.menu_sel = (self.menu_sel as isize + delta).rem_euclid(n) as usize;
        }
    }

    pub(super) fn complete_menu(&mut self) {
        let matches = self.menu_matches();
        if let Some(cmd) = matches.get(self.menu_sel.min(matches.len().saturating_sub(1))) {
            self.editor.set_text(&format!("/{} ", cmd.name));
            self.menu_sel = 0;
        }
    }

    /// Enter in the editor; true when the text was sent or run, false when there's more to type.
    pub(super) fn enter(&mut self, fx: &mut Vec<Effect>) -> bool {
        let matches = self.menu_matches();
        if !matches.is_empty() {
            let cmd = &matches[self.menu_sel.min(matches.len() - 1)];
            if cmd.needs_args {
                self.editor.set_text(&format!("/{} ", cmd.name));
                self.menu_sel = 0;
                return false;
            }
            self.editor.clear();
            self.menu_sel = 0;
            self.run_command(&cmd.name, &format!("/{}", cmd.name), fx);
            return true;
        }
        let text = self.editor.text().trim().to_string();
        if text.is_empty() {
            return false;
        }
        self.editor.clear();
        self.menu_sel = 0;
        let local = text
            .starts_with('/')
            .then(|| menu::lookup(&text))
            .flatten()
            .filter(|c| c.local);
        match local {
            Some(cmd) => self.run_command(cmd.name, &text, fx),
            None => self.send(text, fx),
        }
        true
    }

    pub(super) fn run_command(&mut self, name: &str, text: &str, fx: &mut Vec<Effect>) {
        match name {
            "sessions" => self.open_switcher(fx),
            "sidebar" => self.toggle_sessions(),
            "expand" => self.toggle_expanded(),
            "thoughts" => self.show_thoughts = !self.show_thoughts,
            "help" => self.overlay = Overlay::Help,
            "github" => self.github.open(),
            "theme" => self.switch_theme(text),
            "todo" => self.add_todo(text, fx),
            "files" => self.focus_files(),
            "terminal" => self.toggle_terminal(),
            "quit" => fx.push(Effect::Quit),
            // Alone, the words there now go into the editor to be changed and sent again.
            "system-prompt"
                if text.trim() == "/system-prompt" && self.prompt_addition.is_some() =>
            {
                let now = self.prompt_addition.clone().unwrap_or_default();
                self.editor.set_text(&format!("/system-prompt {now}"));
            }
            _ => self.send(text.to_string(), fx),
        }
    }

    /// What the GitHub view hands over: text for the editor, or a message to send right away.
    pub(super) fn take_github(&mut self, out: Outgoing, fx: &mut Vec<Effect>) {
        match out {
            Outgoing::Attach(text) => {
                // After what is already typed, with room to add instructions below.
                self.editor.text_end();
                if !self.editor.is_empty() {
                    self.editor.insert_str("\n\n");
                }
                self.editor.insert_str(&text);
                self.editor.insert_str("\n\n");
                self.histories.of(&self.session).reset();
            }
            Outgoing::Send(text) => self.send(text, fx),
        }
    }

    /// A comment from the diff view, as `@path:12-14 text`: a new one goes on a line of its own
    /// at the end of the prompt; an edited one takes the old one's place (where the user left
    /// it), a deleted one goes.
    pub(super) fn comment_in_prompt(&mut self, before: Option<String>, after: Option<String>) {
        let text = self.editor.text().to_string();
        let new = match (before.filter(|b| text.contains(b.as_str())), after) {
            (Some(b), Some(a)) => text.replacen(&b, &a, 1),
            (Some(b), None) => {
                let gone = text.replacen(&format!("{b}\n"), "", 1);
                let gone = if gone == text {
                    text.replacen(&b, "", 1)
                } else {
                    gone
                };
                gone.trim_end().to_string()
            }
            (None, Some(a)) if text.trim().is_empty() => a,
            (None, Some(a)) => format!("{}\n{a}", text.trim_end()),
            (None, None) => return,
        };
        self.editor.set_text(&new);
    }

    pub(super) fn send(&mut self, text: String, fx: &mut Vec<Effect>) {
        self.histories.record(&self.session, &text);
        self.widgets.comments_sent(&text);
        self.chat.follow();
        // Sending ends the typing: a card it brings (/model's choice) takes the editor's place at
        // once instead of showing in the chat first.
        self.prompt.typed_ms = 0;
        let (text, images) = attach::outgoing(&text, &self.attachments);
        self.attachments.retain(|(_, path)| !images.contains(path));
        if self.running {
            self.waiting.push(WaitingInput {
                app: self.app_id.clone(),
                text: text.clone(),
                images: (!images.is_empty()).then(|| images.clone()),
            });
        }
        fx.push(Effect::Input {
            session: self.session.clone(),
            text,
            images,
            steer: false,
        });
        // What was written in its place is out: the draft put aside comes back.
        if self.editor.is_empty() && self.stash.is_some() {
            self.swap_stash();
        }
    }

    /// Ctrl-S: the draft and its images go aside and the editor is free for another message, or
    /// the one put aside comes back (trading places with what is in the editor now).
    pub(super) fn swap_stash(&mut self) {
        let text = self.editor.text().to_string();
        let attachments = std::mem::take(&mut self.attachments);
        let draft = (!text.trim().is_empty()).then_some((text, attachments));
        match std::mem::replace(&mut self.stash, draft) {
            Some((text, attachments)) => {
                self.editor.set_text(&text);
                self.attachments = attachments;
            }
            None => self.editor.clear(),
        }
        self.menu_sel = 0;
        self.histories.of(&self.session).reset();
        self.dirty = true;
    }
}
