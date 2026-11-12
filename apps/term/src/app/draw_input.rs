use super::*;

impl App {
    /// Rows a menu over `above` (what is between the chat's top and the input box) has for its
    /// entries, and how wide they may be: past the accent bar when the box is raised, under a
    /// rule of their own when it isn't.
    pub(super) fn menu_room(&self, above: Rect) -> (usize, usize) {
        let fills = self.styles.fills();
        let rows = usize::from(above.height).saturating_sub(usize::from(!fills) + 1);
        let w = usize::from(above.width).saturating_sub(if fills { 3 } else { 0 });
        (rows.min(MENU_ROWS), w)
    }

    /// A menu's entries drawn right on top of the input box, over the chat and the working
    /// line: raised and barred like the box, or set off by a rule.
    pub(super) fn draw_on_input(&self, frame: &mut Frame, above: Rect, lines: Vec<Line<'static>>) {
        let fills = self.styles.fills();
        let height = (lines.len() + usize::from(!fills)) as u16;
        let rect = Rect {
            y: above.bottom() - height,
            height,
            ..above
        };
        frame.render_widget(Clear, rect);
        if fills {
            crate::style::fill(frame.buffer_mut(), rect, self.styles.raised);
            let bar = self.styles.accent.bg(self.styles.raised);
            for y in rect.top()..rect.bottom() {
                frame.buffer_mut()[(rect.x, y)]
                    .set_symbol("┃")
                    .set_style(bar);
            }
            let inner = Rect {
                x: rect.x + 2,
                width: rect.width.saturating_sub(3),
                ..rect
            };
            frame.render_widget(Paragraph::new(lines), inner);
        } else {
            let rule = Line::from(Span::styled(
                "─".repeat(usize::from(rect.width)),
                self.styles.border,
            ));
            let lines: Vec<Line<'static>> = std::iter::once(rule).chain(lines).collect();
            frame.render_widget(Paragraph::new(lines), rect);
        }
    }

