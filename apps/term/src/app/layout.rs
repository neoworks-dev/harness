use super::*;

impl App {
    // ---- drawing ----

    pub(super) fn areas(&mut self, area: Rect) -> Areas {
        // The side column: the todos, plan and files as wide as the files need, up to a third of
        // the terminal; with the agent's commands under them, two fifths, as the commands want
        // room.
        let terminal = self.terminal_on
            && self.widgets.has_commands()
            && area.width >= CHAT_MIN_WIDTH + 1 + TERMINAL_MIN_WIDTH;
        let panels_base = self
            .widgets
            .sidebar_width(SIDE_MIN_WIDTH, (area.width / 3).clamp(32, 40));
        let (side_base, side_min) = if terminal {
            let base = (area.width * 2 / 5).clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
            (base.max(panels_base), TERMINAL_MIN_WIDTH)
        } else {
            (panels_base, SIDE_MIN_WIDTH)
        };
        let side_w = resized(
            side_base,
            self.resized.side,
            side_min,
            area.width.saturating_sub(1 + CHAT_MIN_WIDTH),
        );
        let side = (area.width >= side_w + 1 + 40
            && (terminal || self.widgets.wants_sidebar(area.width)))
        .then_some(side_w);
        if side.is_some() {
            self.resized.side = side_w as i16 - side_base as i16;
        }
        let right_w = side.map_or(0, |w| w + 1);
        // The sessions sidebar yields to the plan / files column, and to the chat's width.
        let left_base = sidebar_width(area.width);
        let left_w = resized(
            left_base,
            self.resized.sessions,
            SIDEBAR_WIDTH,
            area.width.saturating_sub(right_w + CHAT_MIN_WIDTH),
        );
        if left_w >= SIDEBAR_WIDTH {
            self.resized.sessions = left_w as i16 - left_base as i16;
        }
        let left = (self.sessions_on && !self.sessions.is_empty() && left_w >= SIDEBAR_WIDTH)
            .then_some(Rect {
                width: left_w,
                ..area
            });
        let left_w = left.map_or(0, |r| r.width);
        let main = Rect {
            x: area.x + left_w,
            width: area.width - left_w - right_w,
            ..area
        };
        let terminal = terminal && side.is_some();
        let side = side.map(|w| Rect {
            x: area.x + area.width - w,
            y: area.y,
            width: w,
            height: area.height,
        });
        // Past the prompt (or the input box's bar), and its right margin with fills.
        let margin = u16::from(self.styles.fills());
        self.editor
            .set_width(usize::from(main.width.saturating_sub(2 + margin)).max(1));
        // The subagents list sits under the status lines.
        let agents_h = self.agents_rows(main.height);
        let jobs_h = self.jobs_rows(main.height - agents_h);
        let h = main.height - agents_h - jobs_h;
        let status_h = h.saturating_sub(3).min(2);
        let rule_h = h.saturating_sub(status_h).min(1);
        let room = h - status_h - rule_h;
        // An engaged prompt takes the editor's place; a waiting one adds a hint row above it.
        let box_w = main.width.saturating_sub(3 * margin);
        let panel = if let Some(secret) = &self.secret {
            secret.lines(
                usize::from(box_w),
                self.styles.accent,
                self.panel_marker(),
                self.styles.muted,
            )
        } else if self.prompt.engaged {
            self.panel_lines(box_w, usize::from(room.saturating_sub(1).min(PANEL_ROWS)))
        } else {
            Vec::new()
        };
        let hint_h = u16::from(self.prompt_waits() && room >= 3);
        let pasted = !attach::referenced(self.editor.text(), &self.attachments).is_empty();
        let attached_h = if !panel.is_empty() || !pasted {
            0
        } else if self.images.is_some() && room >= PREVIEW_ROWS + 1 + 4 {
            PREVIEW_ROWS + 1
        } else {
            u16::from(room >= 3)
        };
        let waiting_h = if panel.is_empty() && room >= 6 {
            self.waiting.len().min(WAITING_ROWS) as u16
        } else {
            0
        };
        let wanted = if panel.is_empty() {
            self.editor.height().clamp(1, EDITOR_ROWS) as u16 + hint_h + attached_h + waiting_h
        } else {
            panel.len() as u16
        };
        let editor_h = wanted.min(room.saturating_sub(1)).max(room.min(1));
        let chat_h = room - editor_h;
        let at = |y: u16, height: u16| Rect {
            y: main.y + y,
            height,
            ..main
        };
        let hint_h = hint_h.min(editor_h.saturating_sub(1));
        let waiting_h = waiting_h.min(editor_h.saturating_sub(1 + hint_h));
        let attached_h = attached_h.min(editor_h.saturating_sub(1 + hint_h + waiting_h));
        // Too little room for a thumbnail: none, rather than half of one.
        let attached_h = if attached_h > 1 && attached_h < PREVIEW_ROWS + 1 {
            1
        } else {
            attached_h
        };
        let above = hint_h + waiting_h + attached_h;
        let top = chat_h + rule_h;
        // With fills, the rule's row, the editor and the first status line are one raised box
        // with the accent bar on its left, a row of the chat's above it; what is in the box
        // starts past the bar.
        let gap = u16::from(self.styles.fills() && chat_h > 1);
        // While a turn runs, a row of its own says so, over the input box; while idle, that the
        // cache has gone.
        let work_h = u16::from(
            (self.running || self.cache_cold || self.voice != Voice::Off) && chat_h >= gap + 6,
        );
        // A blank row parts it from the chat.
        let work_gap = u16::from(work_h > 0 && chat_h >= gap + 8);
        let inset = |r: Rect| {
            if self.styles.fills() {
                Rect {
                    x: r.x + 2,
                    width: r.width.saturating_sub(3),
                    ..r
                }
            } else {
                r
            }
        };
        Areas {
            chat: at(0, chat_h - gap - work_h - work_gap),
            work: at(chat_h - gap - work_h, work_h),
            rule: at(chat_h, rule_h),
            hint: inset(at(top, hint_h)),
            waiting: inset(at(top + hint_h, waiting_h)),
            attached: inset(at(top + hint_h + waiting_h, attached_h)),
            editor: at(chat_h + rule_h + above, editor_h - above),
            panel,
            input: at(chat_h, rule_h + editor_h + status_h.min(1)),
            status: at(chat_h + rule_h + editor_h, status_h),
            agents: at(h, agents_h),
            jobs: at(h + agents_h, jobs_h),
            side,
            terminal,
            left,
        }
    }

