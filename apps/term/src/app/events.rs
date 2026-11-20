use super::*;

impl App {
    // ---- daemon events ----

    pub fn handle_event(&mut self, event: Event) -> Vec<Effect> {
        let mut fx = Vec::new();
        match event {
            Event::Focus(focus) => self.on_focus(focus, &mut fx),
            Event::Connectors(connectors) => self.connectors = connectors,
            // `/settings`' side-by-side diffs, for the chat and the file view; charts as images;
            // folded edits.
            Event::UserSettings(settings) => {
                self.split_diffs = settings.split_diffs;
                self.widgets.set_split_diffs(settings.split_diffs);
                self.chart_images = settings.chart_images == Some(true);
                self.hide_edits = settings.hide_edits == Some(true);
                self.prompt_addition = settings.prompt_addition;
                self.maybe_ask_work(settings.work.is_some());
            }
            Event::SessionChanged(info) => {
                if info.id == self.session {
                    self.sync_info(&info);
                }
                self.sessions.upsert(info);
            }
            Event::SessionRemoved(id) => {
                self.sessions.remove(&id);
                if let Some(at) = self.marks.iter().position(|m| *m == id) {
                    self.marks.remove(at);
                    let _ = self.prefs.set_marks(&self.marks);
                }
            }
            Event::Session(record) => {
                if record.session == self.session {
                    self.on_record(&record);
                }
            }
            Event::Subscribed { session, .. } => {
                if session.id == self.session {
                    self.waiting = session.waiting.clone().unwrap_or_default();
                    if session.state == SessionState::Running && !self.running {
                        self.running = true;
                        self.turn_started_ms = self.now_ms;
                    } else if session.state == SessionState::Idle {
                        self.running = false;
                    }
                }
            }
            Event::Connection(up) => {
                self.connected = up;
                // The daemon drops a question with its connection: an answer would find none.
                if !up && self.secret.take().is_some() {
                    self.notify("connection lost: the question was dropped", true);
                }
            }
            Event::Secret(request) => self.on_secret(request, &mut fx),
            Event::Theme(theme) => {
                self.daemon_palette = Palette::from_theme(&theme);
                self.apply_palette();
            }
            Event::ViewSnapshot(snapshot) => {
                if snapshot.session == self.session {
                    match (&snapshot.agent, self.transcript.as_mut()) {
                        (None, _) => {
                            self.chat.apply_snapshot(snapshot);
                            self.pages.sync(&self.session, self.chat.surfaces());
                            self.refresh_forms();
                            self.sync_prompt();
                        }
                        (Some(agent), Some(t)) if *agent == t.agent.harness => {
                            t.chat.apply_snapshot(snapshot);
                        }
                        _ => {}
                    }
                }
            }
            Event::ViewPatch(patch) => {
                if patch.session == self.session {
                    match (&patch.agent, self.transcript.as_mut()) {
                        (None, _) => {
                            self.chat.apply_patch(patch);
                            self.pages.sync(&self.session, self.chat.surfaces());
                            self.refresh_forms();
                            self.sync_prompt();
                        }
                        (Some(agent), Some(t)) if *agent == t.agent.harness => {
                            t.chat.apply_patch(patch);
                        }
                        _ => {}
                    }
                }
            }
            Event::FileChanged(changed) => self.widgets.on_file_changed(&changed.path),
        }
        self.dirty = true;
        fx
    }

    /// The daemon asks this app alone for a secret. It is shown in the editor's place when the
    /// question is for the shown session, else answered as cancelled (nothing could show it). A
    /// newer question replaces the open one, which is answered as cancelled.
    pub(super) fn on_secret(&mut self, request: SecretRequest, fx: &mut Vec<Effect>) {
        if request.session != self.session {
            fx.push(Effect::Secret {
                id: request.id,
                value: None,
            });
            return;
        }
        self.cancel_secret(fx);
        self.secret = Some(Secret::new(request.id, request.title, request.placeholder));
        self.chat.follow();
    }

    /// Closes the open secret question, if any, answering it as cancelled.
    pub(super) fn cancel_secret(&mut self, fx: &mut Vec<Effect>) {
        if let Some(secret) = self.secret.take() {
            fx.push(Effect::Secret {
                id: secret.id.clone(),
                value: None,
            });
        }
    }

