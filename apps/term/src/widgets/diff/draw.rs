use super::*;

impl DiffView {
    /// The columns of the split view at this width, None when the unified view is shown.
    pub(super) fn split_geom(&self, w: usize) -> Option<Geom> {
        if !self.aligned.has_lines() || !self.force.unwrap_or(w >= SPLIT_MIN) {
            return None;
        }
        let gutter = self.aligned.gutter + 3;
        let left = w.saturating_sub(1) / 2;
        let right = w.saturating_sub(1) - left;
        // The left column is the narrower one.
        if left < gutter + MIN_TEXT {
            return None;
        }
        Some(Geom {
            gutter: self.aligned.gutter,
            left,
            right,
        })
    }

    /// Fills `area`: a title line (and the column headers of the split view), then the diff,
    /// long lines cut.
    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        frame.render_widget(Clear, area);
        let w = area.width as usize;
        let look = Look::new(palette);
        let geom = self.split_geom(w);
        let split = geom.is_some();
        let head = if split { 2 } else { 1 };
        let rows = (area.height as usize).saturating_sub(head);
        let col_w = geom.map_or(w.saturating_sub(1 + self.uni_gutter(w)), |g| {
            g.left - g.gutter - 3
        });
        self.colors.set((look.accent, palette.muted));
        if self.to_change.take()
            && let (Some(at), Some(change)) =
                (self.change_position(split), self.first_change(split))
        {
            self.scroll.set(at);
            self.cursor.set(change);
            self.scroll_split.set(split);
        }
        // A resize (or `s`) changed the mode: land on the same hunk, the cursor on its line.
        if self.scroll_split.get() != split {
            let from = self.scroll_split.get();
            let pos = self.convert(self.scroll.get(), from);
            self.scroll.set(pos);
            self.cursor.set(self.convert_row(self.cursor.get(), from));
            if let Some(v) = self.visual.get() {
                let row = self.convert_row(v.row, from);
                self.visual.set(Some(Visual { row, ..v }));
            }
            self.scroll_split.set(split);
        }
        self.view.set(View {
            rows,
            col_w,
            split,
            width: w,
        });
        // The cursor stays in view however the size changed.
        let cur = self.cursor.get();
        if cur < self.scroll.get().min(self.max_scroll()) || cur > self.last_visible() {
            self.scroll.set(self.top_for(cur, split, rows));
        }
        let n = self.len(split);
        let first = self.scroll.get().min(self.max_scroll());
        let hscroll = self.hscroll.min(self.max_hscroll());

        // Highlighted: what is on screen, and a screen above and below it for scrolling.
        let lazy = if split {
            &self.lit_split
        } else {
            &self.lit_uni
        };
        if let Some(lazy) = lazy {
            lazy.want(first.saturating_sub(rows), first + 2 * rows.max(1));
        }
        let (body, shown) = match &geom {
            Some(g) => self.split_lines(g, first, rows, hscroll, &look),
            None => self.unified_lines(w, first, rows, hscroll, &look),
        };
        let mut lines = vec![self.title_line(w, first, shown, n, &look)];
        if let Some(g) = &geom {
            lines.push(self.column_header(g, &look));
        }
        lines.extend(body);
        lines.truncate(area.height as usize);
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// The path, the counts of added and removed lines, and where the view is.
    pub(super) fn title_line(
        &self,
        w: usize,
        first: usize,
        shown: usize,
        n: usize,
        look: &Look,
    ) -> Line<'static> {
        let muted = look.muted;
        // A search being typed takes the line.
        if let Some(text) = &self.typing {
            let shown = truncate_start(text, w.saturating_sub(2).max(1));
            return Line::from(vec![
                Span::styled("/", Style::new().fg(look.accent)),
                Span::raw(shown),
                Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
            ]);
        }
        let mut title = vec![Span::styled(
            truncate_start(&self.title, w.saturating_sub(14).max(1)),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(message) = &self.message {
            title.push(Span::styled(
                format!("  {message}"),
                Style::new().fg(look.danger),
            ));
        }
        if !self.loaded {
            title.push(Span::styled("  …", muted));
        } else if self.adds + self.dels > 0 {
            title.push(Span::raw("  "));
            title.push(Span::styled(
                format!("+{}", self.adds),
                Style::new().fg(look.success),
            ));
            title.push(Span::raw(" "));
            title.push(Span::styled(
                format!("−{}", self.dels),
                Style::new().fg(look.danger),
            ));
        }
        if self.proposed {
            title.push(Span::styled("  proposed edit", muted));
        } else if self.full {
            title.push(Span::styled("  full file", muted));
        }
        let used: usize = title.iter().map(|s| width(&s.content)).sum();
        if shown > 0 {
            let (a, b) = (first + 1, first + shown);
            let long = format!("lines {a}–{b} of {n}");
            let short = format!("{a}–{b}/{n}");
            if let Some(pos) = [long, short].into_iter().find(|p| used + 2 + width(p) <= w) {
                title.push(Span::raw(" ".repeat(w - used - width(&pos))));
                title.push(Span::styled(pos, muted));
            }
        }
        Line::from(title)
    }

    /// "HEAD │ working tree" over the two columns.
    pub(super) fn column_header(&self, g: &Geom, look: &Look) -> Line<'static> {
        let rename = self
            .req
            .as_ref()
            .and_then(|r| r.orig.as_ref().map(|o| (o, &r.rel)));
        let left = if self.aligned.new_file {
            "(new file)".to_string()
        } else if let Some((orig, _)) = rename {
            format!("HEAD · {}", clean_line(orig))
        } else {
            "HEAD".to_string()
        };
        let right = if self.aligned.deleted {
            "(deleted)".to_string()
        } else if let Some((_, rel)) = rename {
            format!("working tree · {}", clean_line(rel))
        } else {
            "working tree".to_string()
        };
        let style = with_bg(look.muted.add_modifier(Modifier::BOLD), look.band);
        Line::from(vec![
            Span::styled(fit(&format!(" {left}"), g.left), style),
            Span::styled("│", look.border),
            Span::styled(fit(&format!(" {right}"), g.right), style),
        ])
    }

