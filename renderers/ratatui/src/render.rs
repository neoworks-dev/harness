//! Components and items as styled lines.

use neo_protocol as pb;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::palette::blend;
use crate::syntax::{self, Highlighter, code_colors, syntax_for};
use crate::text::{
    clean, clip, hard_wrap, hard_wrap_spans, line_width, one_line, push_text, sline, truncate,
    truncate_spans, width, wrap,
};
use crate::{ImageOptions, ImageSlot, ImageStatus, Options, Palette, filled_cells, image_cells};

// The rest of `impl Cx`: items, text blocks, pictures, diffs.
mod blocks;
mod diffs;
mod items;
mod pictures;

/// Marks the first line reserved for an image until `item` has found its place. Zero width, so
/// padding and clipping ignore it; it is removed before the lines leave the renderer.
const SLOT_MARK: &str = "\u{2060}";
/// Pictures side by side: at most this many a row, each at least this many cells wide, this
/// many cells apart.
const PICTURES_PER_ROW: usize = 3;
const MIN_PICTURE: usize = 16;
const GAP: usize = 2;

/// Rows the label of a finished tool call takes at most, and its command; unless expanded.
const LABEL_ROWS: usize = 2;
/// Rows the command of a running tool call takes at most, unless expanded or asked about.
const RUNNING_ROWS: usize = 4;
/// A diff is side by side from this width on (each side then has some 40 cells).
const SPLIT_MIN: usize = 81;
/// An open edit's lines around each change, and the rows its diff takes at most (unless
/// expanded): a whole new file is cut there.
const EDIT_CONTEXT: usize = 5;
const EDIT_ROWS: usize = 40;

pub(crate) type Lines = Vec<Line<'static>>;

/// Whether a diff reads side by side in `w` cells: no more than a fifth of its changed lines
/// would wrap in a side (half the width, less the `+`/`-` before each line). Otherwise code
/// breaks mid-word on both sides, and unified reads better.
fn split_fits(lines: &[pb::DiffLine], w: usize) -> bool {
    use pb::DiffLineKind::{Add, Remove};
    let room = (w.saturating_sub(1) / 2).saturating_sub(1);
    let changed: Vec<&pb::DiffLine> = lines
        .iter()
        .filter(|l| matches!(l.kind, Add | Remove))
        .collect();
    let wrapping = changed
        .iter()
        .filter(|l| width(&clean(&l.text)) > room)
        .count();
    wrapping * 5 <= changed.len()
}

