//! Components and items as styled lines.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use suite_protocol as pb;

use crate::text::{
    clean, clip, hard_wrap, line_width, one_line, push_text, sline, truncate, truncate_spans,
    width, wrap,
};
use crate::{Options, Palette};

pub(crate) type Lines = Vec<Line<'static>>;

/// What one render needs: the options and the running count of buttons drawn so far (the
/// n-th button drawn is `buttons(item)[n]`, so the focus and the default keys line up).
pub(crate) struct Cx<'a> {
    pub(crate) o: &'a Options,
    pub(crate) p: Palette,
    pub(crate) next_button: usize,
    /// Lines of diffs and code still allowed in the item being drawn (None: no limit).
    budget: Option<usize>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(o: &'a Options) -> Self {
        Cx {
            o,
            p: o.theme,
            next_button: 0,
            budget: None,
        }
    }

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
        clip(&mut lines, w);
        lines
    }

    /// `> text`, the app it came from dimmed after the text.
    fn user(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
        let inner = w.saturating_sub(2).max(1);
        let mut body = self.comp(&it.component, inner);
        if let Some(label) = label {
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
        let lead = vec![
            Span::styled(
                ">",
                Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ];
        if body.is_empty() {
            return vec![Line::from(lead[0].clone())];
        }
        prefixed(body, lead, vec![Span::raw("  ")])
    }

    /// The text; a guest agent's gets a small header with its name.
    fn agent(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
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

    fn thought(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
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

    fn notice(&mut self, it: &pb::ViewItem, w: usize) -> Lines {
        let mut body = self.comp(&it.component, w);
        if it.state == Some(pb::ItemState::Failed) {
            let color = self.p.danger;
            restyle(&mut body, |s| s.fg(color));
        } else {
            tint(&mut body, self.p.muted);
        }
        body
    }

    /// A header line (`● label`, the bullet colored by state), then the result: summarized
    /// under `⎿`, or in full when expanded. Items with buttons are always shown in full:
    /// the buttons are what the item is for.
    fn tool(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
        let color = match it.state {
            Some(pb::ItemState::Running) => self.p.accent,
            Some(pb::ItemState::Done) => self.p.success,
            Some(pb::ItemState::Failed) => self.p.danger,
            Some(pb::ItemState::Pending) | None => self.p.muted,
        };
        let bullet = vec![Span::styled("● ", Style::new().fg(color))];
        let mut out = match label {
            Some(label) => {
                let bold = Span::styled(label, Style::new().add_modifier(Modifier::BOLD));
                let head = wrap(&[bold], w.saturating_sub(2).max(1));
                prefixed(head, bullet, vec![Span::raw("  ")])
            }
            None => vec![Line::from(bullet)],
        };
        let prompt = has_buttons(&it.component);
        if self.o.expanded || prompt {
            self.budget = self.o.preview_lines.filter(|_| prompt && !self.o.expanded);
            let body = self.comp(&it.component, w.saturating_sub(2).max(1));
            out.extend(indented(body, 2));
        } else {
            let summary = self.summary(&it.component, w.saturating_sub(4).max(1));
            let elbow = vec![Span::styled("  ⎿ ", self.p.muted_style())];
            out.extend(prefixed(summary, elbow, vec![Span::raw("    ")]));
        }
        out
    }

    // ---- components ----

    pub(crate) fn comp(&mut self, c: &pb::Component, w: usize) -> Lines {
        let w = w.max(1);
        match c {
            pb::Component::CardComponent(v) => self.card(v, w),
            pb::Component::RowComponent(v) => self.row(v, w),
            pb::Component::StackComponent(v) => self.stack(v, w),
            pb::Component::TextComponent(v) => self.text(v, w),
            pb::Component::ImageComponent(v) => self.image(v, w),
            pb::Component::FileComponent(v) => self.file(v, w),
            pb::Component::CodeComponent(v) => {
                let lines = self.code(&v.code, w);
                self.preview(lines)
            }
            pb::Component::DiffComponent(v) => {
                let lines = self.diff(v, w);
                self.preview(lines)
            }
            pb::Component::TableComponent(v) => self.table(v, w),
            pb::Component::KvComponent(v) => self.kv(v, w),
            pb::Component::ProgressComponent(v) => self.progress(v, w),
            pb::Component::ButtonComponent(v) => vec![self.button(v, w)],
            pb::Component::LinkComponent(v) => {
                wrap(&link(&v.url, v.text.as_deref(), Style::new(), &self.p), w)
            }
            pb::Component::Unknown(v) => {
                let name = v.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                vec![self.placeholder(&format!("[unsupported: {}]", one_line(name)), w)]
            }
        }
    }

    fn placeholder(&self, text: &str, w: usize) -> Line<'static> {
        sline(truncate(text, w), self.p.muted_style())
    }

    fn children(&mut self, cs: &[pb::Component], w: usize) -> Vec<Lines> {
        cs.iter()
            .map(|c| self.comp(c, w))
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Children top to bottom. Buttons that follow content (a diff to approve) get a blank line
    /// above them, so they read as the question and not as part of the diff.
    fn stack(&mut self, v: &pb::StackComponent, w: usize) -> Lines {
        let mut parts: Vec<Lines> = Vec::new();
        let mut content_above = false;
        for c in &v.children {
            let lines = self.comp(c, w);
            if lines.is_empty() {
                continue;
            }
            if has_buttons(c) {
                if content_above && v.gap != Some(pb::Space::L) {
                    parts.push(vec![Line::default()]);
                }
                content_above = false;
            } else {
                content_above = true;
            }
            parts.push(lines);
        }
        stack(parts, v.gap)
    }

    /// Cuts a diff or code block to what is left of the item's line budget.
    fn preview(&mut self, mut lines: Lines) -> Lines {
        let Some(left) = self.budget else {
            return lines;
        };
        // A hint takes a line itself: cutting a single line would save nothing.
        if lines.len() <= left + 1 {
            self.budget = Some(left.saturating_sub(lines.len()));
            return lines;
        }
        let hidden = lines.len() - left;
        lines.truncate(left);
        lines.push(sline(
            format!("… {hidden} more lines (expand to see all)"),
            self.p.muted_style(),
        ));
        self.budget = Some(0);
        lines
    }

    /// Side by side when every child is a single line and they all fit, else stacked.
    fn row(&mut self, v: &pb::RowComponent, w: usize) -> Lines {
        let parts = self.children(&v.children, w);
        let sep = match v.gap {
            None | Some(pb::Space::S) => 2,
            Some(pb::Space::M) => 3,
            Some(pb::Space::L) => 4,
        };
        let single = parts.iter().all(|p| p.len() == 1);
        let total: usize =
            parts.iter().map(|p| p[0].width()).sum::<usize>() + sep * parts.len().saturating_sub(1);
        if !single || parts.len() < 2 || total > w {
            return stack(parts, v.gap);
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            if i > 0 {
                push_text(&mut spans, &" ".repeat(sep), Style::new());
            }
            if let Some(line) = part.into_iter().next() {
                spans.extend(line.spans);
            }
        }
        vec![Line::from(spans)]
    }

    /// A gutter along the children, with the title on top. The tone colors both.
    fn card(&mut self, v: &pb::CardComponent, w: usize) -> Lines {
        let tone = v.tone.map(|t| self.p.tone(t));
        let title = v.title.as_deref().map(one_line).filter(|t| !t.is_empty());
        let inner = if w < 6 { w } else { w - 2 };
        let body = stack(self.children(&v.children, inner), None);
        if w < 6 {
            let mut out = Vec::new();
            if let Some(t) = title {
                out.push(sline(
                    truncate(&t, w),
                    Style::new().add_modifier(Modifier::BOLD),
                ));
            }
            out.extend(body);
            return out;
        }
        let bar = Style::new().fg(tone.unwrap_or(self.p.border));
        let mut out = Vec::new();
        if let Some(t) = title {
            let mut title_style = Style::new().add_modifier(Modifier::BOLD);
            if let Some(c) = tone {
                title_style = title_style.fg(c);
            }
            out.push(Line::from(vec![
                Span::styled("╭ ", bar),
                Span::styled(truncate(&t, w - 2), title_style),
            ]));
        }
        for line in body {
            if line.spans.is_empty() {
                out.push(sline("│", bar));
            } else {
                let mut spans = vec![Span::styled("│ ", bar)];
                spans.extend(line.spans);
                out.push(Line::from(spans));
            }
        }
        out
    }

    pub(crate) fn text(&self, v: &pb::TextComponent, w: usize) -> Lines {
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
    pub(crate) fn blocks(&self, bs: &[pb::Block], w: usize, loose: bool) -> Lines {
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

    fn block(&self, b: &pb::Block, w: usize) -> Lines {
        match b {
            pb::Block::ParagraphBlock(v) => wrap(&self.spans(&v.spans), w),
            pb::Block::HeadingBlock(v) => {
                let mut spans = self.spans(&v.spans);
                for s in &mut spans {
                    s.style = s.style.add_modifier(Modifier::BOLD);
                }
                wrap(&spans, w)
            }
            pb::Block::ListBlock(v) => self.list(v, w),
            pb::Block::CodeBlock(v) => self.code(&v.code, w),
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
            pb::Block::RuleBlock(_) => vec![sline("─".repeat(w), Style::new().fg(self.p.border))],
            pb::Block::Unknown(_) => vec![self.placeholder("[unsupported block]", w)],
        }
    }

    fn spans(&self, spans: &[pb::Span]) -> Vec<Span<'static>> {
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
    fn list(&self, v: &pb::ListBlock, w: usize) -> Lines {
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
    fn code(&self, code: &str, w: usize) -> Lines {
        let st = self.p.code_style();
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

    fn image(&self, v: &pb::ImageComponent, w: usize) -> Lines {
        let name = v
            .alt
            .as_deref()
            .map(one_line)
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| one_line(&v.src));
        let mut s = format!("[image {name}");
        if let (Some(iw), Some(ih)) = (v.width, v.height) {
            s.push_str(&format!(" · {iw}×{ih}"));
        }
        s.push(']');
        let mut out = wrap(&[Span::styled(s, self.p.muted_style())], w);
        if let Some(cap) = v.caption.as_deref().map(clean).filter(|c| !c.is_empty()) {
            out.extend(wrap(&[Span::raw(cap.into_owned())], w));
        }
        out
    }

    fn file(&self, v: &pb::FileComponent, w: usize) -> Lines {
        let path = one_line(&v.path);
        let mut spans = vec![Span::raw("📄 ")];
        match v
            .name
            .as_deref()
            .map(one_line)
            .filter(|n| !n.is_empty() && *n != path)
        {
            Some(name) => {
                spans.push(Span::raw(name));
                spans.push(Span::styled(format!(" ({path})"), self.p.muted_style()));
            }
            None => spans.push(Span::raw(path)),
        }
        wrap(&spans, w)
    }

    fn diff(&self, v: &pb::DiffComponent, w: usize) -> Lines {
        let mut out = Vec::new();
        if let Some(path) = v.path.as_deref().map(one_line).filter(|p| !p.is_empty()) {
            for l in hard_wrap(&path, w) {
                out.push(sline(l, Style::new().add_modifier(Modifier::BOLD)));
            }
        }
        for dl in &v.lines {
            let (mark, st) = match dl.kind {
                pb::DiffLineKind::Add => ("+", Style::new().fg(self.p.success)),
                pb::DiffLineKind::Remove => ("-", Style::new().fg(self.p.danger)),
                pb::DiffLineKind::Hunk => ("", self.p.muted_style()),
                pb::DiffLineKind::Context => (" ", Style::new()),
            };
            let text = clean(&dl.text).replace('\n', " ");
            let mut body = hard_wrap(&text, w.saturating_sub(mark.len()).max(1));
            if body.is_empty() {
                body.push(String::new());
            }
            for (i, l) in body.into_iter().enumerate() {
                let lead = if i == 0 {
                    mark
                } else {
                    &" ".repeat(mark.len())
                };
                out.push(Line::from(vec![Span::styled(format!("{lead}{l}"), st)]));
            }
        }
        out
    }

    fn kv(&self, v: &pb::KvComponent, w: usize) -> Lines {
        let keys: Vec<String> = v.items.iter().map(|i| one_line(&i.key)).collect();
        let key_w = keys
            .iter()
            .map(|k| width(k))
            .max()
            .unwrap_or(0)
            .min((w / 2).max(1));
        let mut out = Vec::new();
        for (key, item) in keys.iter().zip(&v.items) {
            let key = truncate(key, key_w);
            let head = vec![
                Span::styled(key.clone(), self.p.muted_style()),
                Span::raw(" ".repeat(key_w - width(&key) + 2)),
            ];
            let value = clean(&item.value).into_owned();
            let mut body = wrap(&[Span::raw(value)], w.saturating_sub(key_w + 2).max(1));
            if body.is_empty() {
                body.push(Line::default());
            }
            out.extend(prefixed(body, head, vec![Span::raw(" ".repeat(key_w + 2))]));
        }
        out
    }

    /// A grid with a header row. Columns that don't fit are cut (cells end in `…`), the widest
    /// first, so short columns stay whole.
    fn table(&self, v: &pb::TableComponent, w: usize) -> Lines {
        let cols = v
            .rows
            .iter()
            .map(Vec::len)
            .chain([v.columns.len()])
            .max()
            .unwrap_or(0);
        if cols == 0 {
            return Vec::new();
        }
        let cell = |row: &[String], c: usize| row.get(c).map(|s| one_line(s)).unwrap_or_default();
        let mut widths: Vec<usize> = (0..cols)
            .map(|c| {
                v.rows
                    .iter()
                    .map(|r| width(&cell(r, c)))
                    .chain([width(&cell(&v.columns, c))])
                    .max()
                    .unwrap_or(1)
                    .clamp(1, w)
            })
            .collect();
        let avail = w.saturating_sub(3 * (cols - 1));
        let mut total: usize = widths.iter().sum();
        while total > avail {
            let max = widths.iter().copied().max().unwrap_or(1);
            if max <= 1 {
                break;
            }
            // Cut the widest columns down towards the next widest in one go.
            let next = widths
                .iter()
                .copied()
                .filter(|w| *w < max)
                .max()
                .unwrap_or(1);
            let ties = widths.iter().filter(|w| **w == max).count();
            let cut = (max - next).min((total - avail).div_ceil(ties)).max(1);
            for w in widths.iter_mut().filter(|w| **w == max) {
                *w -= cut.min(*w - 1);
            }
            total = widths.iter().sum();
        }

        let border = Style::new().fg(self.p.border);
        let row_line = |row: &[String], st: Style| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, cw) in widths.iter().enumerate() {
                if c > 0 {
                    spans.push(Span::styled(" │ ", border));
                }
                let mut txt = truncate(&cell(row, c), *cw);
                if c + 1 < cols {
                    txt.push_str(&" ".repeat(cw - width(&txt)));
                }
                spans.push(Span::styled(txt, st));
            }
            Line::from(spans)
        };
        let rule = sline(
            widths
                .iter()
                .map(|cw| "─".repeat(*cw))
                .collect::<Vec<_>>()
                .join("─┼─"),
            border,
        );
        let mut out = vec![
            row_line(&v.columns, Style::new().add_modifier(Modifier::BOLD)),
            rule,
        ];
        out.extend(v.rows.iter().map(|r| row_line(r, Style::new())));
        clip(&mut out, w);
        out
    }

    /// `▓▓▓░░ 60% label` with a bar of up to 20 cells.
    fn progress(&self, v: &pb::ProgressComponent, w: usize) -> Lines {
        let value = if v.value.is_nan() {
            0.0
        } else {
            v.value.clamp(0.0, 1.0)
        };
        let percent = format!(" {}%", (value * 100.0).round() as u32);
        let cells = 20.min(w.saturating_sub(percent.len()).max(1));
        let filled = (value * cells as f64).round() as usize;
        let mut spans = vec![
            Span::styled("▓".repeat(filled), Style::new().fg(self.p.accent)),
            Span::styled("░".repeat(cells - filled), self.p.muted_style()),
            Span::raw(percent),
        ];
        let label = v.label.as_deref().map(one_line).filter(|l| !l.is_empty());
        let Some(label) = label else {
            return vec![Line::from(spans)];
        };
        if line_width(&spans) + 1 + width(&label) <= w {
            spans.push(Span::styled(format!(" {label}"), self.p.muted_style()));
            return vec![Line::from(spans)];
        }
        let mut out = vec![Line::from(spans)];
        out.extend(wrap(&[Span::styled(label, self.p.muted_style())], w));
        out
    }

    /// `[k Label]`; k is the key `buttons` reports. The focused one is reversed.
    fn button(&mut self, b: &pb::ButtonComponent, w: usize) -> Line<'static> {
        let idx = self.next_button;
        self.next_button += 1;
        let key = button_key(b, idx);
        let focused = self.o.focused_button == Some(idx);
        let mut spans = vec![
            Span::raw("["),
            Span::styled(
                one_line(&key),
                Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {}]", one_line(&b.label))),
        ];
        if focused {
            for s in &mut spans {
                s.style = s.style.add_modifier(Modifier::REVERSED);
            }
        }
        Line::from(truncate_spans(spans, w))
    }
}

/// The key that triggers a button: its own, else its 1-based position among the item's buttons.
pub(crate) fn button_key(b: &pb::ButtonComponent, index: usize) -> String {
    match b.key.as_deref() {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => (index + 1).to_string(),
    }
}

pub(crate) fn has_buttons(c: &pb::Component) -> bool {
    match c {
        pb::Component::ButtonComponent(_) => true,
        pb::Component::CardComponent(v) => v.children.iter().any(has_buttons),
        pb::Component::RowComponent(v) => v.children.iter().any(has_buttons),
        pb::Component::StackComponent(v) => v.children.iter().any(has_buttons),
        _ => false,
    }
}

/// Underlined text, followed by the URL in muted parentheses unless that is the text.
pub(crate) fn link(url: &str, text: Option<&str>, base: Style, p: &Palette) -> Vec<Span<'static>> {
    let url = one_line(url);
    let st = base.fg(p.accent).add_modifier(Modifier::UNDERLINED);
    match text.map(one_line).filter(|t| !t.is_empty() && *t != url) {
        Some(t) => vec![
            Span::styled(t, st),
            Span::styled(format!(" ({url})"), p.muted_style()),
        ],
        None => vec![Span::styled(url, st)],
    }
}

/// Children top to bottom; a gap of `l` is one blank line (s and m: none, as in the TUI).
pub(crate) fn stack(parts: Vec<Lines>, gap: Option<pb::Space>) -> Lines {
    let mut out: Lines = Vec::new();
    for part in parts {
        if !out.is_empty() && gap == Some(pb::Space::L) {
            out.push(Line::default());
        }
        out.extend(part);
    }
    out
}

/// `first` before the first line, `rest` before the others. Empty lines stay empty.
pub(crate) fn prefixed(lines: Lines, first: Vec<Span<'static>>, rest: Vec<Span<'static>>) -> Lines {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let lead = if i == 0 { &first } else { &rest };
            if line.spans.is_empty() && i > 0 {
                return line;
            }
            let mut spans = lead.clone();
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

pub(crate) fn indented(lines: Lines, n: usize) -> Lines {
    let pad = vec![Span::raw(" ".repeat(n))];
    prefixed(lines, pad.clone(), pad)
}

/// Gives every span without a foreground color this one.
pub(crate) fn tint(lines: &mut Lines, color: Color) {
    restyle_spans(lines, |s| {
        if s.fg.is_none() {
            s.fg = Some(color);
        }
    });
}

pub(crate) fn restyle(lines: &mut Lines, f: impl Fn(Style) -> Style) {
    restyle_spans(lines, |s| *s = f(*s));
}

fn restyle_spans(lines: &mut Lines, mut f: impl FnMut(&mut Style)) {
    for line in lines {
        for span in &mut line.spans {
            f(&mut span.style);
        }
    }
}
