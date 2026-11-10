use super::*;

impl App {
    /// A card for each background command that has just ended, once.
    pub(super) fn toast_jobs(&mut self, rows: &[crate::widgets::JobRow], now_ms: i64) {
        let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        for job in rows {
            let Some(ended) = job.ended_ms else {
                continue;
            };
            if now_ms - ended > TOAST_FRESH_MS || !self.toasted_jobs.insert(job.id.clone()) {
                continue;
            }
            let (title, tone) = match job.exit {
                Some(0) | None => ("Command finished", toast::Tone::Success),
                Some(_) => ("Command failed", toast::Tone::Danger),
            };
            let mut detail = one_line(&job.command);
            if let Some(code) = job.exit.filter(|c| *c != 0) {
                detail = format!("exit {code} · {detail}");
            }
            self.toasts.push(title, detail, tone, now_ms);
            self.dirty = true;
        }
    }

    /// A card for a subagent that has just started (not one replayed from the log).
    pub(super) fn toast_subagent(&mut self, record: &SessionEventRecord) {
        let SessionEvent::SubagentStarted(e) = &record.event else {
            return;
        };
        if self.now_ms - record.ts > TOAST_FRESH_MS {
            return;
        }
        let who = e.name.as_deref().unwrap_or(&e.agent);
        let title = if e.background.unwrap_or(false) {
            format!("{who} started in the background")
        } else {
            format!("{who} started")
        };
        let task = e.task.split_whitespace().collect::<Vec<_>>().join(" ");
        self.toasts
            .push(title, task, toast::Tone::Accent, self.now_ms);
        self.dirty = true;
    }

    /// The spinner's frame now.
    pub(super) fn spinner(&self) -> char {
        SPINNER[usize::try_from(self.now_ms / SPIN_MS).unwrap_or(0) % SPINNER.len()]
    }

    /// Seconds the running turn has taken; None when idle.
    pub(super) fn working_secs(&self) -> Option<u64> {
        self.running
            .then(|| u64::try_from((self.now_ms - self.turn_started_ms) / 1000).unwrap_or(0))
    }

