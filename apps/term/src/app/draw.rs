use super::*;

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.size = (area.width, area.height);
        let areas = self.areas(area);
        self.sidebar_shown = areas.side.is_some() && self.widgets.has_panels();
        self.terminal_area = areas
            .side
            .and_then(|side| self.widgets.side_terminal(side, areas.terminal));
        self.sessions_shown = areas.left.is_some();
        self.work_shown = areas.work.height > 0;
        if std::mem::take(&mut self.sessions_wanted) {
            if self.sessions_shown {
                self.focus = Some(Focus::Sessions);
                self.enter_focus();
            } else {
                self.notify("terminal too narrow for the sessions sidebar", false);
            }
        }
        match self.focus {
            Some(Focus::Panel(Panel::Terminal)) if !self.terminal_area.is_some() => {
                self.focus = None
            }
            Some(Focus::Panel(Panel::Todos | Panel::Plan | Panel::Files))
                if !self.sidebar_shown =>
            {
                self.focus = None;
            }
            // The last todo went: so did its panel.
            Some(Focus::Panel(Panel::Todos)) if !self.widgets.has_todos() => self.focus = None,
            Some(Focus::Sessions) if !self.sessions_shown => self.focus = None,
            Some(Focus::Agents) if !self.agents.visible(self.now_ms) => self.focus = None,
            Some(Focus::Jobs) if self.widgets.jobs(self.now_ms).is_empty() => {
                self.widgets.show_job(None);
                self.focus = None;
            }
            Some(Focus::Chat) if !self.chat.has_picks() => self.focus = None,
            _ => {}
        }
        self.sync_prompt();
        self.chat.set_highlight(self.focus == Some(Focus::Chat));
        let hint = self.backgroundable().map(|id| format!("c-{id}"));
        self.chat.set_background_hint(hint);

        let mut menus = false;
        if self.github.is_open() {
            self.github
                .draw(frame, areas.chat, self.now_ms, &self.styles);
        } else if self.widgets.overlay_open() {
            self.widgets.draw_overlay(frame, areas.chat);
        } else if self.transcript.is_some() {
            let opts = ChatOpts {
                expanded: self.expanded,
                show_thoughts: self.show_thoughts,
                palette: self.palette,
                active: None,
                in_panel: false,
                marker: self.styles.accent,
                ui: Default::default(),
                icons: self.icons,
                // A subagent's work has no turns to sum up.
                running: true,
                split_diffs: self.split_diffs,
                hide_edits: self.hide_edits,
                dismissed: Vec::new(),
                linked: None,
            };
            if let Some(t) = self.transcript.as_mut() {
                t.draw(frame, areas.chat, &opts, &self.styles);
            }
        } else if let Some(r) = self.reader.as_mut() {
            r.draw(frame, areas.chat, &self.palette);
        } else {
            match self.overlay {
                Overlay::None => self.draw_chat(frame, areas.chat),
                Overlay::Help => self.draw_help(frame, areas.chat),
                Overlay::Switcher => self.draw_switcher(frame, areas.chat),
                Overlay::Marks => {
                    self.draw_chat(frame, areas.chat);
                    self.draw_marks(frame, areas.chat);
                }
                // Over the whole screen, at the end.
                Overlay::Setup => self.draw_chat(frame, areas.chat),
            }
            menus = true;
        }
        self.review_shown =
            areas.review.height > 0 && self.widgets.draw_review(frame, areas.review);
        if self.review_shown && self.styles.fills() {
            // On the input box's background, as part of the card; the diff's lines keep theirs.
            let buf = frame.buffer_mut();
            for y in areas.review.top()..areas.review.bottom() {
                for x in areas.review.left()..areas.review.right() {
                    if buf[(x, y)].bg == ratatui::style::Color::Reset {
                        buf[(x, y)].set_bg(self.styles.raised);
                    }
                }
            }
        }
        self.toasts
            .draw(frame.buffer_mut(), areas.chat, self.now_ms, &self.styles);
        self.details_shown = false;
        if let Some(left) = areas.left {
            self.draw_sessions(frame, left);
        }
        if let Some(side) = areas.side {
            let panel = match self.focus {
                Some(Focus::Panel(panel)) => Some(panel),
                _ => None,
            };
            // Whichever was scrolled last leads: the chat scrolled up, the terminal shows the
            // commands as they were at that point; the terminal scrolled, the chat goes to the
            // command on its bottom row.
            if self.widgets.take_terminal_scrolled() {
                self.terminal_leads = Some(self.chat.top());
            } else if self
                .terminal_leads
                .is_some_and(|top| top != self.chat.top())
            {
                self.terminal_leads = None;
            }
            if self.terminal_leads.is_none() {
                self.widgets.follow_chat(self.chat.items_through_view());
            }
            self.widgets
                .draw_sidebar(frame, side, panel, areas.terminal);
            if self.terminal_leads.is_some()
                && let Some(run) = self.widgets.terminal_bottom_run()
            {
                let before = self.chat.top();
                self.chat.scroll_to(&format!("c-{run}"));
                self.terminal_leads = Some(self.chat.top());
                // The chat was drawn already: once more, where it went.
                self.dirty |= self.chat.top() != before;
            }
        }
        let fills = self.styles.fills();
        if fills {
            crate::style::fill(frame.buffer_mut(), areas.input, self.styles.raised);
        } else if areas.rule.height > 0 {
            let rule = "─".repeat(usize::from(areas.rule.width));
            frame.render_widget(
                Paragraph::new(Span::styled(rule, self.styles.border)),
                areas.rule,
            );
        }
        if areas.hint.height > 0 {
            let hint = if let Some(setting) = self.setting_hint() {
                setting
            } else if self.active_form().is_some() {
                "form open · ctrl-p to fill it in · ctrl-s stashes your draft".into()
            } else {
                "permission needed · ctrl-p to answer · ctrl-s stashes your draft".into()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    truncate(&hint, usize::from(areas.hint.width)),
                    self.styles.accent,
                )),
                areas.hint,
            );
        }
        if areas.waiting.height > 0 {
            self.draw_waiting(frame, areas.waiting);
        }
        if areas.attached.height > 0 {
            self.draw_attached(frame, areas.attached);
        }
        if areas.panel.is_empty() {
            self.draw_editor(frame, areas.editor);
        } else if fills {
            let at = Rect {
                x: areas.editor.x + 2,
                width: areas.editor.width.saturating_sub(3),
                ..areas.editor
            };
            frame.render_widget(Paragraph::new(areas.panel), at);
        } else {
            frame.render_widget(Paragraph::new(areas.panel), areas.editor);
        }
        self.draw_status(frame, areas.status);
        self.draw_work(frame, areas.work);
        if fills {
            // The bar down the input box's left edge.
            let bar = self.styles.accent.bg(self.styles.raised);
            for y in areas.input.top()..areas.input.bottom() {
                frame.buffer_mut()[(areas.input.x, y)]
                    .set_symbol("┃")
                    .set_style(bar);
            }
        }
        // The command and mention menus sit right on the input box, over the working line.
        if menus {
            let above = Rect {
                height: areas.input.y - areas.chat.y,
                ..areas.chat
            };
            self.draw_menu(frame, above);
            self.draw_mentions(frame, above);
        }
        self.draw_agents(frame, areas.agents);
        self.draw_jobs(frame, areas.jobs);
        if self.overlay == Overlay::Setup {
            self.draw_setup(frame);
        }
        if !self.no_color && self.theme.paints() {
            crate::theme::paint(frame.buffer_mut(), &self.palette);
        }
    }

    /// The sessions on the left: the list, then a border and a gap before the chat.
    pub(super) fn draw_sessions(&mut self, frame: &mut Frame, area: Rect) {
        let inner = area.width.saturating_sub(2);
        crate::style::fill(
            frame.buffer_mut(),
            Rect {
                width: inner + 1,
                ..area
            },
            self.styles.panel,
        );
        // The shown session's details at the foot, while the list keeps a dozen rows.
        let dir = shorten_home(&self.cwd, self.home.as_deref());
        // The keys left the sidebar: next time they start on the sessions.
        if self.focus != Some(Focus::Sessions) {
            self.apps_sel = None;
        }
        let selected = self.apps_sel;
        let (details, rows) = crate::details::lines(
            &crate::details::Details {
                dir: &dir,
                usage: self.widgets.usage(),
                connectors: &self.connectors,
                apps_open: self.apps_open,
                selected,
            },
            usize::from(inner),
            &self.styles,
        );
        self.app_rows = rows;
        let details_h = u16::try_from(details.len() + 1).unwrap_or(u16::MAX);
        let details_h = if area.height >= details_h + 12 {
            details_h
        } else {
            0
        };
        self.details_shown = details_h > 0;
        // A row of the fill above the list.
        let top = u16::from(self.styles.fills() && area.height - details_h > 4);
        let list = Rect {
            y: area.y + top,
            width: inner,
            height: area.height - details_h - top,
            ..area
        };
        self.apps_toggle_at = None;
        if details_h > 0 {
            let at = Rect {
                y: area.y + area.height - details_h + 1,
                height: details_h - 1,
                ..list
            };
            frame.render_widget(Paragraph::new(details), at);
            // Where a click shows or hides the rest of the apps.
            self.apps_toggle_at = rows.toggle.map(|row| Rect {
                y: at.y + (rows.first + row) as u16,
                height: 1,
                ..at
            });
        }
        let look = Look {
            current: &self.session,
            // On the apps, the keys aren't on a session.
            focused: self.focus == Some(Focus::Sessions) && self.apps_sel.is_none(),
            spinner: self.spinner(),
            now_ms: self.now_ms,
            marks: &self.marks,
        };
        let lines = self.sessions.render(
            usize::from(list.width),
            usize::from(list.height),
            &look,
            &self.styles,
        );
        frame.render_widget(Paragraph::new(lines), list);
        crate::style::pane_edge(frame.buffer_mut(), area.x + inner, area, &self.palette);
    }

    pub(super) fn chat_opts(&self) -> ChatOpts {
        ChatOpts {
            expanded: self.expanded,
            show_thoughts: self.show_thoughts,
            palette: self.palette,
            active: self
                .active_ref()
                .map(|r| (r.to_string(), self.prompt.button)),
            in_panel: self.prompt.engaged && self.active_form().is_none(),
            marker: self.styles.accent,
            ui: self.forms.all().clone(),
            icons: self.icons,
            running: self.running,
            split_diffs: self.split_diffs,
            hide_edits: self.hide_edits,
            dismissed: self.prompt.dismissed.clone(),
            // Only while the panel is on screen.
            linked: self
                .terminal_area
                .and_then(|_| self.widgets.terminal_linked_run())
                .map(|run| format!("c-{run}")),
        }
    }

    /// The cells the mouse is selecting, reversed.
    pub(super) fn draw_mouse_selection(&self, frame: &mut Frame, content: Rect) {
        let Some(((l0, c0), (l1, c1))) = self.chat.selection() else {
            return;
        };
        let top = self.chat.top();
        let buf = frame.buffer_mut();
        for y in 0..content.height {
            let line = top + usize::from(y);
            if line < l0 || line > l1 {
                continue;
            }
            let from = if line == l0 { c0 } else { 0 };
            let to = if line == l1 { c1 } else { usize::MAX };
            for x in 0..content.width {
                if (from..=to).contains(&usize::from(x)) {
                    let cell = &mut buf[(content.x + x, content.y + y)];
                    cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                }
            }
        }
    }

    /// A session nothing was said in yet: where it works, the latest sessions there when the
    /// sidebar doesn't list them, and the keys to start with.
    pub(super) fn empty_chat(&mut self, w: usize) -> Vec<Line<'static>> {
        let muted = self.styles.muted;
        let project = self
            .cwd
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        let mut head = vec![Span::styled(project, Modifier::BOLD)];
        if let Some(branch) = self.widgets.branch() {
            head.push(Span::styled(format!(" · {branch}"), muted));
        }
        let mut lines = vec![fit_line(Line::from(head), w), Line::default()];
        if !self.sessions_shown {
            let recent = self
                .sessions
                .recent_at(&self.cwd, &self.session, EMPTY_RECENT);
            if !recent.is_empty() {
                lines.push(Line::from(Span::styled("recent here", muted)));
                for info in &recent {
                    let age = relative_time(info.last_activity, self.now_ms);
                    let room = w.saturating_sub(width(&age) + 4);
                    let title = truncate(&session_title(info), room);
                    let pad = w.saturating_sub(width(&title) + width(&age) + 2).max(1);
                    lines.push(fit_line(
                        Line::from(vec![
                            Span::raw(format!("  {title}")),
                            Span::raw(" ".repeat(pad)),
                            Span::styled(age, muted),
                        ]),
                        w,
                    ));
                }
                lines.push(Line::default());
            }
        }
        let key = |k: &'static str, what: &'static str| {
            vec![
                Span::styled(k, self.styles.accent),
                Span::styled(format!(" {what}"), muted),
            ]
        };
        let sep = || Span::styled("  ·  ", muted);
        let mut keys = key("enter", "send");
        for (k, what) in [("/", "commands"), ("ctrl-g", "sessions"), ("/help", "keys")] {
            keys.push(sep());
            keys.extend(key(k, what));
        }
        lines.push(fit_line(Line::from(keys), w));
        lines
    }

    pub(super) fn draw_chat(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let content = Rect {
            width: if area.width >= 20 {
                area.width - 1
            } else {
                area.width
            },
            ..area
        };
        // With fills, in from the edge as far as the input box's text.
        let content = if self.styles.fills() && content.width >= 20 && content.height > 4 {
            Rect {
                x: content.x + 2,
                y: content.y + 1,
                width: content.width - 2,
                height: content.height - 1,
            }
        } else {
            content
        };
        let opts = self.chat_opts();
        if let Some(images) = self.images.as_mut() {
            images.request(self.chat.image_sources());
            let mut view = images.chat_view();
            view.charts = self.chart_images && images.draws_charts();
            self.chat.set_images(Some(view));
        }
        self.chat.layout(content.width, content.height, &opts);
        self.chat_area = (!self.chat.items().is_empty()).then_some(content);
        if self.chat.items().is_empty() {
            let lines = if self.chat.loaded() {
                self.empty_chat(usize::from(content.width))
            } else {
                vec![Line::from(Span::styled(
                    "connecting to the session…",
                    self.styles.muted,
                ))]
            };
            frame.render_widget(Paragraph::new(lines), content);
            return;
        }
        frame.render_widget(
            Paragraph::new(self.chat.visible_at(Some(self.now_ms))),
            content,
        );
        if let Some(images) = self.images.as_mut() {
            for at in self.chat.placements() {
                images.draw(frame.buffer_mut(), content, &at, self.styles.muted);
            }
        }
        self.draw_mouse_selection(frame, content);
        let total = self.chat.total_lines();
        let height = usize::from(content.height);
        if total > height && area.width >= 20 {
            let bar = Rect {
                x: area.x + area.width - 1,
                width: 1,
                ..area
            };
            let mut state = ScrollbarState::new(total - height).position(self.chat.top());
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(None)
                    .thumb_symbol("│")
                    .thumb_style(self.styles.border),
                bar,
                &mut state,
            );
        }
        if !self.chat.following() {
            let text = if self.chat.has_unseen() {
                "↓ new output (End)"
            } else {
                "↓ End to follow"
            };
            let w = width(text) as u16;
            if content.width > w + 2 {
                let at = Rect {
                    x: content.x + content.width - w - 1,
                    y: content.y + content.height - 1,
                    width: w,
                    height: 1,
                };
                frame.render_widget(Clear, at);
                frame.render_widget(Paragraph::new(Span::styled(text, self.styles.muted)), at);
            }
        }
    }
}