    /// Rows the subagents list takes of a pane `total` rows tall: none when the pane is small.
    pub(super) fn agents_rows(&self, total: u16) -> u16 {
        if total < 12 {
            return 0;
        }
        let want = self
            .agents
            .height(self.now_ms, self.focus == Some(Focus::Agents));
        u16::try_from(want).unwrap_or(u16::MAX).min(total / 3)
    }

    /// Rows the background commands take of what is left of a pane `total` rows tall.
    pub(super) fn jobs_rows(&self, total: u16) -> u16 {
        if total < 12 {
            return 0;
        }
        let rows = self.widgets.jobs(self.now_ms);
        let want = Jobs::height(&rows, self.focus == Some(Focus::Jobs));
        u16::try_from(want).unwrap_or(u16::MAX).min(total / 4)
    }

    pub(super) fn draw_jobs(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let rows = self.widgets.jobs(self.now_ms);
        let lines = self.jobs.lines(
            &rows,
            usize::from(area.width),
            self.now_ms,
            self.focus == Some(Focus::Jobs),
            &self.styles,
        );
        frame.render_widget(Paragraph::new(lines), area);
    }

    pub(super) fn draw_agents(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let main = MainRow {
            harness: &self.harness,
            running: self.running,
        };
        let lines = self.agents.lines(
            usize::from(area.width),
            self.now_ms,
            &main,
            self.focus == Some(Focus::Agents),
            &self.styles,
        );
        frame.render_widget(Paragraph::new(lines), area);
    }
}