/// What one render needs: the options and the running count of buttons drawn so far (the
/// n-th button drawn is `buttons(item)[n]`, so the focus and the default keys line up).
pub(crate) struct Cx<'a> {
    pub(crate) o: &'a Options,
    pub(crate) p: Palette,
    pub(crate) next_button: usize,
    /// Lines of diffs and code still allowed in the item being drawn (None: no limit).
    budget: Option<usize>,
    /// Diffs are left out: drawn already, as the edit they will be (see `tool`).
    skip_diffs: bool,
    /// Rooms reserved for images so far, top to bottom: (cols, rows, src).
    rooms: Vec<(u16, u16, String)>,
    /// Where those rooms ended up, once the item is laid out.
    slots: Vec<ImageSlot>,
    /// What the neoui docs drawn so far noted (targets, the focused line).
    pub(crate) ui: crate::neoui::Run,
    /// What the focused option says it does, for its prompt's title (see `row`).
    focused_detail: Option<Lines>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(o: &'a Options) -> Self {
        Cx {
            o,
            p: o.theme,
            next_button: 0,
            budget: None,
            skip_diffs: false,
            rooms: Vec::new(),
            slots: Vec::new(),
            ui: Default::default(),
            focused_detail: None,
        }
    }

    pub(crate) fn take_slots(&mut self) -> Vec<ImageSlot> {
        std::mem::take(&mut self.slots)
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
                let lines = self.code(&v.code, v.language.as_deref(), true, w);
                self.preview(lines)
            }
            pb::Component::DiffComponent(_) if self.skip_diffs => Vec::new(),
            pb::Component::DiffComponent(v) => {
                let lines = self.diff(v, w);
                self.preview(lines)
            }
            pb::Component::TableComponent(v) => self.table(v, w),
            pb::Component::KvComponent(v) => self.kv(v, w),
            pb::Component::ProgressComponent(v) => self.progress(v, w),
            pb::Component::ButtonComponent(v) => vec![self.button(v, w)],
            pb::Component::NeoUiComponent(v) => self.neoui_doc(&v.ui, w),
            pb::Component::LinkComponent(v) => {
                wrap(&link(&v.url, v.text.as_deref(), Style::new(), &self.p), w)
            }
            pb::Component::Unknown(v) => {
                let name = v.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                vec![self.placeholder(&format!("[unsupported: {}]", one_line(name)), w)]
            }
        }
    }

    pub(crate) fn placeholder(&self, text: &str, w: usize) -> Line<'static> {
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
        let mut i = 0;
        while i < v.children.len() {
            // Pictures one after the other share the width.
            let run = self.ready_pictures(&v.children[i..]);
            if run >= 2 {
                let pictures: Vec<&pb::ImageComponent> = v.children[i..i + run]
                    .iter()
                    .filter_map(|c| match c {
                        pb::Component::ImageComponent(p) => Some(p),
                        _ => None,
                    })
                    .collect();
                parts.push(self.pictures(&pictures, w));
                content_above = true;
                i += run;
                continue;
            }
            let c = &v.children[i];
            i += 1;
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
                // Diffs one after another (a patch of several files) part by a blank row, so
                // each file's reads as its own.
                if content_above
                    && matches!(c, pb::Component::DiffComponent(_))
                    && v.gap != Some(pb::Space::L)
                {
                    parts.push(vec![Line::default()]);
                }
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
        let hint = if self.o.shown_above {
            format!("… {hidden} more lines, in full above")
        } else {
            format!("… {hidden} more lines (expand to see all)")
        };
        lines.push(sline(hint, self.p.muted_style()));
        self.budget = Some(0);
        lines
    }

    /// Side by side when every child is a single line and they all fit, else stacked.
    fn row(&mut self, v: &pb::RowComponent, w: usize) -> Lines {
        // An option with its description, while the keys go through the options: only the
        // focused one says what it does, by the prompt's title, so the options stay put.
        if let (Some(focused), [pb::Component::ButtonComponent(b), rest @ ..]) =
            (self.o.focused_button, v.children.as_slice())
            && !rest.is_empty()
            && rest
                .iter()
                .all(|c| matches!(c, pb::Component::TextComponent(_)))
        {
            if self.next_button == focused {
                self.focused_detail = Some(stack(self.children(rest, w), v.gap));
            }
            return vec![self.button(b, w)];
        }
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
        let body = stack(self.children(&v.children, card_inner(w)), None);
        self.card_frame(v.title.as_deref(), v.tone, body, w)
    }

    /// The body inside the card's gutter (`card_inner` wide), the title on top.
    pub(crate) fn card_frame(
        &self,
        title: Option<&str>,
        tone: Option<pb::Tone>,
        body: Lines,
        w: usize,
    ) -> Lines {
        let tone = tone.map(|t| self.p.tone(t));
        let title = title.map(one_line).filter(|t| !t.is_empty());
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

    /// A kv value with a tone is highlighted: its color, bold.
    pub(crate) fn value_style(&self, item: &pb::KvItem) -> Style {
        match item.tone {
            Some(tone) => Style::new()
                .fg(self.p.tone(tone))
                .add_modifier(Modifier::BOLD),
            None => Style::new(),
        }
    }

    pub(crate) fn kv(&self, v: &pb::KvComponent, w: usize) -> Lines {
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
            let mut body = wrap(
                &[Span::styled(value, self.value_style(item))],
                w.saturating_sub(key_w + 2).max(1),
            );
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
        fit_columns(&mut widths, w.saturating_sub(3 * (cols - 1)));

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
    pub(crate) fn progress(&self, v: &pb::ProgressComponent, w: usize) -> Lines {
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
        let mut spans = if self.o.hide_keys {
            vec![Span::raw(format!("[{}]", one_line(&b.label)))]
        } else {
            vec![
                Span::raw("["),
                Span::styled(
                    one_line(&key),
                    Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}]", one_line(&b.label))),
            ]
        };
        if b.current == Some(true) {
            spans.push(Span::styled(" ✓", Style::new().fg(self.p.success)));
        }
        if focused {
            for s in &mut spans {
                s.style = s.style.add_modifier(Modifier::REVERSED);
            }
        }
        Line::from(truncate_spans(spans, w))
    }
}

/// What a card's children get of its width: the gutter takes two cells.
pub(crate) fn card_inner(w: usize) -> usize {
    if w < 6 { w } else { w - 2 }
}

/// The last part of a path (the whole of anything else).
pub(crate) fn file_name(src: &str) -> String {
    match src.rsplit_once('/') {
        Some((_, name)) if !name.is_empty() && !src.contains("://") => one_line(name),
        _ => one_line(src),
    }
}