    /// The `@` file list on the input box, like the slash menu: the files with their icons,
    /// the selected one marked.
    pub(super) fn draw_mentions(&mut self, frame: &mut Frame, above: Rect) {
        let matches = self.mention_matches();
        if matches.is_empty() {
            return;
        }
        self.mention_sel = self.mention_sel.min(matches.len() - 1);
        let (rows, w) = self.menu_room(above);
        let rows = matches.len().min(rows);
        if rows == 0 {
            return;
        }
        let start = (self.mention_sel + 1).saturating_sub(rows);
        let sigil =
            mention::at_cursor(self.editor.text(), self.editor.cursor()).map_or("@", |m| m.sigil());
        let lines = matches
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(i, c)| {
                let (marker, style) = if i == self.mention_sel {
                    ("› ", self.styles.accent.add_modifier(Modifier::BOLD))
                } else if c.dir {
                    ("  ", self.styles.muted)
                } else {
                    ("  ", Style::new())
                };
                let mut spans = vec![Span::styled(marker, self.styles.accent)];
                if self.icons {
                    spans.push(neo_renderer_ratatui::file_icon(&c.path, &self.palette));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(format!("{sigil}{}", c.path), style));
                fit_line(Line::from(spans), w)
            })
            .collect();
        self.draw_on_input(frame, above, lines);
    }

    pub(super) fn draw_menu(&mut self, frame: &mut Frame, above: Rect) {
        let matches = self.menu_matches();
        if matches.is_empty() {
            return;
        }
        self.menu_sel = self.menu_sel.min(matches.len() - 1);
        let (rows, w) = self.menu_room(above);
        let rows = matches.len().min(rows);
        if rows == 0 {
            return;
        }
        let start = (self.menu_sel + 1).saturating_sub(rows);
        let name_w = matches
            .iter()
            .map(|c| width(&c.name) + 1 + width(&c.args) + usize::from(!c.args.is_empty()))
            .max()
            .unwrap_or(0)
            .min(32);
        let lines: Vec<Line<'static>> = matches
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(i, c)| {
                let selected = i == self.menu_sel;
                let label = if c.args.is_empty() {
                    format!("/{}", c.name)
                } else {
                    format!("/{} {}", c.name, c.args)
                };
                let pad = " ".repeat((name_w + 1).saturating_sub(width(&label)));
                let (marker, name_style) = if selected {
                    ("› ", self.styles.accent.add_modifier(Modifier::BOLD))
                } else {
                    ("  ", Style::new())
                };
                fit_line(
                    Line::from(vec![
                        Span::styled(marker, self.styles.accent),
                        Span::styled(label, name_style),
                        Span::raw(format!("{pad}  ")),
                        Span::styled(c.desc.clone(), self.styles.muted),
                    ]),
                    w,
                )
            })
            .collect();
        self.draw_on_input(frame, above, lines);
    }

    /// The prompt as it fills the editor's place: the question with a compact preview, the
    /// buttons (one focused), "1 of 3" when more wait, and a line of keys (or the note being
    /// typed).
    pub(super) fn panel_lines(
        &self,
        width: u16,
        max_rows: usize,
        review_above: bool,
    ) -> Vec<Line<'static>> {
        let Some(r) = self.active_ref() else {
            return Vec::new();
        };
        let Some(item) = self.chat.item(r) else {
            return Vec::new();
        };
        if self.chat.has_targets(r) {
            return vec![self.form_hint(r, width)];
        }
        let buttons = self.chat.buttons_of(r);
        let filtering = choice::is_long(buttons);
        // A long choice shows only the options that match, numbered from its own first line.
        let shown = filtering.then(|| choice::matching(buttons, &self.prompt.filter));
        let filtered = shown
            .as_ref()
            .map(|s| neo_renderer_ratatui::keep_buttons(item, s));
        let (item, focused) = match (&filtered, &shown) {
            (Some(filtered), Some(shown)) => (
                filtered,
                shown
                    .iter()
                    .position(|&i| i == self.prompt.button)
                    .unwrap_or(usize::MAX),
            ),
            _ => (
                item,
                self.prompt.button.min(buttons.len().saturating_sub(1)),
            ),
        };
        let w = usize::from(width);
        let accent = self.styles.accent;
        let marker = self.panel_marker();
        // An edit shown in its file view above: how to comment on it, and how many comments go
        // with the answer.
        let review = self
            .widgets
            .reviewing()
            .filter(|shown| review_above && *shown == r)
            .map(|r| self.widgets.review_note_count(r));
        // The view above has the file, its name and the counts: the card is its buttons.
        let bare;
        let item = if review.is_some() {
            bare = buttons_only(item);
            &bare
        } else {
            item
        };
        let tail = match &self.note {
            Some(note) => chat::note_line(&note.text, w.saturating_sub(2), accent),
            None => Line::from(Span::styled(
                keys_hint(
                    buttons
                        .get(self.prompt.button.min(buttons.len().saturating_sub(1)))
                        .filter(|_| !shown.as_ref().is_some_and(Vec::is_empty)),
                    shown.as_ref().map_or(buttons.len(), Vec::len),
                    filtering.then_some(self.prompt.filter.as_str()),
                    review,
                ),
                self.styles.muted,
            )),
        };
        let nothing = shown.as_ref().is_some_and(Vec::is_empty);
        // The question, then the filter field, then the options.
        let head = 1 + usize::from(filtering);
        let budget = max_rows.saturating_sub(1).max(1);
        let look = chat::PanelLook {
            palette: self.palette,
            icons: self.icons,
            split_diffs: self.split_diffs,
            cwd: Some(&self.cwd),
            shown_above: self.expanded,
            now_ms: self.now_ms,
        };
        let compose = |preview: usize| {
            let mut body =
                chat::panel_lines(item, width, focused, preview, filtering, &look, marker);
            if filtering {
                let field = chat::filter_line(&self.prompt.filter, w.saturating_sub(2), accent);
                body.insert(1.min(body.len()), chat::barred(field, marker));
                if nothing {
                    body.push(chat::barred(
                        Line::from(Span::styled("no match", self.styles.muted)),
                        marker,
                    ));
                }
            }
            body
        };
        let mut body = Vec::new();
        // Expanded (Ctrl-O), the chat has the card in full, and an edit has its file view above:
        // no preview of it here again.
        let previews: &[usize] = if self.expanded || review.is_some() {
            &[0]
        } else {
            &[10, 6, 3, 1, 0]
        };
        for &preview in previews {
            body = compose(preview);
            if body.len() <= budget {
                break;
            }
        }
        if body.len() > budget {
            // Too small for the preview: keep the question (and the filter) and a window of the
            // buttons that follows the focused one, scrolling no further than it takes to show it.
            let rows = budget.saturating_sub(head);
            let mut rest = body.split_off(head.min(body.len()));
            // The focused button is on the first line that differs from a drawing without one.
            let unfocused = chat::panel_lines(item, width, usize::MAX, 0, filtering, &look, marker);
            let at = rest
                .iter()
                .zip(unfocused.iter().skip(1))
                .position(|(a, b)| a != b)
                .unwrap_or(rest.len().saturating_sub(1));
            let max_top = rest.len().saturating_sub(rows);
            let mut top = self.prompt.panel_top.get().unwrap_or(max_top);
            if at < top {
                top = at;
            } else if at >= top + rows {
                top = at + 1 - rows;
            }
            top = top.min(max_top);
            self.prompt.panel_top.set(Some(top));
            body.extend(rest.drain(top..top + rows));
        }
        let (at, of) = self.queue();
        if of > 1
            && let Some(first) = body.first_mut()
        {
            first
                .spans
                .push(Span::styled(format!("  ({at} of {of})"), self.styles.muted));
            *first = fit_line(first.clone(), w);
        }
        body.push(fit_line(chat::barred(tail, marker), w));
        body
    }

    /// What the empty input says. While the agent works: what Enter and Alt-Enter do. In a
    /// session nothing was said in yet: asks what to do, and where the commands are. After
    /// that: asks what is next. Which of a few wordings is the session's own (it does not
    /// change while it is open).
    pub(super) fn placeholder(&self) -> String {
        const FRESH: [&str; 3] = [
            "What are we building today?",
            "What are we working on?",
            "Where should we start?",
        ];
        const AFTER: [&str; 2] = ["What's next?", "Anything to change?"];
        let pick = self
            .session
            .bytes()
            .fold(5381usize, |h, b| h.wrapping_mul(33) ^ usize::from(b));
        if self.running {
            "Queue a message · alt-enter sends now".to_string()
        } else if self.chat.items().is_empty() {
            format!("{} · / for commands", FRESH[pick % FRESH.len()])
        } else {
            AFTER[pick % AFTER.len()].to_string()
        }
    }

    pub(super) fn draw_editor(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        // In the input box its bar takes the prompt's place.
        let prompt = if self.styles.fills() {
            Span::raw("  ")
        } else {
            Span::styled("> ", self.styles.accent)
        };
        if self.editor.is_empty() {
            let hint = truncate(
                &self.placeholder(),
                usize::from(area.width.saturating_sub(2)),
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    prompt,
                    Span::styled(hint, self.styles.muted),
                ])),
                area,
            );
        } else {
            let rows = self.editor.rows();
            let (row, col) = self.editor.cursor_pos();
            let h = usize::from(area.height);
            let start = if row >= h { row + 1 - h } else { 0 };
            // The markers of pasted images stand out: that is where they go.
            let marks: Vec<std::ops::Range<usize>> = attach::markers(self.editor.text())
                .into_iter()
                .filter(|(_, n)| self.attachments.iter().any(|(m, _)| m == n))
                .map(|(range, _)| range)
                .collect();
            let accent = self.styles.accent;
            let lines: Vec<Line<'static>> = rows
                .iter()
                .enumerate()
                .skip(start)
                .take(h)
                .map(|(i, r)| {
                    let lead = if i == 0 {
                        prompt.clone()
                    } else {
                        Span::raw("  ")
                    };
                    let all = self.editor.text();
                    let end = r.start + all[r.start..r.end].trim_end_matches('\n').len();
                    let mut spans = vec![lead];
                    let mut at = r.start;
                    for m in marks.iter().filter(|m| m.start < end && m.end > r.start) {
                        let (from, to) = (m.start.max(r.start), m.end.min(end));
                        if from > at {
                            spans.push(Span::raw(all[at..from].to_string()));
                        }
                        spans.push(Span::styled(all[from..to].to_string(), accent));
                        at = to;
                    }
                    if at < end {
                        spans.push(Span::raw(all[at..end].to_string()));
                    }
                    Line::from(spans)
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), area);
            if self.editor_has_cursor() {
                let x = (2 + col).min(usize::from(area.width) - 1) as u16;
                frame.set_cursor_position(Position::new(area.x + x, area.y + (row - start) as u16));
            }
            return;
        }
        if self.editor_has_cursor() {
            frame.set_cursor_position(Position::new(area.x + 2.min(area.width - 1), area.y));
        }
    }

    /// The inputs waiting behind the running turn, one muted italic `◷` row each (their first
    /// line; not the `›` of a message that was sent), the first with a `queued` tag at the right
    /// edge, the last row counting those that don't fit.
    pub(super) fn draw_waiting(&self, frame: &mut Frame, area: Rect) {
        let rows = usize::from(area.height);
        let w = usize::from(area.width);
        let shown = if self.waiting.len() > rows {
            rows - 1
        } else {
            rows
        };
        let row = self.styles.muted.add_modifier(Modifier::ITALIC);
        let mut lines: Vec<Line> = self.waiting[..shown]
            .iter()
            .enumerate()
            .map(|(i, input)| {
                let first = input
                    .text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("");
                let mut text = format!("◷ {}", first.trim());
                if input.text.trim().lines().nth(1).is_some() {
                    text.push_str(" …");
                }
                match input.images.as_ref().map_or(0, Vec::len) {
                    0 => {}
                    1 => text.push_str("  [image]"),
                    n => text.push_str(&format!("  [{n} images]")),
                }
                let tag = (i == 0).then(|| {
                    let mut spans = vec![Span::styled("queued", self.styles.accent)];
                    if self.waiting.iter().any(super::keys::is_users) {
                        spans.push(Span::styled(" · ↑ edits", self.styles.muted));
                    }
                    Line::from(spans)
                });
                let room = w.saturating_sub(tag.as_ref().map_or(0, |t| t.width() + 2));
                let left = Line::from(Span::styled(truncate(&text, room.max(1)), row));
                split_line(left, tag, w)
            })
            .collect();
        if shown < self.waiting.len() {
            let more = format!("  … +{} more queued", self.waiting.len() - shown);
            lines.push(Line::from(Span::styled(
                truncate(&more, w),
                self.styles.muted,
            )));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// The images pasted into the draft, above the editor: thumbnails with their marker under
    /// them where the terminal shows pictures, else one row of `[image 1] cat.png`.
    pub(super) fn draw_attached(&mut self, frame: &mut Frame, area: Rect) {
        let attached: Vec<(usize, String)> =
            attach::referenced(self.editor.text(), &self.attachments)
                .into_iter()
                .map(|(n, path)| (n, path.to_string()))
                .collect();
        let (accent, muted) = (self.styles.accent, self.styles.muted);
        if area.height > 1
            && let Some(images) = self.images.as_mut()
        {
            images.request(attached.iter().map(|(_, path)| path.as_str()));
            let view = images.chat_view();
            let rows = area.height - 1;
            let mut x = 0;
            for (n, path) in &attached {
                let status = view.status.get(path);
                let cols = match status {
                    Some(ImageStatus::Ready { width, height }) => {
                        thumbnail_cols(*width, *height, rows, view.cell)
                    }
                    _ => PREVIEW_COLS / 2,
                };
                if x + cols > area.width {
                    break;
                }
                let label_at = Rect {
                    x: area.x + x,
                    y: area.y + rows,
                    width: cols,
                    height: 1,
                };
                match status {
                    Some(ImageStatus::Ready { .. }) => {
                        let at = chat::Placement {
                            src: path.clone(),
                            x,
                            y: 0,
                            cols,
                            rows,
                        };
                        images.draw(frame.buffer_mut(), area, &at, muted);
                    }
                    Some(ImageStatus::Failed(_)) => frame.render_widget(
                        Paragraph::new(Span::styled("can't show it", muted)),
                        Rect {
                            y: area.y,
                            ..label_at
                        },
                    ),
                    _ => frame.render_widget(
                        Paragraph::new(Span::styled("loading…", muted)),
                        Rect {
                            y: area.y,
                            ..label_at
                        },
                    ),
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        truncate(&attach::marker(*n), usize::from(cols)),
                        accent,
                    )),
                    label_at,
                );
                x += cols + 2;
            }
            return;
        }
        let mut spans = Vec::new();
        for (n, path) in &attached {
            let name = std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            spans.push(Span::styled(attach::marker(*n), accent));
            spans.push(Span::styled(format!(" {name}  "), muted));
        }
        let line = fit_line(Line::from(spans), usize::from(area.width));
        frame.render_widget(Paragraph::new(line), area);
    }

    /// The terminal cursor sits in the editor unless something else has the keys.
    pub(super) fn editor_has_cursor(&self) -> bool {
        self.focus.is_none()
            && self.secret.is_none()
            && self.overlay != Overlay::Switcher
            && !self.github.is_open()
            && self.transcript.is_none()
    }

    /// The command Ctrl-B sends to the background: the newest the running turn waits for that has
    /// run [`BACKGROUND_AFTER_MS`]. Only neo's: other harnesses have no way to let go of one.
    pub(super) fn backgroundable(&self) -> Option<String> {
        if !self.running || self.harness != DEFAULT_HARNESS {
            return None;
        }
        self.widgets
            .foreground_commands()
            .into_iter()
            .rev()
            .find(|(_, started)| self.now_ms - started >= BACKGROUND_AFTER_MS)
            .map(|(id, _)| id)
    }
}

/// A card without its title and its diffs: what is left is its buttons.
fn buttons_only(item: &neo_protocol::ViewItem) -> neo_protocol::ViewItem {
    fn strip(node: &mut serde_json::Value) {
        if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
            children.retain(|c| c.get("type").and_then(|t| t.as_str()) != Some("diff"));
            children.iter_mut().for_each(strip);
        }
    }
    let mut value = serde_json::to_value(item).unwrap_or_default();
    if let Some(map) = value.as_object_mut() {
        map.remove("label");
    }
    if let Some(component) = value.get_mut("component") {
        strip(component);
    }
    serde_json::from_value(value).unwrap_or_else(|_| item.clone())
}
