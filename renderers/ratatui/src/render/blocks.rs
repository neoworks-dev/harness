use super::*;

impl Cx<'_> {
    pub(crate) fn text(&mut self, v: &pb::TextComponent, w: usize) -> Lines {
        let mut lines = self.blocks(&v.blocks, w, true);
        match v.role {
            Some(pb::TextRole::Title) => restyle(&mut lines, |s| s.add_modifier(Modifier::BOLD)),
            Some(pb::TextRole::Code) => {
                let code = self.p.code_style();
                restyle(&mut lines, |s| s.patch(code));
            }
            Some(pb::TextRole::Body) | None => {}
        }
        if let Some(tone) = v.tone {
            tint(&mut lines, self.p.tone(tone));
        }
        lines
    }

    /// Loose blocks are separated by a blank line; tight ones (inside list items) stack.
    pub(crate) fn blocks(&mut self, bs: &[pb::Block], w: usize, loose: bool) -> Lines {
        let mut out: Lines = Vec::new();
        for b in bs {
            let lines = self.block(b, w);
            if lines.is_empty() {
                continue;
            }
            if loose && !out.is_empty() {
                out.push(Line::default());
            }
            out.extend(lines);
        }
        out
    }

    pub(super) fn block(&mut self, b: &pb::Block, w: usize) -> Lines {
        match b {
            pb::Block::ParagraphBlock(v) => wrap(&self.spans(&v.spans), self.prose(w)),
            pb::Block::HeadingBlock(v) => {
                let style = self.heading_style(v.level);
                let mut spans = self.spans(&v.spans);
                for s in &mut spans {
                    s.style = style.patch(s.style);
                }
                wrap(&spans, self.prose(w))
            }
            pb::Block::ListBlock(v) => self.list(v, w),
            pb::Block::CodeBlock(v) => self.code(&v.code, v.language.as_deref(), false, w),
            pb::Block::QuoteBlock(v) => {
                let mut inner = self.blocks(&v.blocks, w.saturating_sub(2).max(1), true);
                tint(&mut inner, self.p.muted);
                let bar = self.p.muted_style();
                inner
                    .into_iter()
                    .map(|line| {
                        if line.spans.is_empty() {
                            sline("│", bar)
                        } else {
                            let mut spans = vec![Span::styled("│ ", bar)];
                            spans.extend(line.spans);
                            Line::from(spans)
                        }
                    })
                    .collect()
            }
            pb::Block::RuleBlock(_) => vec![sline(
                "─".repeat(self.prose(w)),
                Style::new().fg(self.p.border),
            )],
            pb::Block::TableBlock(v) => self.table_block(v, w),
            pb::Block::NeoUiBlock(v) => self.neoui_doc(&v.ui, w),
            pb::Block::Unknown(_) => vec![self.placeholder("[unsupported block]", w)],
        }
    }

    /// Where prose wraps: the width, or `prose_width` when that is narrower.
    pub(super) fn prose(&self, w: usize) -> usize {
        self.o.prose_width.map_or(w, |cap| w.min(cap.max(1)))
    }

    /// Levels 1 and 2 bold in the accent color, the rest bold. Not underlined: that is how
    /// links look.
    pub(super) fn heading_style(&self, level: i64) -> Style {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        match level {
            1 | 2 => bold.fg(self.p.accent),
            _ => bold,
        }
    }

    /// A markdown table in a box. Cells wrap inside their column; columns that don't fit share
    /// the room (the widest give first). Once a row takes more than one line, rules separate the
    /// rows too. Too narrow for the box: each row as `header: cell` lines.
    pub(super) fn table_block(&self, v: &pb::TableBlock, w: usize) -> Lines {
        let cols = v
            .rows
            .iter()
            .map(Vec::len)
            .chain([v.header.len()])
            .max()
            .unwrap_or(0);
        if cols == 0 {
            return Vec::new();
        }
        let cells = |row: &[pb::TableCell]| -> Vec<Vec<Span<'static>>> {
            (0..cols)
                .map(|c| {
                    row.get(c)
                        .map(|cell| self.spans(&cell.spans))
                        .unwrap_or_default()
                })
                .collect()
        };
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let mut header = cells(&v.header);
        for cell in &mut header {
            for s in cell.iter_mut() {
                s.style = bold.patch(s.style);
            }
        }
        let body: Vec<_> = v.rows.iter().map(|r| cells(r)).collect();

        // "│ " before each column, " │" after the last, " │ " between.
        let frame = 3 * cols + 1;
        if w < frame + cols {
            return self.stacked_table(&header, &body, w);
        }
        let natural = |cell: &[Span<'static>]| {
            wrap(cell, usize::MAX / 2)
                .iter()
                .map(|l| line_width(&l.spans))
                .max()
                .unwrap_or(0)
        };
        let mut widths: Vec<usize> = (0..cols)
            .map(|c| {
                body.iter()
                    .chain([&header])
                    .map(|row| natural(&row[c]))
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        fit_columns(&mut widths, w - frame);

        let align = |c: usize| {
            v.align
                .as_ref()
                .and_then(|a| a.get(c).copied())
                .unwrap_or(pb::ColumnAlign::Left)
        };
        let border = Style::new().fg(self.p.border);
        let row_lines = |row: &[Vec<Span<'static>>]| -> Lines {
            let wrapped: Vec<Lines> = row
                .iter()
                .zip(&widths)
                .map(|(cell, cw)| wrap(cell, *cw))
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
            (0..height)
                .map(|i| {
                    let mut spans = vec![Span::styled("│ ", border)];
                    for (c, cw) in widths.iter().enumerate() {
                        if c > 0 {
                            spans.push(Span::styled(" │ ", border));
                        }
                        let line = wrapped[c]
                            .get(i)
                            .map(|l| l.spans.clone())
                            .unwrap_or_default();
                        let room = cw.saturating_sub(line_width(&line));
                        let (left, right) = match align(c) {
                            pb::ColumnAlign::Left => (0, room),
                            pb::ColumnAlign::Right => (room, 0),
                            pb::ColumnAlign::Center => (room / 2, room - room / 2),
                        };
                        if left > 0 {
                            spans.push(Span::raw(" ".repeat(left)));
                        }
                        spans.extend(line);
                        if right > 0 {
                            spans.push(Span::raw(" ".repeat(right)));
                        }
                    }
                    spans.push(Span::styled(" │", border));
                    Line::from(spans)
                })
                .collect()
        };
        let rule = |l: &str, m: &str, r: &str| {
            let inner: Vec<String> = widths.iter().map(|cw| "─".repeat(cw + 2)).collect();
            sline(format!("{l}{}{r}", inner.join(m)), border)
        };
        let rows: Vec<Lines> = body.iter().map(|r| row_lines(r)).collect();
        let ruled = rows.iter().any(|r| r.len() > 1);
        let mut out = vec![rule("┌", "┬", "┐")];
        out.extend(row_lines(&header));
        out.push(rule("├", "┼", "┤"));
        for (i, row) in rows.into_iter().enumerate() {
            if ruled && i > 0 {
                out.push(rule("├", "┼", "┤"));
            }
            out.extend(row);
        }
        out.push(rule("└", "┴", "┘"));
        clip(&mut out, w);
        out
    }

    /// A table too narrow for its box: each row's cells as `header: cell`, rows apart.
    pub(super) fn stacked_table(
        &self,
        header: &[Vec<Span<'static>>],
        body: &[Vec<Vec<Span<'static>>>],
        w: usize,
    ) -> Lines {
        let mut out = Vec::new();
        for row in body {
            if !out.is_empty() {
                out.push(Line::default());
            }
            for (name, cell) in header.iter().zip(row) {
                let mut spans = name.clone();
                if !spans.is_empty() {
                    spans.push(Span::styled(
                        ": ",
                        Style::new().add_modifier(Modifier::BOLD),
                    ));
                }
                spans.extend(cell.iter().cloned());
                out.extend(wrap(&spans, w));
            }
        }
        out
    }

    pub(super) fn spans(&self, spans: &[pb::Span]) -> Vec<Span<'static>> {
        let mut out = Vec::new();
        for s in spans {
            let mut st = Style::new();
            if s.bold == Some(true) {
                st = st.add_modifier(Modifier::BOLD);
            }
            if s.italic == Some(true) {
                st = st.add_modifier(Modifier::ITALIC);
            }
            if s.strike == Some(true) {
                st = st.add_modifier(Modifier::CROSSED_OUT);
            }
            if s.code == Some(true) {
                st = st.patch(self.p.code_style());
            }
            let text = clean(&s.text);
            match s.href.as_deref().filter(|h| !h.is_empty()) {
                Some(href) => out.extend(link(href, Some(&text), st, &self.p)),
                None => push_text(&mut out, &text, st),
            }
        }
        out
    }

    /// Bullets, numbers and task boxes; item text hangs under the marker's text, nested lists
    /// sit under the item's text.
    pub(super) fn list(&mut self, v: &pb::ListBlock, w: usize) -> Lines {
        let start = v.start.unwrap_or(1);
        let last = start.saturating_add(v.items.len() as i64 - 1);
        let num_w = if v.ordered {
            last.to_string().len().max(start.to_string().len())
        } else {
            0
        };
        let mut out = Vec::new();
        for (i, it) in v.items.iter().enumerate() {
            let mut marker: Vec<Span<'static>> = Vec::new();
            if v.ordered {
                let n = start.saturating_add(i as i64);
                marker.push(Span::raw(format!("{n:>num_w$}. ")));
            } else if it.checked.is_none() {
                marker.push(Span::raw("• "));
            }
            match it.checked {
                Some(true) => marker.push(Span::styled("[x] ", self.p.fg(self.p.success))),
                Some(false) => marker.push(Span::styled("[ ] ", self.p.muted_style())),
                None => {}
            }
            let pad = line_width(&marker);
            let mut body = self.blocks(&it.blocks, w.saturating_sub(pad).max(1), false);
            if body.is_empty() {
                body.push(Line::default());
            }
            out.extend(prefixed(body, marker, vec![Span::raw(" ".repeat(pad))]));
        }
        out
    }

    /// Monospace on the surface color, indented 2, hard-wrapped, the band filling the width.
    /// Highlighted by the grammar for its language; a shell command with `reflow` (one to
    /// approve, not code the model wrote in its reply) also breaks after `&&`, `|` and `;`.
    pub(crate) fn code(&self, code: &str, language: Option<&str>, reflow: bool, w: usize) -> Lines {
        let st = self.p.code_style();
        if let Some(syntax) = language.and_then(syntax::syntax_for_language) {
            let inner = w.saturating_sub(2).max(1);
            let code = clean(code);
            let code = if reflow && crate::shell::is_shell(language) {
                crate::shell::reflow(&code, inner).into()
            } else {
                code
            };
            let spans = if self.o.plain_code {
                vec![Span::styled(code.into_owned(), st)]
            } else {
                syntax::highlight_spans(&code, syntax, &syntax::block_colors(&self.p), st)
            };
            return hard_wrap_spans(&spans, inner)
                .into_iter()
                .map(|mut line| {
                    let used = line_width(&line);
                    if w >= 4 {
                        line.insert(0, Span::styled("  ", st));
                        line.push(Span::styled(" ".repeat(inner - used), st));
                    }
                    Line::from(line)
                })
                .collect();
        }
        if w < 4 {
            return hard_wrap(code, w)
                .into_iter()
                .map(|l| sline(l, st))
                .collect();
        }
        hard_wrap(code, w - 2)
            .into_iter()
            .map(|l| {
                let pad = " ".repeat(w - 2 - width(&l));
                sline(format!("  {l}{pad}"), st)
            })
            .collect()
    }
}