    /// `rows` screen lines of the split view from row `first`, each exactly as wide as the
    /// overlay, and how many rows they show (a wrapped row takes several lines).
    pub(super) fn split_lines(
        &self,
        g: &Geom,
        first: usize,
        rows: usize,
        hscroll: usize,
        look: &Look,
    ) -> (Vec<Line<'static>>, usize) {
        let total = g.left + 1 + g.right;
        let mut out = Vec::with_capacity(rows);
        let mut shown = 0;
        for (index, row) in self.aligned.rows.iter().enumerate().skip(first) {
            if out.len() >= rows {
                break;
            }
            shown += 1;
            for block in self.blocks_at(index, true) {
                out.extend(block);
            }
            let marks = self.marks(index, true);
            // A separator or a note across both sides; the cursor can be on it.
            let across = |text: &str, offset: usize, style: Style| {
                let mut chars: Vec<(char, usize)> = text.chars().map(|c| (c, 0)).collect();
                let mut styles = vec![style; 3];
                let extra = paint(&mut chars, &mut styles, marks, offset, look.selection);
                let cells = width(text) + extra;
                Line::from(windowed(chars.into_iter(), cells, 0, total, &styles, true))
            };
            match row {
                Row::Hunk(text) => {
                    out.push(across(
                        &format!("⋯ {text}"),
                        2,
                        with_bg(look.muted, look.band),
                    ));
                }
                Row::Note(text) => out.push(across(text, 0, look.muted)),
                Row::Line {
                    left,
                    right,
                    changed,
                } => {
                    let (lc, rc) = match self.lit_split.as_ref().and_then(|l| l.get(index)) {
                        Some((l, r)) => (l.as_slice(), r.as_slice()),
                        None => (&[][..], &[][..]),
                    };
                    let (lt, rt) = if *changed {
                        (Tone::Del, Tone::Add)
                    } else {
                        (Tone::Ctx, Tone::Ctx)
                    };
                    let (gut, wrap) = (g.gutter, self.wrap);
                    // The cursor is on the new side, or the old one when only it has a line.
                    let (lm, rm) = if right.is_some() {
                        (Marks::default(), marks)
                    } else {
                        (marks, Marks::default())
                    };
                    let (lw, rw) = ((gut, g.left, hscroll, wrap), (gut, g.right, hscroll, wrap));
                    let l = side_rows(left.as_ref(), lc, lt, lw, lm, look);
                    let r = side_rows(right.as_ref(), rc, rt, rw, rm, look);
                    // The shorter side goes on blank, in its tint, as long as the other wraps.
                    for i in 0..l.len().max(r.len()) {
                        let mut spans = l
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| side_filler(left.is_some(), lt, g.left, look));
                        spans.push(Span::styled("│", look.border));
                        spans.extend(
                            r.get(i)
                                .cloned()
                                .unwrap_or_else(|| side_filler(right.is_some(), rt, g.right, look)),
                        );
                        out.push(Line::from(spans));
                    }
                }
            }
        }
        out.truncate(rows);
        // Below the last row the columns go on.
        while out.len() < rows {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(g.left)),
                Span::styled("│", look.border),
                Span::raw(" ".repeat(g.right)),
            ]));
        }
        (out, shown)
    }

    /// Cells the line numbers of the unified view take, with the space after them; none when
    /// no line has one (a note) or `w` leaves too little room for the text.
    pub(super) fn uni_gutter(&self, w: usize) -> usize {
        let gutter = self.aligned.gutter.max(3) + 1;
        if self.numbers.iter().any(Option::is_some) && w >= gutter + MIN_TEXT {
            gutter
        } else {
            0
        }
    }

    /// `rows` screen lines of the unified view from line `first`, and how many lines they show
    /// (a wrapped line takes several rows).
    pub(super) fn unified_lines(
        &self,
        w: usize,
        first: usize,
        rows: usize,
        hscroll: usize,
        look: &Look,
    ) -> (Vec<Line<'static>>, usize) {
        let mut out = Vec::with_capacity(rows);
        let mut shown = 0;
        for (index, l) in self.lines.iter().enumerate().skip(first) {
            if out.len() >= rows {
                break;
            }
            shown += 1;
            for block in self.blocks_at(index, false) {
                out.extend(block);
            }
            out.extend(self.unified_rows(index, l, w, hscroll, look));
        }
        out.truncate(rows);
        (out, shown)
    }

    /// The screen rows of unified line `index`: one, or more when it wraps.
    pub(super) fn unified_rows(
        &self,
        index: usize,
        l: &DiffLine,
        w: usize,
        hscroll: usize,
        look: &Look,
    ) -> Vec<Line<'static>> {
        let style = match l.kind {
            Kind::Add => Style::new().fg(look.success),
            Kind::Del => Style::new().fg(look.danger),
            Kind::Hunk | Kind::Header | Kind::Note => look.muted,
            Kind::Context => Style::new(),
        };
        // The +/-/space stays put when scrolling sideways.
        let (mark, body) = match l.kind {
            Kind::Add | Kind::Del | Kind::Context => l
                .text
                .split_at(l.text.chars().next().map_or(0, char::len_utf8)),
            _ => ("", l.text.as_str()),
        };
        let gutter = self.uni_gutter(w);
        let avail = w.saturating_sub(width(mark) + gutter);
        // A removed line is the old version's; the rest the new one's.
        let classes = match self.lit_uni.as_ref().and_then(|l| l.get(index)) {
            Some((old, _)) if l.kind == Kind::Del => old.as_slice(),
            Some((_, new)) => new.as_slice(),
            None => &[][..],
        };
        // Highlighted code keeps the change's color as a background tint.
        let tone = match l.kind {
            Kind::Add => Tone::Add,
            Kind::Del => Tone::Del,
            _ => Tone::Ctx,
        };
        let lit = !classes.is_empty() && look.syntax.is_some();
        let base = if lit {
            with_bg(Style::new(), look.bg(tone))
        } else {
            style
        };
        let mut styles = vec![base, base, base];
        let mut chars: Vec<(char, usize)> = body
            .chars()
            .enumerate()
            .map(|(i, c)| match look.code(classes.get(i).copied()) {
                Some(code) => (c, style_id(&mut styles, base.patch(code))),
                None => (c, 0),
            })
            .collect();
        let marks = self.marks(index, false);
        let extra = paint(&mut chars, &mut styles, marks, 0, look.selection);
        let pad = lit && tone != Tone::Ctx;
        let total = width(body) + extra;
        let text = fitted(chars, total, hscroll, avail, &styles, pad, self.wrap);
        text.into_iter()
            .enumerate()
            .map(|(i, text)| {
                // The number and the sign on the first row; under them the line goes on.
                let mut spans = Vec::new();
                if i == 0 {
                    if gutter > 0 {
                        let no = self.numbers.get(index).copied().flatten();
                        let no = no.map_or_else(String::new, |n| n.to_string());
                        spans.push(Span::styled(format!("{no:>0$} ", gutter - 1), look.muted));
                    }
                    spans.push(Span::styled(mark.to_string(), style));
                } else {
                    spans.push(Span::styled(" ".repeat(gutter + width(mark)), base));
                }
                spans.extend(text);
                Line::from(spans)
            })
            .collect()
    }

    /// Screen rows row `i` of the shown mode takes: one, or more when it wraps.
    pub(super) fn height(&self, i: usize, split: bool) -> usize {
        let blocks: usize = if self.comments.is_empty() && self.writing.is_none() {
            0
        } else {
            self.blocks_at(i, split).iter().map(Vec::len).sum()
        };
        blocks + self.text_height(i, split)
    }

    /// Screen rows the text of row `i` takes, without the comments above it.
    pub(super) fn text_height(&self, i: usize, split: bool) -> usize {
        let v = self.view.get();
        if !self.wrap {
            return 1;
        }
        if split {
            return match self.aligned.rows.get(i) {
                Some(Row::Line { left, right, .. }) => {
                    let side = |s: &Option<Side>| {
                        s.as_ref().map_or(1, |s| {
                            let eol = if s.no_eol { width(NO_EOL) } else { 0 };
                            wrapped_rows(width(&s.text) + eol, v.col_w)
                        })
                    };
                    side(left).max(side(right))
                }
                _ => 1,
            };
        }
        self.lines.get(i).map_or(1, |l| {
            wrapped_rows(width(&l.text).saturating_sub(1), v.col_w)
        })
    }
}
