use super::*;

impl Cx<'_> {
    /// A colored diff under its path: unified, or side by side when the app asks for it
    /// ([`Options::split_diffs`]) and the width allows.
    pub(crate) fn diff(&self, v: &pb::DiffComponent, w: usize) -> Lines {
        let mut out = self.diff_title(v, w);
        out.extend(self.diff_rows(v.path.as_deref(), &v.lines, w));
        out
    }

    /// The diff's path, bold, with its icon.
    pub(super) fn diff_title(&self, v: &pb::DiffComponent, w: usize) -> Lines {
        let mut out = Vec::new();
        let path = v
            .path
            .as_deref()
            .map(|p| one_line(crate::relative_path(p, self.o.cwd.as_deref())))
            .filter(|p| !p.is_empty());
        if let Some(path) = &path {
            let icon = self.o.file_icons && w > 2;
            for (i, l) in hard_wrap(path, if icon { w - 2 } else { w })
                .into_iter()
                .enumerate()
            {
                let mut line = sline(l, Style::new().add_modifier(Modifier::BOLD));
                if icon {
                    let lead = match i {
                        0 => crate::file_icon(path, &self.p),
                        _ => Span::raw(" "),
                    };
                    line.spans.splice(0..0, [lead, Span::raw(" ")]);
                }
                out.push(line);
            }
        }
        out
    }

    /// A diff line's mark, its style, and whether it is tinted (a change).
    pub(super) fn diff_style(&self, kind: pb::DiffLineKind) -> (&'static str, Style, bool) {
        let (mark, fg, bg) = match kind {
            pb::DiffLineKind::Add => ("+", self.p.success, blend(self.p.bg, self.p.success, 16)),
            pb::DiffLineKind::Remove => ("-", self.p.danger, blend(self.p.bg, self.p.danger, 16)),
            pb::DiffLineKind::Hunk => ("", self.p.muted, None),
            pb::DiffLineKind::Context => (" ", Color::Reset, None),
        };
        let st = match (kind, bg) {
            (pb::DiffLineKind::Context, _) => Style::new(),
            (_, Some(bg)) => Style::new().fg(fg).bg(bg),
            (_, None) => Style::new().fg(fg),
        };
        (mark, st, bg.is_some())
    }

    /// Highlighters for the old and new side of a diff of `path`, when it is highlighted.
    pub(super) fn diff_highlighters(
        &self,
        path: Option<&str>,
        n: usize,
    ) -> Option<(Highlighter, Highlighter)> {
        let colors = code_colors(&self.p);
        path.filter(|_| !self.o.plain_code && colors.is_some() && n <= syntax::MAX_LINES)
            .and_then(syntax_for)
            .map(|s| (Highlighter::new(s), Highlighter::new(s)))
    }

    /// A diff line's text as spans: highlighted by `classes` (one per char) over `st`.
    pub(super) fn diff_spans(
        &self,
        text: String,
        classes: Vec<syntax::Class>,
        st: Style,
    ) -> Vec<Span<'static>> {
        let Some(colors) = code_colors(&self.p).filter(|_| !classes.is_empty()) else {
            return vec![Span::styled(text, st)];
        };
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (c, class) in text.chars().zip(classes) {
            let style = st.patch(colors[class as usize]);
            match spans.last_mut() {
                Some(Span {
                    content,
                    style: last,
                }) if *last == style => content.to_mut().push(c),
                _ => spans.push(Span::styled(c.to_string(), style)),
            }
        }
        spans
    }

    /// The lines of a diff: side by side when the app asks and both sides get room, else
    /// unified.
    pub(crate) fn diff_rows(&self, path: Option<&str>, lines: &[pb::DiffLine], w: usize) -> Lines {
        if self.o.split_diffs && w >= SPLIT_MIN && split_fits(lines, w) {
            return self.split_rows(path, lines, w);
        }
        self.unified_rows(path, lines, w)
    }

    /// Unified: one line after the other. With true colors the code is highlighted by the
    /// grammar for the path (old and new lines parsed apart, each hunk starting over) and changed
    /// lines are tinted across the width; without, a changed line is just green or red.
    pub(super) fn unified_rows(
        &self,
        path: Option<&str>,
        lines: &[pb::DiffLine],
        w: usize,
    ) -> Lines {
        let mut out = Vec::new();
        let mut hl = self.diff_highlighters(path, lines.len());
        for dl in lines {
            let (mark, st, tinted) = self.diff_style(dl.kind);
            let bg = tinted.then_some(());
            let text = clean(&dl.text).replace('\n', " ");
            let classes = match (dl.kind, hl.as_mut()) {
                (pb::DiffLineKind::Hunk, Some((old, new))) => {
                    old.reset();
                    new.reset();
                    Vec::new()
                }
                (pb::DiffLineKind::Context, Some((old, new))) => {
                    old.line(&text);
                    new.line(&text)
                }
                (pb::DiffLineKind::Add, Some((_, new))) => new.line(&text),
                (pb::DiffLineKind::Remove, Some((old, _))) => old.line(&text),
                _ => Vec::new(),
            };
            let spans = self.diff_spans(text, classes, st);
            let indent = " ".repeat(mark.len());
            for (i, row) in hard_wrap_spans(&spans, w.saturating_sub(mark.len()).max(1))
                .into_iter()
                .enumerate()
            {
                let lead = if i == 0 { mark } else { &indent };
                let mut line = vec![Span::styled(lead.to_string(), st)];
                line.extend(row);
                // A changed line's tint runs to the edge.
                let used = line_width(&line);
                if bg.is_some() && used < w {
                    line.push(Span::styled(" ".repeat(w - used), st));
                }
                out.push(Line::from(line));
            }
        }
        out
    }

    /// Side by side: the old lines on the left, the new on the right, a removal next to the
    /// addition that replaced it, context on both sides and a hunk line across. Each side wraps
    /// in its own half; the shorter one goes on blank.
    pub(super) fn split_rows(&self, path: Option<&str>, lines: &[pb::DiffLine], w: usize) -> Lines {
        use pb::DiffLineKind::{Add, Context, Hunk, Remove};
        let left_w = (w - 1) / 2;
        let right_w = w - 1 - left_w;
        let mut hl = self.diff_highlighters(path, lines.len());
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let dl = &lines[i];
            match dl.kind {
                Hunk => {
                    if let Some((old, new)) = hl.as_mut() {
                        old.reset();
                        new.reset();
                    }
                    let text = truncate(&one_line(&dl.text), w);
                    out.push(sline(text, self.p.muted_style()));
                    i += 1;
                }
                Context => {
                    let text = clean(&dl.text).replace('\n', " ");
                    let (old, new) = match hl.as_mut() {
                        Some((old, new)) => (old.line(&text), new.line(&text)),
                        None => (Vec::new(), Vec::new()),
                    };
                    let left = self.side(Some((text.clone(), old, Context)), left_w);
                    let right = self.side(Some((text, new, Context)), right_w);
                    out.extend(self.beside(left, right, left_w, right_w));
                    i += 1;
                }
                Remove | Add => {
                    let dels: Vec<&pb::DiffLine> =
                        lines[i..].iter().take_while(|l| l.kind == Remove).collect();
                    let adds: Vec<&pb::DiffLine> = lines[i + dels.len()..]
                        .iter()
                        .take_while(|l| l.kind == Add)
                        .collect();
                    let mut side_of = |l: Option<&&pb::DiffLine>, old_side: bool| {
                        l.map(|l| {
                            let text = clean(&l.text).replace('\n', " ");
                            let classes = match hl.as_mut() {
                                Some((old, _)) if old_side => old.line(&text),
                                Some((_, new)) => new.line(&text),
                                None => Vec::new(),
                            };
                            (text, classes, l.kind)
                        })
                    };
                    for k in 0..dels.len().max(adds.len()) {
                        let left = side_of(dels.get(k), true);
                        let right = side_of(adds.get(k), false);
                        let left = self.side(left, left_w);
                        let right = self.side(right, right_w);
                        out.extend(self.beside(left, right, left_w, right_w));
                    }
                    i += dels.len() + adds.len();
                }
            }
        }
        out
    }

    /// One side of a split row: the mark and the text wrapped in `col` cells, a change's tint
    /// to its edge. None: no line on this side.
    pub(super) fn side(
        &self,
        line: Option<(String, Vec<syntax::Class>, pb::DiffLineKind)>,
        col: usize,
    ) -> Vec<Vec<Span<'static>>> {
        let Some((text, classes, kind)) = line else {
            return Vec::new();
        };
        let (mark, st, tinted) = self.diff_style(kind);
        let spans = self.diff_spans(text, classes, st);
        let indent = " ".repeat(mark.len());
        hard_wrap_spans(&spans, col.saturating_sub(mark.len()).max(1))
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let lead = if i == 0 { mark } else { &indent };
                let mut cells = vec![Span::styled(lead.to_string(), st)];
                cells.extend(row);
                let used = line_width(&cells);
                if used < col {
                    let fill = if tinted { st } else { Style::new() };
                    cells.push(Span::styled(" ".repeat(col - used), fill));
                }
                cells
            })
            .collect()
    }

    /// Two sides next to each other, a rule between them.
    pub(super) fn beside(
        &self,
        left: Vec<Vec<Span<'static>>>,
        right: Vec<Vec<Span<'static>>>,
        left_w: usize,
        right_w: usize,
    ) -> Lines {
        let rule = Style::new().fg(self.p.border);
        let rows = left.len().max(right.len()).max(1);
        let mut left = left.into_iter();
        let mut right = right.into_iter();
        (0..rows)
            .map(|_| {
                let mut spans = left
                    .next()
                    .unwrap_or_else(|| vec![Span::raw(" ".repeat(left_w))]);
                spans.push(Span::styled("│", rule));
                spans.extend(
                    right
                        .next()
                        .unwrap_or_else(|| vec![Span::raw(" ".repeat(right_w))]),
                );
                Line::from(spans)
            })
            .collect()
    }
}
