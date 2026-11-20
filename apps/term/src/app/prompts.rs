use super::*;

impl App {
    // ---- prompts (items with buttons) ----

    /// The prompt the keys answer: one picked by hand, else the oldest one still pending.
    pub(super) fn active_ref(&self) -> Option<&str> {
        if let Some(r) = &self.prompt.selected
            && self.chat.is_prompt(r)
        {
            return Some(r);
        }
        self.chat
            .pending_prompts()
            .into_iter()
            .find(|r| !self.prompt.dismissed.iter().any(|d| d == r))
    }

    /// The pending prompts that are not dismissed, and the active one's place among them.
    pub(super) fn queue(&self) -> (usize, usize) {
        let pending: Vec<&str> = self
            .chat
            .pending_prompts()
            .into_iter()
            .filter(|r| !self.prompt.dismissed.iter().any(|d| d == r))
            .collect();
        let at = self
            .active_ref()
            .and_then(|a| pending.iter().position(|r| *r == a));
        (at.map_or(1, |i| i + 1), pending.len().max(1))
    }

    /// A prompt takes the editor's place once the editor is empty and the user has paused typing
    /// (at once when they weren't typing), and gives it back, draft intact, when it is answered
    /// or dismissed. A draft holds it back until it is sent or stashed, or Ctrl-P asks for it.
    pub(super) fn update_engaged(&mut self) {
        let quiet = self.now_ms - self.prompt.typed_ms >= TYPING_PAUSE_MS;
        let free = self.editor.is_empty() && self.attachments.is_empty();
        let engaged = self.active_ref().is_some()
            && self.setting_input.is_none()
            && (self.prompt.engaged || quiet && free);
        if engaged != self.prompt.engaged {
            self.prompt.engaged = engaged;
            self.dirty = true;
        }
        // An edit to answer shows in its own file view above the input while its card has the
        // editor's place.
        let edit = self.active_ref().filter(|_| engaged).and_then(|r| {
            let diff = self.chat.diff_of(r)?;
            Some((r.to_string(), diff.path.clone()?, diff.lines.clone()))
        });
        let before = self.widgets.reviewing().map(str::to_string);
        match edit {
            Some((r, path, lines)) => self.widgets.open_review(&r, &path, &lines),
            None => self.widgets.hide_review(),
        }
        // The layout made before this frame's prompt was known: the next one has the view.
        if self.widgets.reviewing() != before.as_deref() {
            self.dirty = true;
        }
    }