    /// What the work row says the agent is up to: a phrase from [`WORK_WORDS`], another one every
    /// [`WORK_WORD_MS`], each turn starting somewhere else in the list.
    pub(super) fn work_word(&self) -> &'static str {
        let turn = usize::try_from(self.turn_started_ms / 1000).unwrap_or(0);
        let step =
            usize::try_from((self.now_ms - self.turn_started_ms) / WORK_WORD_MS).unwrap_or(0);
        // Stepping by a number prime to the length visits every phrase before one comes back.
        WORK_WORDS[(turn.wrapping_mul(7) + step * 5) % WORK_WORDS.len()]
    }

    /// The row over the input box: `◐ noodling… · 1m08s · esc to interrupt` with a light sweeping
    /// across it, or while idle a band saying the cache has expired.
    pub(super) fn draw_work(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        // As far in as the chat's text.
        let inset = if self.styles.fills() { 2 } else { 0 };
        let at = Rect {
            x: area.x + inset,
            width: area.width.saturating_sub(inset),
            ..area
        };
        // Speech to text, over whatever else the row would say.
        match self.voice {
            Voice::Recording(since) => {
                let secs = u64::try_from((self.now_ms - since) / 1000).unwrap_or(0);
                let line = Line::from(vec![
                    Span::styled(
                        "● recording",
                        self.styles.danger.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            " · {} · let go of space to stop",
                            crate::status::duration(secs)
                        ),
                        self.styles.muted,
                    ),
                ]);
                frame.render_widget(Paragraph::new(fit_line(line, usize::from(at.width))), at);
                return;
            }
            Voice::Transcribing => {
                let line = Line::from(Span::styled(
                    format!("{} writing out what you said…", self.spinner()),
                    self.styles.accent,
                ));
                frame.render_widget(Paragraph::new(fit_line(line, usize::from(at.width))), at);
                return;
            }
            Voice::Off => {}
        }
        let Some(secs) = self.working_secs() else {
            if self.cache_cold {
                self.draw_cache_cold(frame, at);
            }
            return;
        };
        if !self.waiting_on.is_empty() {
            self.draw_waiting_on(frame, at, secs);
            return;
        }
        let bold = self.styles.accent.add_modifier(Modifier::BOLD);
        let mut spans = vec![
            Span::styled(format!("{} {}", self.spinner(), self.work_word()), bold),
            Span::styled(
                format!(" · {}", crate::status::duration(secs)),
                self.styles.muted,
            ),
        ];
        if self.active_ref().is_none() {
            spans.push(Span::styled(" · esc to interrupt", self.styles.muted));
        }
        let line = chat::sweep(&Line::from(spans), self.now_ms, &self.palette);
        frame.render_widget(Paragraph::new(fit_line(line, usize::from(at.width))), at);
    }

    /// The agent is done, but its turn stays open for subagents working in the background: a
    /// band the row's whole width, tinted with the info color, saying what still works.
    pub(super) fn draw_waiting_on(&self, frame: &mut Frame, at: Rect, secs: u64) {
        let band = chat::mix(self.palette.bg, self.palette.info, 0.18);
        let tone = |s: Style| match band {
            Some(bg) => s.bg(bg),
            None => s,
        };
        let who = match self.waiting_on.as_slice() {
            [one] => format!("a subagent is still working: {one}"),
            many => format!("{} subagents are still working", many.len()),
        };
        let line = Line::from(vec![
            Span::styled(
                format!(" {} {who} ", self.spinner()),
                tone(self.styles.info.add_modifier(Modifier::BOLD)),
            ),
            Span::styled(
                format!("· {} · the reply above is done ", crate::status::duration(secs)),
                tone(self.styles.muted),
            ),
        ]);
        let mut line = fit_line(line, usize::from(at.width));
        if let Some(bg) = band {
            line = line.style(Style::new().bg(bg));
        }
        frame.render_widget(Paragraph::new(line), at);
    }

    /// `⚠ cache expired · …` on a band tinted with the danger color, the row's whole width.
    pub(super) fn draw_cache_cold(&self, frame: &mut Frame, at: Rect) {
        let band = chat::mix(self.palette.bg, self.palette.danger, 0.18);
        let tone = |s: Style| match band {
            Some(bg) => s.bg(bg),
            None => s,
        };
        let line = Line::from(vec![
            Span::styled(
                " ⚠ cache expired ",
                tone(self.styles.danger.add_modifier(Modifier::BOLD)),
            ),
            Span::styled(
                "· the next message re-reads the whole conversation, slower and at full price ",
                tone(Style::new()),
            ),
        ]);
        let mut line = fit_line(line, usize::from(at.width));
        if let Some(bg) = band {
            line = line.style(Style::new().bg(bg));
        }
        frame.render_widget(Paragraph::new(line), at);
    }

    pub(super) fn status_lines(&self, width: usize) -> [Line<'static>; 2] {
        // The project's name: the terminal's own title or prompt has the whole path.
        let cwd = shorten_home(&self.cwd, self.home.as_deref());
        let cwd = cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(&cwd);
        let cwd = if cwd.is_empty() { "/" } else { cwd };
        let branch = self.widgets.branch();
        let agent_in = self
            .agent_app
            .as_deref()
            .filter(|a| *a != self.app_id.as_str());
        let notice = self.notice.as_ref().map(|n| (n.text.as_str(), n.error));
        Status {
            cwd,
            branch: branch.as_deref(),
            settings: &self.settings,
            names: &self.model_names,
            harness: &self.harness,
            agent_in,
            // The sessions sidebar's foot has them when it shows.
            usage: self.widgets.usage_line().filter(|_| !self.details_shown),
            working_secs: self.working_secs().filter(|_| !self.work_shown),
            spinner: self.spinner(),
            queued: self.waiting.len(),
            stashed: self.stash.is_some(),
            cache_cold: self.cache_cold && !self.work_shown,
            connected: self.connected,
            approvals: self.approvals,
            notice,
            ui_hint: self.active_ref().is_none() && self.chat.latest_has_targets(),
            styles: &self.styles,
        }
        .lines(width)
    }

    /// The status lines; with fills both start past the input box's bar, the first inside the
    /// box and the second under it.
    pub(super) fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let area = if self.styles.fills() {
            Rect {
                x: area.x + 2,
                width: area.width.saturating_sub(3),
                ..area
            }
        } else {
            area
        };
        let lines = self.status_lines(usize::from(area.width));
        frame.render_widget(Paragraph::new(lines.to_vec()), area);
    }

    pub(super) fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let key = |k: &str, what: &str| {
            Line::from(vec![
                Span::styled(format!("  {k:<26}"), self.styles.accent),
                Span::raw(what.to_string()),
            ])
        };
        let head = |t: &str| Line::from(Span::styled(t.to_string(), Modifier::BOLD));
        let mut lines = vec![
            head("keys (any key closes this)"),
            key("enter", "send"),
            key(
                "alt-enter, ctrl-enter",
                "send into the running turn, not after it",
            ),
            key("ctrl-j, shift-enter", "new line"),
            key("up / down", "move in the text, then through history"),
            key("ctrl-a / end", "start / end of line"),
            key("alt-b / alt-f", "word left / right"),
            key(
                "ctrl-w / ctrl-u / ctrl-k",
                "delete word / to line start / to line end",
            ),
            key(
                "pgup / pgdn",
                "scroll the chat (also the mouse wheel; dragging copies text)",
            ),
            key("shift-up / shift-down", "scroll the chat a line"),
            key(
                "ctrl-u / ctrl-d",
                "half page up / down (empty input; ctrl-d quits at the end)",
            ),
            key(
                "home / end",
                "top / bottom of the chat (empty input; ctrl-home/end always)",
            ),
            key("ctrl-o", "full tool results"),
            key("ctrl-t", "show thoughts"),
            key("esc", "close menu, dismiss prompt, cancel the running turn"),
            key("ctrl-b", "a command running a while: on in the background"),
            key("ctrl-c", "clear input; twice to quit"),
            key(
                "shift-tab",
                "approvals: ask / review / edits / auto; in a panel, the previous one",
            ),
            key(
                "tab",
                "complete command, note on a prompt, next panel (the chat first)",
            ),
            key(
                "chat (tab)",
                "j/k pick · y copy as markdown · enter: in vim keys (v/V, y) · q",
            ),
            key("ctrl-s", "put the draft aside / bring it back"),
            key(
                "ctrl-g",
                "sessions sidebar: focus (up/down, enter, n new chat, esc)",
            ),
            key(
                "ctrl-e, alt-m, ctrl-1…9",
                "marked chats (a adds, 1-9 shows), mark the chat, show chat n",
            ),
            key("ctrl-r", "GitHub issues and pull requests (/github)"),
            key(
                "ctrl-x, down",
                "subagents under the prompt: enter opens, x stops (down: empty input)",
            ),
            key(
                "diff view",
                "n/p hunk · h/l/0 sideways · s split/unified · f whole file · q close",
            ),
            key(
                "1-9, arrows, enter",
                "answer a prompt or fill in a form: tab, type, space; esc: input",
            ),
            key(
                "ctrl-p / ctrl-n",
                "use the buttons and forms above, older first",
            ),
            Line::default(),
            head("commands"),
        ];
        for c in self.commands() {
            let label = if c.args.is_empty() {
                format!("/{}", c.name)
            } else {
                format!("/{} {}", c.name, c.args)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<36}"), self.styles.accent),
                Span::styled(c.desc.clone(), self.styles.muted),
            ]));
        }
        frame.render_widget(Clear, area);
        let lines = lines
            .into_iter()
            .map(|l| fit_line(l, usize::from(area.width)))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// The marked sessions, by slot: a dialog in the middle of the chat. A row per mark, its
    /// status as the sidebar has it (`!` needs you, the spinner working, `●` done), the title,
    /// and at the right where it works and when; the selected row on an accent band, the shown
    /// chat with the sidebar's bar. The keys under them.
    pub(super) fn draw_marks(&self, frame: &mut Frame, area: Rect) {
        use crate::sessions::Status as Is;
        use ratatui::widgets::{Block, BorderType, Borders, Padding};
        // A blank row over the list and one before the keys.
        let rows = u16::try_from(self.marks.len().max(1)).unwrap_or(u16::MAX) + 3;
        let dialog = Rect {
            width: area.width.saturating_sub(4).min(76),
            height: (rows + 2).min(area.height),
            ..area
        };
        let dialog = Rect {
            x: area.x + (area.width - dialog.width) / 2,
            y: area.y + (area.height - dialog.height) / 3,
            ..dialog
        };
        // Inside the border and a column of padding each side.
        let w = usize::from(dialog.width.saturating_sub(4));
        let mut title = vec![Span::styled(
            " Marks ",
            self.styles.accent.add_modifier(Modifier::BOLD),
        )];
        if !self.marks.is_empty() {
            title.push(Span::styled(
                format!("{} ", self.marks.len()),
                self.styles.muted,
            ));
        }
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.styles.border)
            .title(Line::from(title))
            .style(Style::new().bg(self.styles.raised))
            .padding(Padding::new(1, 1, 1, 0));
        let band = chat::mix(self.styles.raised, self.palette.accent, 0.16);
        let mut lines = Vec::new();
        if self.marks.is_empty() {
            lines.push(Line::from(Span::styled(
                "nothing marked yet: a marks the shown chat",
                self.styles.muted,
            )));
        }
        let sel = self.marks_sel.min(self.marks.len().saturating_sub(1));
        for (i, id) in self.marks.iter().enumerate() {
            let selected = i == sel;
            let bar = if *id == self.session { "▎" } else { " " };
            let slot = if selected {
                self.styles.accent.add_modifier(Modifier::BOLD)
            } else {
                self.styles.muted
            };
            let mut left = vec![
                Span::styled(bar, self.styles.accent),
                Span::styled(format!("{} ", i + 1), slot),
            ];
            let mut right = Vec::new();
            match self.sessions.get(id) {
                Some(info) => {
                    let status = self.sessions.status(info);
                    let (word, tone) = match status {
                        Is::Asking => ("needs you", self.styles.warning),
                        Is::Running => ("working", self.styles.info),
                        Is::Finished => ("your turn", self.styles.success),
                        Is::Idle => ("", Style::new()),
                    };
                    let age = relative_time(info.last_activity, self.now_ms);
                    if !word.is_empty() {
                        right.push(Span::styled(word, tone));
                        right.push(Span::styled(" · ", self.styles.muted));
                    }
                    right.push(Span::styled(age, self.styles.muted));
                    left.push(status.icon(self.spinner(), &self.styles));
                    left.push(Span::raw(" "));
                    let name = match status {
                        _ if selected => Style::new().add_modifier(Modifier::BOLD),
                        Is::Asking => self.styles.warning.add_modifier(Modifier::BOLD),
                        _ => Style::new(),
                    };
                    let right_w: usize = right.iter().map(|s| width(&s.content)).sum();
                    let room = w.saturating_sub(5 + right_w + 2);
                    let title = truncate(&session_title(info), room.min(44));
                    let title_w = width(&title);
                    left.push(Span::styled(title, name));
                    // The project it works in, by its directory's name, in what is left.
                    let project = info.cwd.trim_end_matches('/').rsplit('/').next();
                    let project = project.filter(|p| !p.is_empty()).unwrap_or("/");
                    let gap = room.saturating_sub(title_w);
                    if gap >= 6 {
                        let project = truncate(project, gap - 2);
                        left.push(Span::styled(format!("  {project}"), self.styles.muted));
                    }
                }
                None => left.push(Span::styled(
                    format!("  {id} (not listed)"),
                    self.styles.muted,
                )),
            }
            let used: usize = left.iter().chain(&right).map(|s| width(&s.content)).sum();
            left.push(Span::raw(" ".repeat(w.saturating_sub(used))));
            left.extend(right);
            let mut line = fit_line(Line::from(left), w);
            if selected && let Some(bg) = band {
                line = line.style(Style::new().bg(bg));
            } else if selected {
                line = line.style(Style::new().add_modifier(Modifier::REVERSED));
            }
            lines.push(line);
        }
        lines.push(Line::default());
        let key = |k: &'static str| Span::styled(k, self.styles.accent);
        let what = |d: &'static str| Span::styled(d, self.styles.muted);
        lines.push(fit_line(
            Line::from(vec![
                key("enter"),
                what(" show  "),
                key("a"),
                what(" add  "),
                key("J/K"),
                what(" move  "),
                key("d"),
                what(" unmark  "),
                key("esc"),
                what(" close"),
            ]),
            w,
        ));
        frame.render_widget(Clear, dialog);
        frame.render_widget(Paragraph::new(lines).block(block), dialog);
    }

    pub(super) fn draw_switcher(&self, frame: &mut Frame, area: Rect) {
        let w = usize::from(area.width);
        let s = &self.switcher;
        let mut lines = vec![
            Line::from(vec![
                Span::styled("sessions", Modifier::BOLD),
                Span::styled(
                    "  type to filter · enter switch · esc close",
                    self.styles.muted,
                ),
            ]),
            Line::from(vec![
                Span::styled("> ", self.styles.accent),
                Span::raw(s.filter.clone()),
            ]),
        ];
        let entries = s.entries();
        let rows = usize::from(area.height).saturating_sub(lines.len()).max(1);
        let start = (s.selected + 1).saturating_sub(rows);
        for (i, entry) in entries.iter().enumerate().skip(start).take(rows) {
            let selected = i == s.selected;
            let (marker, style) = if selected {
                ("› ", self.styles.accent.add_modifier(Modifier::BOLD))
            } else {
                ("  ", Style::new())
            };
            let mut spans = vec![Span::styled(marker, self.styles.accent)];
            match entry {
                Entry::New => spans.push(Span::styled(
                    format!(
                        "+ new session in {}",
                        shorten_home(&self.cwd, self.home.as_deref())
                    ),
                    style,
                )),
                Entry::Session(idx) => {
                    let info = &s.sessions.as_ref().expect("entries come from the list")[*idx];
                    let (name, rest) = menu::session_label(info, self.home.as_deref(), self.now_ms);
                    let here = if info.id == s.current { "● " } else { "  " };
                    spans.push(Span::styled(here, self.styles.success));
                    spans.push(Span::styled(truncate(&name, 40.min(w / 2)), style));
                    spans.push(Span::styled(format!("  {rest}"), self.styles.muted));
                }
            }
            lines.push(fit_line(Line::from(spans), w));
        }
        if s.sessions.is_none() {
            let (text, style) = match &s.error {
                Some(e) => (format!("could not list sessions: {e}"), self.styles.danger),
                None => ("loading…".to_string(), self.styles.muted),
            };
            lines.push(Line::from(Span::styled(text, style)));
        } else if entries.len() == 1 && !s.filter.is_empty() {
            lines.push(Line::from(Span::styled(
                "no session matches",
                self.styles.muted,
            )));
        }
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines), area);
    }
}
