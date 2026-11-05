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
    fn place_rooms(&mut self, lines: &mut Lines) {
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
    fn user(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
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

    /// A header (the label, red when the call failed), then the result: summarized under `⎿`
    /// in muted, or in full when expanded. Items with buttons are always shown in full: the
    /// buttons are what the item is for.
    fn tool(&mut self, it: &pb::ViewItem, label: Option<String>, w: usize) -> Lines {
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
    fn title_detail(&self, head: &mut Lines, detail: Lines, w: usize) {
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
    fn tool_label(&self, label: &str) -> Vec<Span<'static>> {
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

    fn block(&mut self, b: &pb::Block, w: usize) -> Lines {
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
    fn prose(&self, w: usize) -> usize {
        self.o.prose_width.map_or(w, |cap| w.min(cap.max(1)))
    }

    /// Levels 1 and 2 bold in the accent color, the rest bold. Not underlined: that is how
    /// links look.
    fn heading_style(&self, level: i64) -> Style {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        match level {
            1 | 2 => bold.fg(self.p.accent),
            _ => bold,
        }
    }

    /// A markdown table in a box. Cells wrap inside their column; columns that don't fit share
    /// the room (the widest give first). Once a row takes more than one line, rules separate the
    /// rows too. Too narrow for the box: each row as `header: cell` lines.
    fn table_block(&self, v: &pb::TableBlock, w: usize) -> Lines {
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
    fn stacked_table(
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
    fn list(&mut self, v: &pb::ListBlock, w: usize) -> Lines {
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

    /// With images on: room for the picture when it is known, else a line of text saying why
    /// not; then the caption. Without: `[image alt · W×H]`.
    pub(crate) fn image(&mut self, v: &pb::ImageComponent, w: usize) -> Lines {
        let muted = self.p.muted_style();
        let name = v
            .alt
            .as_deref()
            .map(one_line)
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| one_line(&v.src));
        let mut out = match self.o.images.as_ref() {
            None => {
                let mut s = format!("[image {name}");
                if let (Some(iw), Some(ih)) = (v.width, v.height) {
                    s.push_str(&format!(" · {iw}×{ih}"));
                }
                s.push(']');
                wrap(&[Span::styled(s, muted)], w)
            }
            Some(images) => match images.status.get(&v.src) {
                Some(ImageStatus::Ready { width, height }) => {
                    let name = v
                        .alt
                        .as_deref()
                        .map(one_line)
                        .filter(|a| !a.is_empty())
                        .unwrap_or_else(|| file_name(&v.src));
                    self.room(images, &v.src, (*width, *height), &name, w)
                }
                Some(ImageStatus::Failed(why)) => wrap(
                    &[Span::styled(
                        format!("[image {name} · {}]", one_line(why)),
                        muted,
                    )],
                    w,
                ),
                _ => wrap(
                    &[Span::styled(
                        format!("[image {name}] loading image…"),
                        muted,
                    )],
                    w,
                ),
            },
        };
        if let Some(cap) = v.caption.as_deref().map(clean).filter(|c| !c.is_empty()) {
            out.extend(wrap(&[Span::raw(cap.into_owned())], w));
        }
        out
    }

    /// The marked first line, the empty lines under it, then `name · W×H`.
    fn room(
        &mut self,
        images: &ImageOptions,
        src: &str,
        size: (u32, u32),
        name: &str,
        w: usize,
    ) -> Lines {
        let max_cols = u16::try_from(w).unwrap_or(u16::MAX);
        let (cols, rows) = filled_cells(size.0, size.1, images.cell, max_cols, images.max_rows);
        let mut out = self.reserve_cells(src, cols, rows);
        let caption = format!("{name} · {}×{}", size.0, size.1);
        out.extend(wrap(&[Span::styled(caption, self.p.muted_style())], w));
        out
    }

    /// How many of `children`, from the first, are pictures ready to draw: a run of them shares
    /// the width.
    fn ready_pictures(&self, children: &[pb::Component]) -> usize {
        let Some(images) = self.o.images.as_ref() else {
            return 0;
        };
        children
            .iter()
            .take_while(|c| match c {
                pb::Component::ImageComponent(v) => {
                    matches!(images.status.get(&v.src), Some(ImageStatus::Ready { .. }))
                }
                _ => false,
            })
            .count()
    }

    /// Pictures side by side, up to three a row, each row as tall as its least tall picture
    /// would be in its share of the width (so they line up), each with its name under it.
    fn pictures(&mut self, pictures: &[&pb::ImageComponent], w: usize) -> Lines {
        let Some(images) = self.o.images.as_ref() else {
            return Vec::new();
        };
        let size = |v: &pb::ImageComponent| match images.status.get(&v.src) {
            Some(ImageStatus::Ready { width, height }) => (*width, *height),
            _ => (1, 1),
        };
        let per_row = (w + GAP) / (MIN_PICTURE + GAP);
        let per_row = per_row.clamp(1, PICTURES_PER_ROW);
        let mut out: Lines = Vec::new();
        for row in pictures.chunks(per_row) {
            if !out.is_empty() {
                out.push(Line::default());
            }
            let share = (w.saturating_sub(GAP * (row.len() - 1)) / row.len()).max(1);
            let share16 = u16::try_from(share).unwrap_or(u16::MAX);
            let tall = row
                .iter()
                .map(|v| {
                    let (iw, ih) = size(v);
                    filled_cells(iw, ih, images.cell, share16, images.max_rows).1
                })
                .min()
                .unwrap_or(1);
            let parts: Vec<(Lines, usize)> = row
                .iter()
                .map(|v| {
                    let (iw, ih) = size(v);
                    let (cols, rows) = filled_cells(iw, ih, images.cell, share16, tall);
                    let mut lines = self.reserve_cells(&v.src, cols, rows);
                    lines.resize(usize::from(tall), Line::default());
                    let name = v
                        .alt
                        .as_deref()
                        .map(one_line)
                        .filter(|a| !a.is_empty())
                        .unwrap_or_else(|| file_name(&v.src));
                    let caption = truncate(&format!("{name} · {iw}×{ih}"), share);
                    lines.push(sline(caption, self.p.muted_style()));
                    (lines, share)
                })
                .collect();
            out.extend(crate::neoui::columns(parts, &vec![GAP; row.len()], "start"));
        }
        out
    }

    /// Room for a picture of `size` pixels in at most `max_cols` x `max_rows` cells: the marked
    /// first line and the empty lines under it, and the cells taken. Measuring (a quiet draw)
    /// reserves nothing, since its lines are thrown away.
    pub(crate) fn reserve(
        &mut self,
        images: &ImageOptions,
        src: &str,
        (iw, ih): (u32, u32),
        max_cols: u16,
        max_rows: u16,
    ) -> (Lines, (u16, u16)) {
        let (cols, rows) = image_cells(iw, ih, images.cell, max_cols, max_rows);
        (self.reserve_cells(src, cols, rows), (cols, rows))
    }

    /// Room for a picture of `cols` x `rows` cells: the marked first line and the empty lines
    /// under it (what is put before the mark moves the picture right).
    pub(crate) fn reserve_cells(&mut self, src: &str, cols: u16, rows: u16) -> Lines {
        if !self.ui.drawing_to_measure() {
            self.rooms.push((cols, rows, src.to_string()));
        }
        let mut out = vec![Line::from(Span::raw(SLOT_MARK))];
        out.resize(usize::from(rows), Line::default());
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

    /// A colored diff under its path: unified, or side by side when the app asks for it
    /// ([`Options::split_diffs`]) and the width allows.
    pub(crate) fn diff(&self, v: &pb::DiffComponent, w: usize) -> Lines {
        let mut out = self.diff_title(v, w);
        out.extend(self.diff_rows(v.path.as_deref(), &v.lines, w));
        out
    }

    /// The diff's path, bold, with its icon.
    fn diff_title(&self, v: &pb::DiffComponent, w: usize) -> Lines {
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
    fn diff_style(&self, kind: pb::DiffLineKind) -> (&'static str, Style, bool) {
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
    fn diff_highlighters(
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
    fn diff_spans(
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
    fn unified_rows(&self, path: Option<&str>, lines: &[pb::DiffLine], w: usize) -> Lines {
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
    fn split_rows(&self, path: Option<&str>, lines: &[pb::DiffLine], w: usize) -> Lines {
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
    fn side(
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
    fn beside(
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