    /// The edit of the card answered now, while it is shown: its file view takes the keys
    /// that don't answer the card.
    pub(super) fn review_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) -> bool {
        if !self.review_shown {
            return false;
        }
        let Some(view) = self.widgets.review_mut() else {
            return false;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Enter, Tab, Esc, ←/→ and the digits answer; Ctrl keys are the app's, but for the
        // ones that scroll. Something started in the view (a comment, a selection) has them all.
        let answers = match key.code {
            KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => true,
            KeyCode::Left | KeyCode::Right => true,
            KeyCode::Char('1'..='9') => !ctrl,
            KeyCode::Char(c) if ctrl => !matches!(c, 'd' | 'u' | 'f' | 'b' | 'e' | 'y'),
            _ => false,
        };
        if answers && !view.busy() {
            return false;
        }
        // q would close it: it closes with the card instead.
        view.key(key);
        if let Some((path, line)) = view.take_edit() {
            fx.push(Effect::Edit { path, line });
        }
        // Its comments go with the answer, not into the prompt.
        view.take_comment_changes();
        if let Some(text) = view.take_copy() {
            self.copy(text, fx);
        }
        true
    }

    /// A prompt is open but the user is typing: the editor stays, with a hint above it.
    pub(super) fn prompt_waits(&self) -> bool {
        !self.prompt.engaged && self.active_ref().is_some()
    }

    pub(super) fn active_buttons(&self) -> &[ButtonRef] {
        self.active_ref().map_or(&[], |r| self.chat.buttons_of(r))
    }

    pub(super) fn focused_button(&self) -> Option<&ButtonRef> {
        self.active_buttons().get(self.prompt.button)
    }

    /// Enter, Space or the digit on a button whose card stays (`/settings`): pressed with
    /// `stay`, the card left open and the focus on it, for the daemon to draw it again as it is
    /// now. One that asks for text has it written in the editor first.
    pub(super) fn press_setting(&mut self, index: usize, fx: &mut Vec<Effect>) {
        self.prompt.button = index;
        let Some(button) = self.focused_button().cloned() else {
            return;
        };
        let Some(input) = button.input else {
            return self.press_staying(None, fx);
        };
        let Some(r) = self.active_ref().map(str::to_string) else {
            return;
        };
        // The draft waits; the editor has the setting's text, the card gives it the keys.
        let draft = (
            self.editor.text().to_string(),
            std::mem::take(&mut self.attachments),
        );
        self.editor
            .set_text(input.value.as_deref().unwrap_or_default());
        let name = button
            .label
            .split(" · ")
            .next()
            .unwrap_or(&button.label)
            .to_string();
        self.setting_input = Some(SettingInput {
            r#ref: r,
            button: index,
            multiline: input.multiline == Some(true),
            name,
            draft,
        });
        self.prompt.engaged = false;
        self.dirty = true;
    }

    /// Presses the focused button with `stay`, and `value`: the text it asked for.
    fn press_staying(&mut self, value: Option<String>, fx: &mut Vec<Effect>) {
        let (Some(r), Some(button)) = (
            self.active_ref().map(str::to_string),
            self.focused_button().cloned(),
        ) else {
            return;
        };
        let mut args = button.args;
        if let Some(serde_json::Value::Object(map)) = args.as_mut() {
            map.insert("stay".into(), true.into());
            if let Some(value) = value {
                map.insert("value".into(), value.into());
            }
        }
        fx.push(Effect::Action {
            session: self.session.clone(),
            item: r,
            action: button.action,
            args,
        });
    }

    /// Keys while a setting's text is written in the editor: Enter keeps it (Shift-Enter or
    /// Alt-Enter is a new line where it takes several), Esc gives up; true when the key was
    /// one of those, else it edits the text like any.
    pub(super) fn setting_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) -> bool {
        let Some(input) = self.setting_input.as_ref() else {
            return false;
        };
        let newline = key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter if newline && input.multiline => self.editor.newline(),
            KeyCode::Enter => {
                let text = self.editor.text().to_string();
                if let Some(button) = self.end_setting_input() {
                    self.prompt.button = button;
                    self.press_staying(Some(text), fx);
                }
            }
            KeyCode::Esc => {
                self.end_setting_input();
            }
            _ => return false,
        }
        true
    }

    /// The setting's text is done with: the draft comes back and the card takes the keys
    /// again. The button it was for, when its card is still the active one.
    pub(super) fn end_setting_input(&mut self) -> Option<usize> {
        let input = self.setting_input.take()?;
        let (text, attachments) = input.draft;
        self.editor.set_text(&text);
        self.attachments = attachments;
        self.prompt.engaged = self.active_ref().is_some();
        self.dirty = true;
        (self.active_ref() == Some(input.r#ref.as_str())).then_some(input.button)
    }

    /// What the line above the editor says while a setting's text is written there.
    pub(super) fn setting_hint(&self) -> Option<String> {
        let input = self.setting_input.as_ref()?;
        let keys = if input.multiline {
            "Enter Save · Shift+Enter New line · Esc Cancel"
        } else {
            "Enter Save · Esc Cancel"
        };
        Some(format!("{} · {keys}", input.name))
    }

    /// The active prompt is a choice long enough to be filtered by typing.
    pub(super) fn filtering(&self) -> bool {
        choice::is_long(self.active_buttons())
    }

    /// The options of the active prompt that match the filter (all of them for a card that
    /// isn't filtered).
    pub(super) fn shown_buttons(&self) -> Vec<usize> {
        let buttons = self.active_buttons();
        if choice::is_long(buttons) {
            choice::matching(buttons, &self.prompt.filter)
        } else {
            (0..buttons.len()).collect()
        }
    }

    /// The filter changed: the focus goes to the first match (the current option when it is
    /// empty) and the window of the list starts over.
    pub(super) fn refilter(&mut self) {
        if let Some(at) = choice::refocus(self.active_buttons(), &self.prompt.filter) {
            self.prompt.button = at;
        }
        self.prompt.panel_top.set(None);
    }

    /// A typed character, Backspace or Ctrl-U on a filtered choice.
    pub(super) fn edit_filter(&mut self, edit: impl FnOnce(&mut String)) {
        edit(&mut self.prompt.filter);
        self.refilter();
    }

    /// A new active prompt starts on its current or first button and is scrolled into view.
    pub(super) fn sync_prompt(&mut self) {
        let current = self.active_ref().map(str::to_string);
        // The card was answered elsewhere, or another one took over: the note is moot.
        if self
            .note
            .as_ref()
            .is_some_and(|n| Some(&n.r#ref) != current.as_ref())
        {
            self.note = None;
        }
        if current != self.prompt.seen {
            if let Some(old) = self.prompt.seen.clone() {
                self.forms.blur(&old, self.now_ms);
            }
            // A picker starts on the value in effect now, anything else on its first button.
            self.prompt.button = current
                .as_deref()
                .and_then(|r| self.chat.buttons_of(r).iter().position(|b| b.current))
                .unwrap_or(0);
            if current.is_some() && self.prompt.selected.is_none() {
                self.chat.follow();
            }
            self.prompt.panel_top.set(None);
            self.prompt.filter.clear();
            self.prompt.seen = current.clone();
            if let Some(r) = current.filter(|r| self.chat.has_targets(r)) {
                let targets = self.form_targets(&r);
                self.forms.enter(&r, &targets, self.now_ms);
                self.chat.reveal_focus(&r);
            }
        }
        self.update_engaged();
    }

    pub(super) fn move_button(&mut self, delta: isize) {
        if self.filtering() {
            let shown = self.shown_buttons();
            self.prompt.button = choice::step(&shown, self.prompt.button, delta);
            return;
        }
        let n = self.active_buttons().len();
        if n > 0 {
            self.prompt.button =
                (self.prompt.button as isize + delta).rem_euclid(n as isize) as usize;
        }
    }

    pub(super) fn press(&mut self, index: usize, fx: &mut Vec<Effect>) {
        self.press_with(index, None, fx);
    }

    /// Presses a button; `note` (what the user added for the agent) goes along in its args.
    pub(super) fn press_with(&mut self, index: usize, note: Option<&str>, fx: &mut Vec<Effect>) {
        let Some(r) = self.active_ref().map(str::to_string) else {
            return;
        };
        let Some(button) = self.chat.buttons_of(&r).get(index).cloned() else {
            return;
        };
        // The comments written on its edit, after what was typed as the note.
        let comments = self.widgets.take_review_notes(&r);
        let note = match (note, comments.is_empty() || !takes_note(&button)) {
            (note, true) => note.map(str::to_string),
            (Some(note), false) => Some(format!("{note}\n{}", comments.join("\n"))),
            (None, false) => Some(comments.join("\n")),
        };
        let note = note.as_deref();
        if let Some(text) = button.copy_text() {
            fx.push(Effect::Copy(text.to_string()));
            self.notify("copied", false);
        } else {
            let mut args = button.args;
            if let (Some(note), Some(serde_json::Value::Object(map))) = (note, args.as_mut()) {
                map.insert("note".into(), note.into());
            }
            fx.push(Effect::Action {
                session: self.session.clone(),
                item: r.clone(),
                action: button.action,
                args,
            });
        }
        self.prompt.dismissed.push(r);
        self.prompt.selected = None;
        self.sync_prompt();
    }

    /// Tab on a card: a one-line note for the agent under it, sent with the focused button.
    /// Cards that aren't the daemon's questions keep Tab as "next button".
    pub(super) fn tab_on_prompt(&mut self) {
        let Some(r) = self.active_ref().map(str::to_string) else {
            return;
        };
        let button = self.prompt.button;
        if self.filtering() && !self.shown_buttons().contains(&button) {
            return;
        }
        if self.chat.buttons_of(&r).get(button).is_some_and(takes_note) {
            self.note = Some(Note {
                r#ref: r,
                button,
                text: String::new(),
            });
            self.chat.follow();
        } else {
            self.move_button(1);
        }
    }

    /// Keys while a note is being typed: text, Enter answers with it, Esc goes back to the
    /// buttons.
    pub(super) fn note_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(note) = self.note.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.note = None,
            KeyCode::Char('c') if ctrl => self.note = None,
            KeyCode::Char('u') if ctrl => note.text.clear(),
            KeyCode::Char('w') if ctrl => {
                let kept = note.text.trim_end().rfind(' ').map_or(0, |i| i + 1);
                note.text.truncate(kept);
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                note.text.push(c);
            }
            KeyCode::Backspace => {
                note.text.pop();
            }
            KeyCode::Enter => {
                let Some(Note { button, text, .. }) = self.note.take() else {
                    return;
                };
                let text = text.trim();
                self.press_with(button, (!text.is_empty()).then_some(text), fx);
            }
            _ => {}
        }
    }

    /// Ctrl-P / Ctrl-N: pick an older (or newer) item with buttons.
    pub(super) fn select_prompt(&mut self, delta: isize) -> bool {
        let refs: Vec<String> = self
            .chat
            .prompt_refs()
            .into_iter()
            .map(str::to_string)
            .collect();
        if refs.is_empty() {
            return false;
        }
        let current = self
            .active_ref()
            .and_then(|a| refs.iter().position(|r| r == a));
        let target = match (current, delta) {
            (Some(i), d) if d < 0 => Some(i.saturating_sub(1)),
            (Some(i), _) if i + 1 < refs.len() => Some(i + 1),
            (Some(_), _) => None,
            (None, d) if d < 0 => Some(refs.len() - 1),
            (None, _) => return true,
        };
        match target {
            Some(i) => {
                let r = refs[i].clone();
                self.prompt.dismissed.retain(|d| *d != r);
                self.prompt.selected = Some(r.clone());
                // Asked for by hand: no waiting for the draft to go.
                self.prompt.engaged = true;
                self.dirty = true;
                self.chat.reveal(&r);
            }
            None => {
                self.prompt.selected = None;
                self.prompt.dismissed.clear();
            }
        }
        self.sync_prompt();
        true
    }

    // ---- neoui forms ----

    /// The active prompt when it is a neoui item: keys move through its targets.
    pub(super) fn active_form(&self) -> Option<&str> {
        self.active_ref().filter(|r| self.chat.has_targets(r))
    }

    /// The focus targets of an item's neoui docs, as drawn with their visual state.
    pub(super) fn form_targets(&self, r: &str) -> Vec<UiTarget> {
        let Some(item) = self.chat.item(r).or_else(|| self.chat.surface(r)) else {
            return Vec::new();
        };
        let opts = Options {
            neoui: self.forms.doc_states(r),
            ..Options::default()
        };
        neoui_targets(item, &opts)
    }

    /// The daemon sent items again: values it has caught up with are not held here any more.
    pub(super) fn refresh_forms(&mut self) {
        let refs: Vec<String> = self.forms.all().keys().cloned().collect();
        for r in refs {
            if self.chat.has_targets(&r) {
                let targets = self.form_targets(&r);
                self.forms.refresh(&r, &targets, self.now_ms);
            }
        }
    }

    pub(super) fn input_effect(&self, p: Pending) -> Effect {
        Effect::Action {
            session: self.session.clone(),
            item: p.item,
            action: "neoui.input".into(),
            args: Some(serde_json::json!({"doc": p.doc, "key": p.key, "value": p.value})),
        }
    }

    /// Sends the values whose pause is over.
    pub(super) fn send_due(&mut self, fx: &mut Vec<Effect>) {
        if !self.forms.has_pending() {
            return;
        }
        for p in self.forms.due(self.now_ms) {
            fx.push(self.input_effect(p));
        }
    }

    /// Keys while a neoui item has the editor's place.
    pub(super) fn form_key(&mut self, r: &str, key: KeyEvent, fx: &mut Vec<Effect>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if let KeyCode::Char(c) = key.code
            && ctrl
            && matches!(c, 'c' | 'p' | 'n' | 'o' | 't' | 'l' | 'g' | 'x')
        {
            match c {
                'c' => self.ctrl_c(fx),
                'p' => {
                    self.select_prompt(-1);
                }
                'n' => {
                    self.select_prompt(1);
                }
                _ => {
                    self.ctrl_key(c, fx);
                }
            }
            return;
        }
        match key.code {
            KeyCode::Up if shift => return self.chat.scroll_up(1),
            KeyCode::Down if shift => return self.chat.scroll_down(1),
            _ => {}
        }
        let targets = self.form_targets(r);
        let out = self.forms.key(r, &targets, key, self.now_ms);
        if !out.taken {
            match key.code {
                KeyCode::PageUp => self.chat.page_up(),
                KeyCode::PageDown => self.chat.page_down(),
                KeyCode::Home => self.chat.to_top(),
                KeyCode::End => self.chat.follow(),
                _ => {}
            }
            return;
        }
        self.chat.reveal_focus(r);
        self.apply_outcome(r, out, fx);
        self.send_due(fx);
    }

    pub(super) fn apply_outcome(&mut self, r: &str, out: Outcome, fx: &mut Vec<Effect>) {
        if let Some(url) = out.url {
            self.open_url(url, fx);
        }
        if let Some(text) = out.copy {
            fx.push(Effect::Copy(text));
            self.notify("copied", false);
        }
        if let Some((doc, key)) = out.press {
            // What was typed goes first, so the press finds it.
            for p in self.forms.flush(r) {
                fx.push(self.input_effect(p));
            }
            fx.push(Effect::Action {
                session: self.session.clone(),
                item: r.to_string(),
                action: "neoui.press".into(),
                args: Some(serde_json::json!({"doc": doc, "key": key})),
            });
        }
        if out.leave {
            self.esc(fx);
        }
    }

    /// A link in the browser, for the schemes that make sense there; anything else is shown.
    pub(super) fn open_url(&mut self, url: String, fx: &mut Vec<Effect>) {
        let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
        if matches!(scheme.as_str(), "http" | "https" | "mailto") {
            self.notify(format!("opening {url}"), false);
            fx.push(Effect::OpenUrl(url));
        } else {
            self.notify(format!("link: {url}"), false);
        }
    }

    /// One line under a form: what the focused target does with the keys.
    pub(super) fn form_hint(&self, r: &str, width: u16) -> Line<'static> {
        let targets = self.form_targets(r);
        let focused = self.forms.focused(r, &targets);
        let mut parts: Vec<String> = Vec::new();
        match focused {
            Some(t) => {
                if !t.label.is_empty() {
                    parts.push(t.label.clone());
                }
                parts.push(
                    match &t.kind {
                        UiKind::Input { .. } | UiKind::Date => "type · enter submits",
                        UiKind::TextArea => "type · enter newline",
                        UiKind::Select { .. } if self.forms.is_open(r, t) => {
                            "↑/↓ choose · enter picks"
                        }
                        UiKind::Select { .. } => "enter opens · ←/→ choose",
                        UiKind::Radio { .. } => "←/→ choose",
                        UiKind::CheckBox | UiKind::Switch => "space toggles",
                        UiKind::CheckBoxGroup { .. } | UiKind::SwitchGroup { .. } => {
                            "↑/↓ move · space toggles"
                        }
                        UiKind::Slider { .. } => "←/→ adjust",
                        UiKind::Button { .. } => "enter presses",
                        UiKind::Tabs { .. } => "←/→ switch tab",
                        UiKind::Accordion => "enter opens / closes",
                        UiKind::Carousel { .. } => "←/→ scroll",
                    }
                    .to_string(),
                );
                if !self.forms.is_open(r, t) {
                    parts.push("tab next".to_string());
                }
            }
            None => parts.push("tab to start".to_string()),
        }
        parts.push(
            if focused.is_some_and(|t| self.forms.is_open(r, t)) {
                "esc closes"
            } else {
                "esc input"
            }
            .to_string(),
        );
        let text = parts.join(" · ");
        let w = usize::from(width);
        chat::barred(
            Line::from(Span::styled(
                truncate(&text, w.saturating_sub(2)),
                self.styles.muted,
            )),
            self.panel_marker(),
        )
    }

    /// The bar down a prompt in the editor's place: none in the input box, which has its own.
    pub(super) fn panel_marker(&self) -> Option<Style> {
        (!self.styles.fills()).then_some(self.styles.accent)
    }
}