    /// Keys while a secret is asked for: they are all the field's. Enter answers, Esc cancels.
    pub(super) fn secret_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let Some(secret) = self.secret.as_mut() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let value = match secret.key(key.code, ctrl, alt) {
            secret::Outcome::Typing => return,
            secret::Outcome::Submit(value) => Some(value),
            secret::Outcome::Cancel => None,
        };
        if let Some(Secret { id, .. }) = self.secret.take() {
            fx.push(Effect::Secret { id, value });
        }
    }

    pub(super) fn on_focus(&mut self, focus: AgentFocus, fx: &mut Vec<Effect>) {
        self.agent_app = focus.app.clone();
        let here = focus.app.as_deref() == Some(self.app_id.as_str());
        if let Some((want, _)) = &self.awaiting {
            if *want == focus.session {
                self.awaiting = None;
            } else if here {
                // Left over from before we moved the agent here.
                return;
            }
        }
        if here && focus.session != self.session {
            // Someone made another session current while the agent is here (/new, `neo
            // move`): follow it. A new conversation starts in the same directory.
            let cwd = self.cwd.clone();
            let from = self.session.clone();
            self.carry_mark(&from, &focus.session);
            self.cancel_secret(fx);
            self.begin_session(&focus.session, &cwd);
            self.awaiting = None;
            fx.push(Effect::Switch {
                from: Some(from),
                to: focus.session,
                cwd,
                move_agent: false,
            });
        }
    }

    /// A marked session followed into a fresh one (/new, /clear): the mark goes with it, in its
    /// slot. A session that has events already (`neo move` to an older one) takes no mark.
    fn carry_mark(&mut self, from: &str, to: &str) {
        let fresh = self.sessions.get(to).is_none_or(|s| s.last_seq == 0);
        if !fresh || self.marks.iter().any(|m| m == to) {
            return;
        }
        if let Some(at) = self.marks.iter().position(|m| m == from) {
            self.marks[at] = to.to_string();
            let _ = self.prefs.set_marks(&self.marks);
        }
    }

    pub(super) fn on_record(&mut self, record: &SessionEventRecord) {
        self.widgets.on_session_event(record);
        let known = match &record.event {
            SessionEvent::SubagentStarted(e) => self.agents.get(&e.id).is_some(),
            _ => true,
        };
        self.agents.on_record(record);
        if !known {
            self.toast_subagent(record);
        }
        if let SessionEvent::ToolCall(call) = &record.event {
            let r = format!("c-{}", call.tool_call_id);
            self.chat.tool_started(&r, record.ts);
            if call.kind == neo_protocol::ToolKind::Read {
                self.chat.tool_reads(&r);
            }
        }
        match &record.event {
            SessionEvent::UserMessage(message) => {
                // A turn starts with it, or the running one took it in.
                if !self.running {
                    self.turn_started_ms = record.ts;
                }
                self.running = true;
                self.context_cleared = false;
                let text = message.shown.as_ref().unwrap_or(&message.text);
                if let Some(at) = self.waiting.iter().position(|w| w.text == *text) {
                    self.waiting.remove(at);
                }
            }
            SessionEvent::TurnEnded(_) => {
                self.running = false;
                self.waiting_on.clear();
                // A guest that wasn't running when the names were asked for is by now.
                let unnamed = self
                    .settings
                    .iter()
                    .filter(|h| h.harness == self.harness)
                    .flat_map(|h| h.values.iter().map(move |(o, v)| (&h.harness, o, v)))
                    .any(|(h, o, v)| {
                        o == "model" && !self.model_names.contains_key(&(h.clone(), v.clone()))
                    });
                self.names_stale |= unnamed;
            }
            SessionEvent::ContextCleared(_) => self.context_cleared = true,
            SessionEvent::CommandsAvailable(offer) => {
                self.offered
                    .insert(offer.harness.clone(), offer.commands.clone());
            }
            SessionEvent::SettingChanged(change) => {
                let option = match change.option {
                    SettingName::Model => "model",
                    SettingName::Effort => "effort",
                    SettingName::Mode => "mode",
                    SettingName::Approvals => "approvals",
                    SettingName::Harness => "harness",
                    SettingName::TitleModel => "title_model",
                };
                self.on_setting(&change.harness, option, &change.value);
            }
            _ => {}
        }
    }

    /// Neo's own settings (approvals, harness) and each harness's (model, effort, mode).
    pub(super) fn on_setting(&mut self, harness: &str, option: &str, value: &str) {
        match (harness, option) {
            ("daemon", "approvals") => {
                if let Some(mode) = Approvals::parse(value) {
                    self.approvals = mode;
                }
            }
            ("daemon", "harness") => {
                self.harness = value.to_string();
                self.names_stale = true;
            }
            ("daemon", _) | (_, "approvals" | "harness") => {}
            _ => {
                if option == "model" {
                    self.names_stale = true;
                }
                record_setting(&mut self.settings, harness, option, value);
            }
        }
    }

    /// What the daemon says about the shown session: how many inputs wait, whether a turn still
    /// runs (a turn that just started is trusted for a few seconds), and where it works (/cd
    /// moves it: the files, git and paths follow).
    pub(super) fn sync_info(&mut self, info: &SessionInfo) {
        if info.cwd != self.cwd {
            self.cwd = info.cwd.clone();
            self.chat.set_dir(&info.cwd);
            self.widgets.move_to(PathBuf::from(&info.cwd));
            self.github.reset(PathBuf::from(&info.cwd));
        }
        self.waiting = info.waiting.clone().unwrap_or_default();
        self.waiting_on = info.waiting_on.clone().unwrap_or_default();
        if self.running
            && info.state == SessionState::Idle
            && self.now_ms - self.turn_started_ms > 3000
        {
            self.running = false;
        }
    }

    pub fn handle_msg(&mut self, msg: Msg) -> Vec<Effect> {
        self.dirty = true;
        let mut fx = Vec::new();
        match msg {
            Msg::Sessions(Ok(list)) => {
                if let Some(me) = list.iter().find(|s| s.id == self.session) {
                    self.sync_info(me);
                }
                self.sessions.set_all(list.clone());
                if self.overlay == Overlay::Switcher {
                    self.switcher.set_sessions(list);
                }
            }
            Msg::Sessions(Err(why)) => {
                if self.overlay == Overlay::Switcher {
                    self.switcher.error = Some(why);
                }
            }
            Msg::Created(Ok(info)) => {
                let info = *info;
                self.sessions.upsert(info.clone());
                let from = self.session.clone();
                self.cancel_secret(&mut fx);
                self.begin_session(&info.id, &info.cwd);
                self.awaiting = Some((info.id.clone(), self.now_ms + 3000));
                fx.push(Effect::Switch {
                    from: Some(from),
                    to: info.id,
                    cwd: info.cwd,
                    move_agent: true,
                });
            }
            Msg::Created(Err(why)) => self.notify(format!("new session failed: {why}"), true),
            Msg::Error(why) => self.notify(why, true),
            Msg::Notice(text) => self.notify(text, false),
            Msg::Pasted(Some(path)) => self.attach(vec![path]),
            Msg::Pasted(None) => self.notify("no image on the clipboard", false),
            Msg::VoiceFailed(why) => {
                self.voice = Voice::Off;
                self.notify(why, true);
            }
            Msg::Transcribed(result) => {
                self.voice = Voice::Off;
                match result {
                    Ok(text) if text.is_empty() => self.notify("nothing heard", false),
                    Ok(text) => self.dictate(&text),
                    Err(why) => self.notify(why, true),
                }
            }
            Msg::Unqueued { session, inputs } if session == self.session => self.redraft(inputs),
            Msg::Unqueued { .. } => {}
            Msg::Page { session, page } if session == self.session => match page {
                Ok(page) => self.chat.prepend(page),
                Err(why) => {
                    // Not asked again until the next snapshot: the item paged from is gone.
                    self.notify(format!("older messages didn't load: {why}"), true);
                }
            },
            Msg::Page { .. } => {}
            Msg::Names { session, harnesses } if session == self.session => {
                // Not being able to name a model is fine: the status line shows its value.
                for h in harnesses.unwrap_or_default() {
                    for option in h
                        .options
                        .iter()
                        .filter(|o| o.category == neo_protocol::ConfigCategory::Model)
                    {
                        for v in &option.values {
                            self.model_names
                                .insert((h.name.clone(), v.value.clone()), v.name.clone());
                        }
                    }
                }
            }
            Msg::Names { .. } => {}
        }
        fx
    }

    /// Shows another session: everything of the old one is dropped but its draft, which waits
    /// for it to be shown again; the new one's draft, if it had one, is back in the editor.
    pub(super) fn begin_session(&mut self, id: &str, cwd: &str) {
        // A setting's text half written is dropped: the draft under it is the session's.
        self.end_setting_input();
        let text = self.editor.text().to_string();
        let attachments = std::mem::take(&mut self.attachments);
        if text.trim().is_empty() {
            self.drafts.remove(&self.session);
        } else {
            self.drafts
                .insert(self.session.clone(), (text, attachments));
        }
        self.session = id.to_string();
        self.cwd = cwd.to_string();
        self.chat = Chat::for_app(&self.app_id).in_dir(cwd);
        self.running = false;
        self.context_cleared = false;
        self.turn_started_ms = self.now_ms;
        self.waiting.clear();
        self.waiting_on.clear();
        self.settings.clear();
        self.model_names.clear();
        self.names_stale = true;
        self.harness = DEFAULT_HARNESS.to_string();
        self.offered.clear();
        self.approvals = Approvals::Ask;
        self.editor.clear();
        if let Some((text, attachments)) = self.drafts.remove(id) {
            self.editor.set_text(&text);
            self.attachments = attachments;
        }
        self.prompt = PromptState::default();
        self.forms = Forms::default();
        self.note = None;
        self.secret = None;
        self.overlay = Overlay::None;
        self.focus = None;
        self.menu_sel = 0;
        self.menu_closed_for = None;
        self.widgets.reset(PathBuf::from(cwd));
        self.github.reset(PathBuf::from(cwd));
        self.agents = Agents::default();
        self.transcript = None;
        self.reader = None;
        self.dirty = true;
    }
}