/// Narrows the widest columns until they fit `avail` cells together (each keeps one).
pub(crate) fn fit_columns(widths: &mut [usize], avail: usize) {
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
}

/// The sources of the images that are drawn in full: not inside a table or the like. Images of
/// neoui docs count too, also those in a text's blocks.
pub(crate) fn sources(c: &pb::Component, out: &mut Vec<String>) {
    match c {
        pb::Component::ImageComponent(v) => out.push(v.src.clone()),
        pb::Component::CardComponent(v) => v.children.iter().for_each(|c| sources(c, out)),
        pb::Component::RowComponent(v) => v.children.iter().for_each(|c| sources(c, out)),
        pb::Component::StackComponent(v) => v.children.iter().for_each(|c| sources(c, out)),
        pb::Component::TextComponent(v) => block_sources(&v.blocks, out),
        pb::Component::NeoUiComponent(v) => doc_sources(&v.ui, out),
        _ => {}
    }
}

fn block_sources(bs: &[pb::Block], out: &mut Vec<String>) {
    for b in bs {
        match b {
            pb::Block::NeoUiBlock(v) => doc_sources(&v.ui, out),
            pb::Block::QuoteBlock(v) => block_sources(&v.blocks, out),
            pb::Block::ListBlock(v) => v.items.iter().for_each(|i| block_sources(&i.blocks, out)),
            _ => {}
        }
    }
}

/// The `src` of every `Image` in a doc, a gallery's included.
fn doc_sources(doc: &pb::UiDoc, out: &mut Vec<String>) {
    fn visit(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Array(items) => items.iter().for_each(|v| visit(v, out)),
            serde_json::Value::Object(map) => {
                if map.get("component").and_then(|c| c.as_str()) == Some("Image")
                    && let Some(src) = value.pointer("/props/src").and_then(|s| s.as_str())
                {
                    out.push(src.to_string());
                }
                map.values().for_each(|v| visit(v, out));
            }
            _ => {}
        }
    }
    if let Some(root) = &doc.root
        && let Ok(value) = serde_json::to_value(root)
    {
        visit(&value, out);
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
/// An app's name without the number of its instance: `terminal-2` is `terminal`.
fn app_family(app: &str) -> &str {
    match app.rsplit_once('-') {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => app,
    }
}

/// At most `rows` lines, the last one ending in `…` when any were dropped.
fn cut_rows(lines: &mut Lines, rows: usize, w: usize) {
    if lines.len() <= rows {
        return;
    }
    lines.truncate(rows);
    if let Some(last) = lines.last_mut() {
        let mut spans = std::mem::take(&mut last.spans);
        let style = spans.last().map_or(Style::new(), |s| s.style);
        spans.push(Span::styled("…", style));
        last.spans = truncate_spans(spans, w);
    }
}

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

/// The changed lines of a diff with `n` lines of context around each; a stretch left out
/// inside a hunk becomes a `⋯` hunk line, a hunk with nothing left goes.
fn around_changes(lines: &[pb::DiffLine], n: usize) -> Vec<pb::DiffLine> {
    use pb::DiffLineKind::{Add, Hunk, Remove};
    let changed: Vec<usize> = (0..lines.len())
        .filter(|&i| matches!(lines[i].kind, Add | Remove))
        .collect();
    let near = |i: usize| {
        let at = changed.partition_point(|&c| c < i);
        let after = changed.get(at).is_some_and(|&c| c - i <= n);
        let before = at > 0 && i - changed[at - 1] <= n;
        after || before
    };
    let keep: Vec<bool> = (0..lines.len())
        .map(|i| lines[i].kind != Hunk && near(i))
        .collect();
    let mut out: Vec<pb::DiffLine> = Vec::new();
    let mut gap = false;
    for (i, line) in lines.iter().enumerate() {
        if line.kind == Hunk {
            let next = lines[i + 1..]
                .iter()
                .position(|l| l.kind == Hunk)
                .map_or(lines.len(), |p| i + 1 + p);
            if keep[i + 1..next].iter().any(|&k| k) {
                out.push(line.clone());
            }
            gap = false;
        } else if keep[i] {
            if gap {
                out.push(pb::DiffLine {
                    kind: Hunk,
                    text: "⋯".into(),
                    old: None,
                    new: None,
                });
                gap = false;
            }
            out.push(line.clone());
        } else if out.last().is_some_and(|l| l.kind != Hunk) {
            gap = true;
        }
    }
    out
}
