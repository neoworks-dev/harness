use super::*;

impl App {
    // ---- keys ----

    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut fx = Vec::new();
        let space = key.code == KeyCode::Char(' ') && key.modifiers.is_empty();
        if key.kind == KeyEventKind::Release {
            // Letting go of Space ends the recording it holds (kitty's keyboard protocol tells).
            if space && matches!(self.voice, Voice::Recording(_)) {
                self.stop_voice(&mut fx);
            }
            return fx;
        }
        if !space {
            self.spaces = SpaceRun::default();
        }
        self.dirty = true;
        if self.secret.is_some() {
            self.secret_key(key, &mut fx);
            return fx;
        }
        if self.widgets.overlay_open() {
            self.widgets.handle_key(Panel::Files, key);
            if let Some((path, line)) = self.widgets.take_edit() {
                fx.push(Effect::Edit { path, line });
            }
            for (before, after) in self.widgets.take_comment_changes() {
                self.comment_in_prompt(before, after);
            }
            if let Some(text) = self.widgets.take_copy() {
                self.copy(text, &mut fx);
            }
            return fx;
        }
        if self.github.is_open() {
            if let Some(out) = self.github.key(key) {
                self.take_github(out, &mut fx);
            }
            return fx;
        }
        if self.transcript.is_some() {
            self.transcript_key(key, &mut fx);
            return fx;
        }
        if let Some(reader) = self.reader.as_mut() {
            if let ReaderAction::Close = reader.key(key) {
                self.reader = None;
            } else if let Some(text) = reader.take_copy() {
                self.copy(text, &mut fx);
            }
            return fx;
        }
        // Alt-. / Alt-, widen / narrow the focused pane (the keys' > and <, with the shift too).
        // Alt and punctuation reach term in any terminal and through tmux; font zoom and tmux's
        // own bindings take the Ctrl ones.
        if key.modifiers.contains(KeyModifiers::ALT)
            && let KeyCode::Char(c @ ('.' | '>' | ',' | '<')) = key.code
        {
            let by = if matches!(c, '.' | '>') {
                RESIZE_STEP
            } else {
                -RESIZE_STEP
            };
            self.resize_pane(by);
            return fx;
        }
        // Harpoon's marks: Ctrl-E opens and closes the quick menu (as harpoon's is usually
        // bound; End goes to the line's end), Ctrl-1…Ctrl-9 show a marked session (kitty's
        // keyboard protocol tells them from the digits; elsewhere Alt-1…Alt-9 or Ctrl-G and the
        // digit), Alt-m marks the shown one, Alt-' lists them too.
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('e') {
            self.overlay = if self.overlay == Overlay::Marks {
                Overlay::None
            } else {
                self.marks_sel = 0;
                Overlay::Marks
            };
            return fx;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && let KeyCode::Char(d @ '1'..='9') = key.code
        {
            self.jump_to_mark(usize::from(d as u8 - b'1'), &mut fx);
            return fx;
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && self.overlay != Overlay::Marks
            && let KeyCode::Char(c @ ('1'..='9' | 'm' | '\'')) = key.code
        {
            match c {
                'm' => self.toggle_mark(),
                '\'' => {
                    self.marks_sel = 0;
                    self.overlay = Overlay::Marks;
                }
                d => self.jump_to_mark(usize::from(d as u8 - b'1'), &mut fx),
            }
            return fx;
        }
        // Ctrl-G then a digit: the same, where the terminal takes Alt-digit (kitty does). The
        // digit counts whether or not Ctrl-G got the sidebar the keys.
        let ctrl_g = key.code == KeyCode::Char('g') && key.modifiers == KeyModifiers::CONTROL;
        let after_ctrl_g = std::mem::replace(&mut self.after_ctrl_g, ctrl_g);
        if after_ctrl_g
            && self.overlay == Overlay::None
            && let KeyCode::Char(d @ '1'..='9') = key.code
        {
            if self.focus == Some(Focus::Sessions) {
                self.focus = None;
            }
            self.sessions_wanted = false;
            self.jump_to_mark(usize::from(d as u8 - b'1'), &mut fx);
            return fx;
        }
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if !ctrl_c && self.quit_armed_until > 0 {
            self.quit_armed_until = 0;
            if self
                .notice
                .as_ref()
                .is_some_and(|n| n.text.starts_with("press ctrl-c"))
            {
                self.notice = None;
            }
        }
        match self.overlay {
            Overlay::Help => self.overlay = Overlay::None,
            Overlay::Switcher => self.switcher_key(key, &mut fx),
            Overlay::Marks => self.marks_key(key, &mut fx),
            Overlay::Setup => self.setup_key(key, &mut fx),
            Overlay::None if self.note.is_some() => self.note_key(key, &mut fx),
            Overlay::None => match self.focus {
                Some(focus) => self.panel_key(focus, key, &mut fx),
                None => self.editor_key(key, &mut fx),
            },
        }
        self.sync_prompt();
        fx
    }

    /// Puts text on the clipboard and says so.
    pub(super) fn copy(&mut self, text: String, fx: &mut Vec<Effect>) {
        let lines = text.lines().count().max(1);
        fx.push(Effect::Copy(text));
        let said = if lines == 1 {
            "copied".to_string()
        } else {
            format!("copied {lines} lines")
        };
        self.notify(&said, false);
    }

    /// The sidebar lists all the apps, or the first few again.
    pub(super) fn toggle_apps(&mut self) {
        self.apps_open = !self.apps_open;
        self.dirty = true;
    }

    /// The mouse wheel scrolls the chat (or the open diff, or the terminal panel under the
    /// pointer); dragging over the chat selects text and copies it when let go; a click on the
    /// sidebar's `▸ N more` shows the rest of the apps. Other mouse events are ignored.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        let mut fx = Vec::new();
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind
            && self
                .apps_toggle_at
                .is_some_and(|a| a.contains(ratatui::layout::Position::new(mouse.column, mouse.row)))
        {
            self.toggle_apps();
            return fx;
        }
        let down = match mouse.kind {
            MouseEventKind::ScrollUp => false,
            MouseEventKind::ScrollDown => true,
            MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left) => {
                self.mouse_select(mouse, &mut fx);
                return fx;
            }
            _ => return fx,
        };
        self.dirty = true;
        if self.github.is_open() {
            self.github.wheel(down);
        } else if let Some(t) = self.transcript.as_mut() {
            t.wheel(down);
        } else if let Some(r) = self.reader.as_mut() {
            r.wheel(down);
        } else if self
            .terminal_area
            .is_some_and(|a| a.contains(ratatui::layout::Position::new(mouse.column, mouse.row)))
        {
            let code = if down { KeyCode::Down } else { KeyCode::Up };
            for _ in 0..WHEEL_LINES {
                self.widgets
                    .handle_key(Panel::Terminal, KeyEvent::new(code, KeyModifiers::NONE));
            }
        } else if self.widgets.overlay_open() {
            // Ctrl-E / Ctrl-Y: the view moves, the cursor only as far as it has to.
            let code = KeyCode::Char(if down { 'e' } else { 'y' });
            for _ in 0..WHEEL_LINES {
                self.widgets
                    .handle_key(Panel::Files, KeyEvent::new(code, KeyModifiers::CONTROL));
            }
        } else if down {
            self.chat.scroll_down(WHEEL_LINES);
        } else {
            self.chat.scroll_up(WHEEL_LINES);
        }
        fx
    }

    /// The left button over the chat: pressed starts a selection, dragged extends it (past the
    /// pane's top or bottom it scrolls), let go copies what it covers.
    pub(super) fn mouse_select(&mut self, mouse: MouseEvent, fx: &mut Vec<Effect>) {
        let shown = !self.github.is_open()
            && !self.widgets.overlay_open()
            && self.transcript.is_none()
            && self.reader.is_none()
            && self.overlay == Overlay::None;
        let Some(area) = self.chat_area.filter(|_| shown) else {
            return;
        };
        let inside = area.contains(Position::new(mouse.column, mouse.row));
        let (col, row) = (
            mouse.column.saturating_sub(area.x).min(area.width - 1),
            mouse.row.saturating_sub(area.y).min(area.height - 1),
        );
        match mouse.kind {
            MouseEventKind::Down(_) if inside => {
                self.dirty = true;
                self.chat.select_to(self.chat.at_cell(col, row), false);
            }
            MouseEventKind::Down(_) => {
                self.dirty |= self.chat.selecting();
                self.chat.clear_selection();
            }
            MouseEventKind::Drag(_) if self.chat.selecting() => {
                self.dirty = true;
                if mouse.row < area.y {
                    self.chat.scroll_up(1);
                } else if mouse.row >= area.bottom() {
                    self.chat.scroll_down(1);
                }
                self.chat.select_to(self.chat.at_cell(col, row), true);
            }
            MouseEventKind::Up(_) if self.chat.selecting() => {
                self.dirty = true;
                if let Some(text) = self.chat.selected_text() {
                    self.copy(text, fx);
                }
                // A click on a file's line or a card's opens it.
                let pick = self.chat.clicked().and_then(|at| self.chat.pick_at(at));
                self.chat.clear_selection();
                if let Some(pick) = pick {
                    self.open_pick(pick, fx);
                }
            }
            _ => {}
        }
    }

    pub fn handle_paste(&mut self, text: &str) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.paste(text, &mut fx);
        fx
    }

    pub(super) fn paste(&mut self, text: &str, fx: &mut Vec<Effect>) {
        self.dirty = true;
        if let Some(secret) = self.secret.as_mut() {
            secret.paste(text);
            return;
        }
        if self.github.is_open() {
            self.github.paste(text);
            return;
        }
        match self.overlay {
            Overlay::Switcher => text
                .chars()
                .filter(|c| !c.is_control())
                .for_each(|c| self.switcher.type_char(c)),
            Overlay::Help | Overlay::Marks => self.overlay = Overlay::None,
            Overlay::Setup => {}
            Overlay::None if self.note.is_some() => {
                if let Some(note) = self.note.as_mut() {
                    note.text.extend(text.chars().filter(|c| !c.is_control()));
                }
            }
            Overlay::None if self.prompt.engaged && self.active_form().is_some() => {
                if let Some(r) = self.active_form().map(str::to_string) {
                    let targets = self.form_targets(&r);
                    self.forms.paste(&r, &targets, text, self.now_ms);
                    self.chat.reveal_focus(&r);
                }
            }
            Overlay::None => {
                if self.focus.is_none() && !self.widgets.overlay_open() && !self.prompt.engaged {
                    self.prompt.typed_ms = self.now_ms;
                    // Files dropped on the terminal arrive as their paths. An empty paste is what
                    // some terminals send when the clipboard holds only an image.
                    if text.is_empty() {
                        return fx.push(Effect::PasteImage);
                    }
                    if let Some(paths) = attach::image_paths(text, &self.cwd) {
                        return self.attach(paths);
                    }
                    self.editor.insert_str(text);
                    self.histories.of(&self.session).reset();
                }
            }
        }
    }

    /// Up with messages waiting: asks for this app's back, to edit them (`redraft`).
    pub(super) fn unqueue(&mut self, fx: &mut Vec<Effect>) -> bool {
        if !self.waiting.iter().any(|w| w.app == self.app_id) {
            return false;
        }
        fx.push(Effect::Unqueue {
            session: self.session.clone(),
        });
        true
    }

    /// The messages taken back out of the queue go into the editor, one per paragraph, before
    /// whatever was typed meanwhile; their images come along.
    pub(super) fn redraft(&mut self, inputs: Vec<WaitingInput>) {
        let app = self.app_id.clone();
        self.waiting.retain(|w| w.app != app);
        if inputs.is_empty() {
            return;
        }
        self.dirty = true;
        let mut parts = Vec::new();
        for input in inputs {
            let next = self.attachments.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
            let images = input.images.unwrap_or_default();
            let (text, attached) = attach::redraft(&input.text, &images, next);
            self.attachments.extend(attached);
            parts.push(text);
        }
        let typed = self.editor.text().trim().to_string();
        if !typed.is_empty() {
            parts.push(typed);
        }
        self.editor.set_text(&parts.join("\n\n"));
        self.histories.of(&self.session).reset();
    }

    /// Pastes images into the draft: each gets a number and its `[image N]` marker goes in at
    /// the cursor, so the message says where it belongs.
    pub(super) fn attach(&mut self, paths: Vec<String>) {
        self.dirty = true;
        self.prompt.typed_ms = self.now_ms;
        for path in paths {
            let n = self.attachments.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
            self.attachments.push((n, path));
            let before = self.editor.text()[..self.editor.cursor()]
                .chars()
                .next_back();
            if before.is_some_and(|c| !c.is_whitespace()) {
                self.editor.insert_char(' ');
            }
            self.editor.insert_str(&attach::marker(n));
            self.editor.insert_char(' ');
        }
        self.histories.of(&self.session).reset();
    }

    pub(super) fn panel_key(&mut self, focus: Focus, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => self.cycle_focus(1),
            // Back a pane; from the first, back to the editor (where Shift-Tab is approvals').
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Char('c') if ctrl => self.ctrl_c(fx),
            KeyCode::Char('g') if ctrl => self.toggle_sessions_focus(),
            _ => match focus {
                Focus::Panel(panel) => match self.widgets.handle_key(panel, key) {
                    WidgetAction::Redraw => {}
                    WidgetAction::Leave => self.focus = None,
                    // A new todo is written in the editor, as /todo.
                    WidgetAction::AddTodo => {
                        self.focus = None;
                        self.editor.set_text("/todo ");
                    }
                    WidgetAction::Todo(args) => self.change_todos(args, fx),
                },
                // `a` shows or hides the apps past the first few, under the sessions.
                Focus::Sessions if key.code == KeyCode::Char('a') && key.modifiers.is_empty() => {
                    self.toggle_apps();
                }
                Focus::Sessions => match self.sessions.key(key) {
                    SessionsAction::Redraw => {}
                    SessionsAction::Leave => self.focus = None,
                    SessionsAction::Open(info) => {
                        self.focus = None;
                        self.open_session(*info, fx);
                    }
                    SessionsAction::New => {
                        self.focus = None;
                        fx.push(Effect::CreateSession {
                            cwd: self.cwd.clone(),
                        });
                    }
                },
                Focus::Agents => self.agents_key(key, fx),
                Focus::Jobs => self.jobs_key(key),
                Focus::Chat => self.chat_pick_key(key, fx),
            },
        }
    }

    /// Keys while the chat's files and replies have focus: move between them, open a file's
    /// diff or a reply as markdown, copy a reply's markdown or a file's path. Ctrl-O on a file
    /// expands it into the file view; elsewhere it expands the chat.
    pub(super) fn chat_pick_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
            match self.chat.picked_file().map(str::to_string) {
                Some(path) => self.widgets.open_path(&path),
                None => self.toggle_expanded(),
            }
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.focus = None,
            KeyCode::Up | KeyCode::Char('k') => self.chat.move_pick(-1),
            KeyCode::Down | KeyCode::Char('j') => self.chat.move_pick(1),
            KeyCode::Home | KeyCode::Char('g') => self.chat.pick(0),
            KeyCode::End | KeyCode::Char('G') => self.chat.pick(-1),
            KeyCode::Enter | KeyCode::Char(' ' | 'v' | 'V') => {
                if let Some(path) = self.chat.picked_file().map(str::to_string) {
                    self.widgets.open_path(&path);
                } else if let Some(dev) = self.chat.picked_ref().cloned() {
                    self.open_pick(Pick::Ref(dev), fx);
                } else if let Some(text) = self.picked_markdown() {
                    self.reader = Some(Reader::new("reply", &text));
                }
            }
            KeyCode::Char('y') => {
                if let Some(path) = self.chat.picked_file().map(str::to_string) {
                    self.copy(path, fx);
                } else if let Some(dev) = self.chat.picked_ref() {
                    let text = match dev {
                        DevRef::Commit { sha, .. } => sha.clone(),
                        DevRef::Github { url: Some(url), .. } => url.clone(),
                        DevRef::Github { .. } => dev.token(),
                    };
                    self.copy(text, fx);
                } else if let Some(text) = self.picked_markdown() {
                    self.copy(text, fx);
                }
            }
            _ => {}
        }
    }

    /// Opens a file in the diff view, a commit's changes there too, an issue or pull request
    /// in the GitHub view; one of another repository in the browser.
    pub(super) fn open_pick(&mut self, pick: Pick, fx: &mut Vec<Effect>) {
        self.dirty = true;
        match pick {
            Pick::File(path) => self.widgets.open_path(&path),
            Pick::Reply => {}
            Pick::Ref(DevRef::Commit { sha, title }) => self.widgets.open_commit(&sha, &title),
            Pick::Ref(DevRef::Github {
                pr,
                number,
                repo,
                url,
            }) => {
                let elsewhere = match (repo.as_deref(), self.github.repo()) {
                    (Some(theirs), Some(ours)) => !theirs.eq_ignore_ascii_case(ours),
                    _ => false,
                };
                match url {
                    Some(url) if elsewhere => self.open_url(url, fx),
                    _ if elsewhere => self.notify(
                        format!("#{number} is in {}", repo.unwrap_or_default()),
                        false,
                    ),
                    _ => {
                        let kind = if pr { Kind::Pr } else { Kind::Issue };
                        self.github.open_number(kind, number);
                    }
                }
            }
        }
    }

    /// The picked reply as markdown; None (and a word why) when it has no text.
    pub(super) fn picked_markdown(&mut self) -> Option<String> {
        let r = self.chat.picked_reply()?;
        let text = self
            .chat
            .item(r)
            .map(crate::markdown::item_markdown)
            .filter(|t| !t.trim().is_empty());
        if text.is_none() {
            self.notify("this reply has no text", false);
        }
        text
    }

    // ---- subagents ----

    /// Ctrl-X, or Down from an empty editor: the keys go to the agents list.
    pub(super) fn focus_agents(&mut self) {
        if self.agents.visible(self.now_ms) {
            self.agents.focus(self.now_ms);
            self.focus = Some(Focus::Agents);
        } else {
            self.notify("no subagents running", false);
        }
    }

    // ---- background commands ----

    /// Down past the editor (and the subagents): the keys go to the background commands, and
    /// the terminal panel shows the selected one.
    pub(super) fn focus_jobs(&mut self) {
        let rows = self.widgets.jobs(self.now_ms);
        let id = self.jobs.focus(&rows);
        if id.is_some() {
            self.terminal_on = true;
            self.widgets.show_job(id);
            self.focus = Some(Focus::Jobs);
        }
    }

    pub(super) fn jobs_key(&mut self, key: KeyEvent) {
        let rows = self.widgets.jobs(self.now_ms);
        match self.jobs.key(key, &rows) {
            JobsAction::Redraw => {}
            JobsAction::Show(id) => self.widgets.show_job(Some(id)),
            JobsAction::Up if self.agents.visible(self.now_ms) => {
                self.widgets.show_job(None);
                self.agents.focus(self.now_ms);
                self.focus = Some(Focus::Agents);
            }
            JobsAction::Up | JobsAction::Leave => {
                self.widgets.show_job(None);
                self.focus = None;
            }
            // It ends (and leaves the list) once its file says so.
            JobsAction::Stop(id) => match self.widgets.stop_job(&id) {
                Ok(_) => self.notify("stopping…", false),
                Err(why) => self.notify(format!("can't stop it: {why}"), true),
            },
            JobsAction::Open(id) => {
                self.widgets.show_job(Some(id));
                if self.terminal_area.is_some() {
                    self.focus = Some(Focus::Panel(Panel::Terminal));
                } else {
                    self.notify("too narrow for the terminal panel", false);
                }
            }
        }
    }

    pub(super) fn agents_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        // Down from the last subagent goes on to the background commands.
        if matches!(key.code, KeyCode::Down | KeyCode::Char('j'))
            && self.agents.on_last(self.now_ms)
            && !self.widgets.jobs(self.now_ms).is_empty()
        {
            self.focus_jobs();
            return;
        }
        match self.agents.key(key, self.now_ms) {
            AgentsAction::Redraw => {}
            AgentsAction::Leave => self.focus = None,
            AgentsAction::Transcript(id) => {
                let Some(agent) = self.agents.get(&id).cloned() else {
                    return;
                };
                self.focus = None;
                fx.push(Effect::AgentView {
                    session: self.session.clone(),
                    agent: agent.harness.clone(),
                    on: true,
                });
                self.transcript = Some(Transcript::new(agent));
            }
            AgentsAction::Stop(id) => {
                self.notify("stopping…", false);
                fx.push(Effect::Action {
                    session: self.session.clone(),
                    item: "agents".into(),
                    action: "neo.cancel_agent".into(),
                    args: Some(serde_json::json!({ "id": id })),
                });
            }
        }
    }

    /// Keys while a subagent's transcript is open: scrolling, expanding, and the way out.
    pub(super) fn transcript_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter if !ctrl => {
                if let Some(t) = self.transcript.take() {
                    fx.push(Effect::AgentView {
                        session: self.session.clone(),
                        agent: t.agent.harness,
                        on: false,
                    });
                }
            }
            KeyCode::Char('c') if ctrl => self.ctrl_c(fx),
            KeyCode::Char('o') if ctrl => self.toggle_expanded(),
            KeyCode::Char('t') if ctrl => self.show_thoughts = !self.show_thoughts,
            _ if !ctrl => {
                if let Some(t) = self.transcript.as_mut() {
                    t.scroll_key(key);
                }
            }
            _ => {}
        }
    }
}
