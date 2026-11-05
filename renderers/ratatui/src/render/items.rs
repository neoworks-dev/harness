use super::*;

impl Cx<'_> {
    // ---- items ----

    pub(crate) fn item(&mut self, it: &pb::ViewItem, w: usize) -> Lines {
        let label = it.label.as_deref().map(one_line).filter(|l| !l.is_empty());
        let mut lines = match it.kind {
            pb::ItemKind::User => self.user(it, label, w),
            pb::ItemKind::Agent => self.agent(it, label, w),
            pb::ItemKind::Thought => self.thought(it, label, w),
            pb::ItemKind::Tool => self.tool(it, label, w),
            pb::ItemKind::Notice => self.notice(it, w),
        };
        self.place_rooms(&mut lines);
        self.place_focus(&mut lines);
        clip(&mut lines, w);
        lines
    }

    /// Finds the marked lines (the rooms for images, in order), takes the marks out and notes
    /// each room's line and column. However the components were nested, the marker line's text
    /// before the mark is exactly the room's indent.
    pub(super) fn place_rooms(&mut self, lines: &mut Lines) {
        let rooms = std::mem::take(&mut self.rooms);
        let mut found: Vec<(usize, u16)> = Vec::new();
        for (i, line) in lines.iter_mut().enumerate() {
            while let Some(at) = line.spans.iter().position(|s| s.content == SLOT_MARK) {
                let x = line_width(&line.spans[..at]);
                line.spans.remove(at);
                found.push((i, u16::try_from(x).unwrap_or(u16::MAX)));
            }
        }
        if found.len() != rooms.len() {
            return;
        }
        self.slots = found
            .into_iter()
            .zip(rooms)
            .map(|((line, x), (cols, rows, src))| ImageSlot {
                line,
                x,
                cols,
                rows,
                src,
            })
            .collect();
    }

    /// `› text` on a full-width band (`Palette::band`), so what the user said stands out
    /// from the agent's replies; the app it came from dimmed after the text.
    pub(super) fn user(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
        let inner = w.saturating_sub(2).max(1);
        let mut body = self.comp(&it.component, inner);
        let own = |label: &str| {
            self.o
                .app
                .as_deref()
                .is_some_and(|app| app_family(app) == app_family(label))
        };
        if let Some(label) = label.filter(|l| !own(l)) {
            let tag = format!("· {label}");
            match body.first_mut() {
                Some(first) if first.width() + 2 + width(&tag) <= inner => {
                    first
                        .spans
                        .push(Span::styled(format!("  {tag}"), self.p.muted_style()));
                }
                _ => body.push(sline(truncate(&tag, inner), self.p.muted_style())),
            }
        }
        // A theme with raised boxes: one, a row taller each way, with the accent bar down its
        // left like the input's.
        let raised = self.p.raised != Color::Reset;
        let accent = Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD);
        let bar = || vec![Span::styled("┃", accent), Span::raw(" ")];
        let lead = if raised {
            bar()
        } else {
            vec![Span::styled("›", accent), Span::raw(" ")]
        };
        let rest = if raised { bar() } else { vec![Span::raw("  ")] };
        // The bar runs through the blank lines between paragraphs: one message, not several.
        for line in &mut body {
            if line.spans.is_empty() {
                line.spans.push(Span::raw(""));
            }
        }
        let mut lines = if body.is_empty() {
            vec![Line::from(lead)]
        } else {
            prefixed(body, lead, rest)
        };
        if raised {
            lines.insert(0, Line::from(bar()));
            lines.push(Line::from(bar()));
        }
        let band = if raised { self.p.raised } else { self.p.band() };
        for line in &mut lines {
            let pad = w.saturating_sub(line.width());
            if pad > 0 {
                line.spans.push(Span::raw(" ".repeat(pad)));
            }
            for span in &mut line.spans {
                span.style = span.style.bg(band);
            }
        }
        lines
    }

    /// The text as it is; a guest agent's gets a small header with its name.
    pub(super) fn agent(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
        let mut out = Vec::new();
        if let Some(label) = label {
            out.push(sline(
                truncate(&label, w),
                Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD),
            ));
        }
        out.extend(self.comp(&it.component, w));
        out
    }

    pub(super) fn thought(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
        let muted = self.p.muted_style();
        let collapsed = || {
            let text = match &label {
                Some(l) => format!("∴ {l} thinking"),
                None => "∴ thinking".to_string(),
            };
            vec![sline(truncate(&text, w), muted)]
        };
        if !self.o.show_thoughts {
            return collapsed();
        }
        let mut body = self.comp(&it.component, w.saturating_sub(2).max(1));
        if body.is_empty() {
            return collapsed();
        }
        let color = self.p.muted;
        restyle(&mut body, |s| s.fg(color).add_modifier(Modifier::ITALIC));
        prefixed(body, vec![Span::styled("∴ ", muted)], vec![Span::raw("  ")])
    }

    pub(super) fn notice(&mut self, it: &pb::ViewItem, w: usize) -> Lines {
        let mut body = self.comp(&it.component, w);
        if it.state == Some(pb::ItemState::Failed) {
            let color = self.p.danger;
            restyle(&mut body, |s| s.fg(color));
        } else {
            tint(&mut body, self.p.muted);
        }
        body
    }

    /// A header (the label, red when the call failed), then the result: summarized under `⎿`
    /// in muted, or in full when expanded. Items with buttons are always shown in full: the
    /// buttons are what the item is for.
    pub(super) fn tool(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
        let failed = it.state == Some(pb::ItemState::Failed);
        // A doc with fields or buttons is a form to fill in; one that holds focus is in use.
        let prompt = has_buttons(&it.component)
            || crate::neoui::interactive(&it.component)
            || self.o.neoui.values().any(|s| s.focus.is_some());
        // A call that has run is history: a long label or command is cut, unless everything is
        // expanded.
        let finished = matches!(it.state, Some(pb::ItemState::Done | pb::ItemState::Failed));
        let cut = finished && !prompt && !self.o.expanded;
        let hw = w;
        // `edit /home/me/site/src/a.rs` says `edit src/a.rs` in the session's directory.
        let label = match (label, self.o.cwd.as_deref()) {
            (Some(label), Some(cwd)) if !cwd.trim_end_matches('/').is_empty() => {
                Some(label.replace(&format!("{}/", cwd.trim_end_matches('/')), ""))
            }
            (label, _) => label,
        };
        // A finished edit is one line, its counts after its path: `edit src/a.rs  +3 −1`.
        if cut && !failed && it.command.is_none() {
            let edit = label
                .as_deref()
                .zip(crate::summary::lone_diff(&it.component));
            if let Some((line, d)) = edit.and_then(|(l, d)| Some((self.edit_line(l, d, w)?, d))) {
                let mut out = vec![line];
                if self.o.open_edits {
                    let lines = around_changes(&d.lines, EDIT_CONTEXT);
                    let mut rows = self.diff_rows(d.path.as_deref(), &lines, w);
                    if rows.len() > EDIT_ROWS {
                        let more = rows.len() - (EDIT_ROWS - 1);
                        rows.truncate(EDIT_ROWS - 1);
                        rows.push(sline(
                            format!("… +{more} lines (tab, enter opens the file)"),
                            self.p.muted_style(),
                        ));
                    }
                    out.extend(rows);
                }
                return out;
            }
        }
        // A question about an edit shows it as it will look once made, `edit src/a.rs  +3 −1`
        // over its diff, and then the buttons.
        if prompt && !self.o.expanded && !failed {
            let edit = label
                .as_deref()
                .zip(crate::summary::asked_diff(&it.component));
            if let Some((line, d)) = edit.and_then(|(l, d)| Some((self.edit_line(l, d, w)?, d))) {
                self.budget = self.o.preview_lines;
                let rows = self.diff_rows(d.path.as_deref(), &d.lines, w);
                let mut out = vec![line];
                out.extend(self.preview(rows));
                self.skip_diffs = true;
                let rest = self.comp(&it.component, w.saturating_sub(2).max(1));
                self.skip_diffs = false;
                if !rest.is_empty() {
                    out.push(Line::default());
                    out.extend(indented(rest, 2));
                }
                return out;
            }
        }
        let mut head = match &label {
            Some(label) => wrap(&self.tool_label(label), hw),
            None => Vec::new(),
        };
        // A command that has run is one line without its output: what it was for when it says,
        // else the command. The output (and the whole command) are in the expanded view.
        let described = it
            .description
            .as_deref()
            .filter(|d| cut && it.command.is_some() && !d.trim().is_empty());
        let mut alone = described.is_some();
        // A shell command is highlighted while it runs or waits, muted once it has run, and
        // broken after its `&&`, `||` and `;` when long (pipelines stay whole). Unless it is a
        // question or everything is expanded, it takes a few rows while it runs and one after.
        // A `cd` into the session's directory is left out.
        if let Some(description) = described {
            // Muted and italic, as the terminal pane has it: it steps back from the reply.
            let style = self.p.muted_style().add_modifier(Modifier::ITALIC);
            head.extend(wrap(
                &[Span::styled(clean(description).into_owned(), style)],
                hw,
            ));
            cut_rows(&mut head, LABEL_ROWS, hw);
        } else if let Some(command) = it
            .command
            .as_deref()
            .map(|c| crate::strip_cd(c, self.o.cwd.as_deref()))
            .filter(|c| !c.trim().is_empty())
        {
            // Cut to its first row, it reads on as far as that goes.
            let code = if cut {
                clean(command.trim_end()).into_owned()
            } else {
                crate::shell::reflow_lists(clean(command.trim_end()).as_ref(), hw)
            };
            let spans = crate::shell::highlight(&code, &self.p, Style::new());
            let mut lines: Lines = hard_wrap_spans(&spans, hw)
                .into_iter()
                .map(Line::from)
                .collect();
            // Once it has run it steps back, muted and italic like a description: the agent's
            // prose is what to read.
            if finished {
                let muted = self.p.muted;
                restyle(&mut lines, |st| {
                    st.fg(muted)
                        .remove_modifier(Modifier::BOLD)
                        .add_modifier(Modifier::ITALIC)
                });
            }
            if !prompt && !self.o.expanded {
                cut_rows(&mut lines, if finished { 1 } else { RUNNING_ROWS }, hw);
            }
            if cut {
                cut_rows(&mut head, LABEL_ROWS, hw);
                alone = true;
            }
            head.extend(lines);
        } else if cut {
            cut_rows(&mut head, LABEL_ROWS, hw);
        }
        if failed {
            let danger = self.p.danger;
            restyle(&mut head, |st| st.fg(danger));
        }
        let mut out = head;
        if alone {
            return out;
        }
        if self.o.expanded || prompt {
            self.budget = self.o.preview_lines.filter(|_| prompt && !self.o.expanded);
            let body = self.comp(&it.component, w.saturating_sub(2).max(1));
            if let Some(detail) = self.focused_detail.take() {
                self.title_detail(&mut out, detail, w);
            }
            out.extend(indented(body, 2));
        } else {
            let mut summary = self.summary(&it.component, w.saturating_sub(4).max(1));
            tint(&mut summary, self.p.muted);
            let elbow = vec![Span::styled("  ⎿ ", self.p.muted_style())];
            out.extend(prefixed(summary, elbow, vec![Span::raw("    ")]));
        }
        out
    }

    /// The focused option's description after the prompt's title: on its last line when one
    /// line of it fits there, else under it.
    pub(super) fn title_detail(&self, head: &mut Lines, detail: Lines, w: usize) {
        const GAP: &str = "  ";
        let fits = |last: &Line<'_>| last.width() + GAP.len() + detail[0].width() <= w;
        match head.last_mut() {
            Some(last) if detail.len() == 1 && fits(last) => {
                last.spans.push(Span::raw(GAP));
                last.spans.extend(detail.into_iter().flat_map(|l| l.spans));
            }
            _ => head.extend(detail),
        }
    }

    /// The verb bold, what it acts on in the normal weight: `read` `src/a.rs`, `search the web:`
    /// `query`. neo's commands (`bash: ls -la`) get the command highlighted.
    pub(super) fn tool_label(&self, label: &str) -> Vec<Span<'static>> {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        if let Some(command) = label.strip_prefix("bash: ") {
            let command = crate::strip_cd(command, self.o.cwd.as_deref());
            let mut spans = vec![Span::styled("bash: ".to_string(), bold)];
            spans.extend(crate::shell::highlight(command, &self.p, Style::new()));
            return spans;
        }
        // A short lead-in ending in a colon is the verb; else the first word is.
        let split = label
            .find(": ")
            .filter(|&at| label[..at].split(' ').count() <= 3)
            .map(|at| at + 2)
            .or_else(|| label.find(' ').map(|at| at + 1));
        match split {
            Some(at) => vec![
                Span::styled(label[..at].to_string(), bold),
                Span::raw(label[at..].to_string()),
            ],
            None => vec![Span::styled(label.to_string(), bold)],
        }
    }
}
