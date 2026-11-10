//! Drawing the GitHub view: the list (tabs, filter line, rows, key hints) and the detail.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::complete::Look;
use super::doc::{self, state_style};
use super::gh::GhError;
use super::model::{Kind, State, Summary, short_age};
use super::{Doc, Github, Mode, Repo, visual, write};
use crate::style::Styles;
use crate::util::{fit_line, split_line, truncate, width};
use crate::chat::Placement;
use crate::images::Images;
use crate::widgets::DiffView;

/// Where an avatar is drawn in a suggestion row, and how many cells wide: a cell is about
/// twice as tall as wide, so two across one down is square.
const AVATAR_X: u16 = 2;
const AVATAR_COLS: u16 = 2;

pub(super) fn draw(
    g: &mut Github,
    frame: &mut Frame,
    area: Rect,
    now_ms: i64,
    st: &Styles,
    images: Option<&mut Images>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Clear, area);
    let (w, h) = (usize::from(area.width), usize::from(area.height));
    if let Mode::Detail(v) = &g.mode
        && let Some(diff) = &v.diff
    {
        draw_diff(diff, frame, area, &g.palette);
        return;
    }
    let (body_h, cursor) = match g.mode {
        Mode::List => (h.saturating_sub(3), true),
        Mode::Detail(_) => (h.saturating_sub(2), false),
    };
    // The input, or what is being chosen in its place, takes the bottom of a loaded detail; the
    // item scrolls above it.
    let loaded = matches!(&g.mode, Mode::Detail(v)
        if g.details.get(&(v.kind, v.number)).is_some_and(|d| d.detail.is_some()));
    let panel = match g.mode {
        Mode::Detail(_) if loaded => g.panel(w, body_h, st),
        _ => write::Panel {
            lines: Vec::new(),
            cursor: None,
        },
    };
    let above = body_h.saturating_sub(panel.lines.len());
    // The body first: it learns how many rows there are, which the head reports on.
    let body = match g.mode {
        Mode::List => list_body(g, w, body_h, now_ms, st),
        Mode::Detail(_) => detail_body(g, w, above, now_ms, st, images.is_some()),
    };
    let head = match g.mode {
        Mode::List => list_head(g, w, st),
        Mode::Detail(_) => detail_head(g, w, st),
    };
    let mut lines = head;
    let head_h = lines.len();
    lines.extend(body);
    lines.resize(head_h + above, Line::default());
    lines.extend(panel.lines);
    lines.resize(head_h + body_h, Line::default());
    // Where the pictures of people go: (row, column, address).
    let mut avatars: Vec<(usize, u16, String)> = Vec::new();
    if cursor {
        for (i, (row, avatar)) in dropdown(g, w, st, images.is_some())
            .into_iter()
            .enumerate()
        {
            if let Some(slot) = lines.get_mut(head_h + i) {
                *slot = row;
                avatars.extend(avatar.map(|url| (head_h + i, AVATAR_X, url)));
            }
        }
    }
    if let Mode::Detail(v) = &g.mode
        && let Some(doc) = &v.doc
    {
        for a in &doc.avatars {
            if a.line >= v.scroll && a.line < v.scroll + v.rows.min(above) {
                avatars.push((
                    head_h + a.line - v.scroll,
                    a.x + GUTTER as u16,
                    doc::avatar_url(&a.login),
                ));
            }
        }
    }
    lines.push(footer(g, w, st));
    lines.truncate(h);
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .map(|l| fit_line(l, w))
                .collect::<Vec<_>>(),
        ),
        area,
    );
    if let Some(images) = images {
        images.request(avatars.iter().map(|(_, _, url)| url.as_str()));
        for (row, x, url) in avatars {
            let at = Placement {
                src: url,
                x,
                y: row as i32,
                cols: AVATAR_COLS,
                rows: 1,
            };
            images.draw_quiet(frame.buffer_mut(), area, &at);
        }
    }
    if cursor && let Some(text) = &g.typing {
        let x = (2 + width(text)).min(w - 1) as u16;
        frame.set_cursor_position(Position::new(area.x + x, area.y + 1));
    }
    if let Some((row, col)) = panel.cursor {
        let x = col.min(w.saturating_sub(1)) as u16;
        frame.set_cursor_position(Position::new(area.x + x, area.y + (head_h + above + row) as u16));
    }
}

fn draw_diff(
    diff: &DiffView,
    frame: &mut Frame,
    area: Rect,
    palette: &neo_renderer_ratatui::Palette,
) {
    diff.draw(frame, area, palette);
}

// ---- list ----

fn list_head(g: &Github, w: usize, st: &Styles) -> Vec<Line<'static>> {
    let tab = |kind: Kind, n: &str, name: &str| {
        if g.tab == kind {
            Span::styled(
                format!("[{n} {name}]"),
                st.accent.add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {n} {name} "), st.muted)
        }
    };
    let tabs = Line::from(vec![
        tab(Kind::Issue, "1", "Issues"),
        Span::raw(" "),
        tab(Kind::Pr, "2", "Pull requests"),
    ]);
    let repo = match &g.repo {
        Repo::Ready(name) => Some(Line::from(Span::styled(name.clone(), st.muted))),
        _ => None,
    };
    let filter = match &g.typing {
        Some(text) => Line::from(vec![Span::styled("/ ", st.accent), Span::raw(text.clone())]),
        None => {
            let mut parts = vec![if g.all { "all states" } else { "open" }.to_string()];
            if !g.query.is_empty() {
                parts.push(format!("search: {}", g.query));
            }
            if let Some(data) = g.lists.get(&g.current()) {
                if data.loading {
                    parts.push("loading…".into());
                } else if data.error.is_none() {
                    parts.push(format!("{} shown", data.rows.len()));
                }
            }
            Line::from(Span::styled(parts.join(" · "), st.muted))
        }
    };
    vec![split_line(tabs, repo, w), filter]
}

pub(super) fn list_body(
    g: &mut Github,
    w: usize,
    h: usize,
    now_ms: i64,
    st: &Styles,
) -> Vec<Line<'static>> {
    g.list_rows = h.max(1);
    let colors = g.label_colors();
    let key = g.current();
    match &g.repo {
        Repo::Unknown | Repo::Loading => return vec![note("loading…", st)],
        Repo::Failed(e) => return error_lines(e, w, st),
        Repo::Ready(_) => {}
    }
    let Some(data) = g.lists.get_mut(&key) else {
        return vec![note("loading…", st)];
    };
    if let Some(e) = &data.error {
        return error_lines(e, w, st);
    }
    if data.rows.is_empty() {
        let text = if data.loading {
            "loading…"
        } else if key.query.is_empty() && !key.all {
            match key.kind {
                Kind::Issue => "no open issues (o shows closed ones too)",
                Kind::Pr => "no open pull requests (o shows closed ones too)",
            }
        } else {
            "nothing matches"
        };
        return vec![note(text, st)];
    }
    // Each issue is a block of lines, so the selection is kept in view by blocks.
    let h = h.max(1);
    let blocks: Vec<Vec<Line<'static>>> = data
        .rows
        .iter()
        .enumerate()
        .map(|(i, s)| item(s, i == data.selected, w, now_ms, st, &colors))
        .collect();
    data.selected = data.selected.min(data.rows.len() - 1);
    data.top = data.top.min(data.selected);
    // The last block has no gap under it, so it needs one line less.
    let span = |from: usize, to: usize| -> usize {
        blocks[from..=to].iter().map(Vec::len).sum::<usize>() - 1
    };
    while data.top < data.selected && span(data.top, data.selected) > h {
        data.top += 1;
    }
    let mut out = Vec::new();
    let mut shown = 0;
    for block in &blocks[data.top..] {
        if !out.is_empty() && out.len() + block.len() - 1 > h {
            break;
        }
        out.extend(block.iter().cloned());
        shown += 1;
    }
    g.list_rows = shown.max(1);
    out
}

/// One issue or pull request: its title in full, a muted line of facts and label chips, and
/// an empty line to set it apart from the next.
fn item(
    s: &Summary,
    selected: bool,
    w: usize,
    now_ms: i64,
    st: &Styles,
    colors: &Colors,
) -> Vec<Line<'static>> {
    let bar = if selected {
        Span::styled("▎ ", st.accent)
    } else {
        Span::raw("  ")
    };
    let glyph = match s.state {
        State::Open => "●",
        State::Closed => "✓",
        State::Merged => "◆",
        State::Draft => "○",
    };
    let number = format!("#{} ", s.number);
    let lead = 2 + 2 + width(&number);
    let title_style = if selected {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let mut out = Vec::new();
    for (i, line) in super::md::paragraph(&s.title, w.saturating_sub(lead).max(1), title_style)
        .into_iter()
        .enumerate()
    {
        let mut spans = vec![bar.clone()];
        if i == 0 {
            spans.push(Span::styled(format!("{glyph} "), state_style(s.state, st)));
            spans.push(Span::styled(number.clone(), st.muted));
        } else {
            spans.push(Span::raw(" ".repeat(lead - 2)));
        }
        spans.extend(line.spans);
        out.push(Line::from(spans));
    }
    let mut facts = vec![s.author.clone()];
    if s.comments > 0 {
        let n = s.comments;
        facts.push(format!("{n} comment{}", if n == 1 { "" } else { "s" }));
    }
    let age = short_age(s.updated_ms, now_ms);
    if !age.is_empty() {
        facts.push(format!("{age} ago"));
    }
    let mut meta = vec![bar, Span::raw(" ".repeat(lead - 2))];
    meta.push(Span::styled(facts.join(" · "), st.muted));
    for label in &s.labels {
        meta.push(Span::raw(" "));
        meta.push(Span::styled(
            format!(" {label} "),
            chip_style(colors.get(label).copied(), st),
        ));
    }
    out.push(fit_line(Line::from(meta), w));
    // The issue sits on a background of its own; the empty line under it is the gap.
    if st.raised != Color::Reset {
        for line in &mut out {
            fill(line, w, st.raised);
        }
    }
    out.push(Line::default());
    out
}

/// The label colors of the repository, by name.
pub(super) type Colors = std::collections::HashMap<String, u32>;

/// A label as a chip: in the color GitHub gives it (the text black or white to read on it), or
/// reversed muted while that is not known.
pub(super) fn chip_style(rgb: Option<u32>, st: &Styles) -> Style {
    let Some(rgb) = rgb else {
        return st.muted.add_modifier(Modifier::REVERSED);
    };
    let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
    let fg = if luma > 140 { Color::Black } else { Color::White };
    Style::new().fg(fg).bg(Color::Rgb(r, g, b))
}

/// The line padded to `w` and every part of it that has no background on `bg`.
pub(super) fn fill(line: &mut Line<'static>, w: usize, bg: Color) {
    let pad = w.saturating_sub(line.width());
    line.spans.push(Span::raw(" ".repeat(pad)));
    for span in &mut line.spans {
        if span.style.bg.is_none() {
            span.style = span.style.bg(bg);
        }
    }
}

/// The suggestions for the search being typed, one row each over the top of the list: labels
/// in their own colors, qualifiers and people in the accent.
///
/// With `pictures` the people get room for their avatar; the address of each comes along with
/// its row, to be drawn over the blank.
fn dropdown(
    g: &Github,
    w: usize,
    st: &Styles,
    pictures: bool,
) -> Vec<(Line<'static>, Option<String>)> {
    let chosen = g.pick;
    g.suggestions()
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let on = chosen == Some(i);
            let mut avatar = None;
            let mut row = Style::new().bg(st.raised);
            if on {
                row = row.add_modifier(Modifier::BOLD);
            }
            let mut spans = vec![Span::styled(
                if on { "› " } else { "  " },
                row.patch(st.accent),
            )];
            match &c.look {
                Look::Label(rgb) => spans.push(Span::styled(
                    format!(" {} ", c.shown),
                    chip_style(Some(*rgb), st),
                )),
                Look::Key => spans.push(Span::styled(c.shown.clone(), row.patch(st.accent))),
                Look::Person(url) => {
                    if pictures {
                        spans.push(Span::styled(" ".repeat(usize::from(AVATAR_COLS) + 1), row));
                        avatar = url.clone();
                    }
                    spans.push(Span::styled(c.shown.clone(), row.patch(st.accent)));
                }
                Look::Word => spans.push(Span::styled(c.shown.clone(), row)),
            }
            if !c.note.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", c.note),
                    row.patch(st.muted),
                ));
            }
            let mut line = fit_line(Line::from(spans), w);
            let used = line.width();
            line.spans
                .push(Span::styled(" ".repeat(w.saturating_sub(used)), row));
            (line, avatar)
        })
        .collect()
}

fn note(text: &str, st: &Styles) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), st.muted))
}

/// A failure in words, and what to do about it.
fn error_lines(e: &GhError, w: usize, st: &Styles) -> Vec<Line<'static>> {
    let mut out = super::md::paragraph(&e.message(), w, st.danger);
    out.push(Line::default());
    out.push(note("r to try again", st));
    out
}

// ---- detail ----

fn detail_head(g: &Github, w: usize, st: &Styles) -> Vec<Line<'static>> {
    let Mode::Detail(v) = &g.mode else {
        return Vec::new();
    };
    let noun = match v.kind {
        Kind::Issue => "Issue",
        Kind::Pr => "Pull request",
    };
    let mut left = vec![Span::styled(
        format!("{noun} #{}", v.number),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if let Some(repo) = g.repo_name() {
        left.push(Span::styled(format!("  {repo}"), st.muted));
    }
    if g.details
        .get(&(v.kind, v.number))
        .is_some_and(|d| d.loading)
    {
        left.push(Span::styled("  loading…", st.muted));
    }
    let total = v.doc.as_ref().map_or(0, |d| d.lines.len());
    let right = (total > v.rows && v.rows > 0).then(|| {
        let first = v.scroll.min(total.saturating_sub(v.rows)) + 1;
        let last = (first + v.rows - 1).min(total);
        Line::from(Span::styled(
            format!("lines {first}–{last} of {total}"),
            st.muted,
        ))
    });
    vec![split_line(Line::from(left), right, w)]
}

pub(super) fn detail_body(
    g: &mut Github,
    w: usize,
    h: usize,
    now_ms: i64,
    st: &Styles,
    pictures: bool,
) -> Vec<Line<'static>> {
    let colors = g.label_colors();
    let Mode::Detail(v) = &mut g.mode else {
        return Vec::new();
    };
    v.rows = h.max(1);
    let Some(data) = g.details.get(&(v.kind, v.number)) else {
        return vec![note("loading…", st)];
    };
    let Some(detail) = &data.detail else {
        return match &data.error {
            Some(e) => error_lines(e, w, st),
            None => vec![note("loading…", st)],
        };
    };
    let minute = now_ms / 60_000;
    // The lines leave room on the left for the mark of the block in focus.
    let inner = w.saturating_sub(GUTTER).max(1);
    let stale = !v.doc.as_ref().is_some_and(|d| {
        d.width == inner
            && d.version == data.version
            && d.minute == minute
            && d.colors == colors.len()
            && d.pictures == pictures
    });
    if stale {
        let with = doc::With {
            colors: &colors,
            pictures,
            fields: &data.fields,
        };
        let built = doc::build(detail, inner, now_ms, st, &with);
        v.doc = Some(Doc {
            width: inner,
            version: data.version,
            minute,
            colors: colors.len(),
            pictures,
            lines: built.lines,
            avatars: built.avatars,
            blocks: built.blocks,
        });
        // The block in focus is gone (a comment deleted): the header instead.
        if v.index().is_none() {
            v.focus = doc::Part::Head;
            v.cur = None;
        }
        // Inside a block, the cursor walks its new lines.
        if let Some(b) = v.block()
            && v.cur.is_some()
        {
            let text = v.block_text(&b);
            if let Some(cur) = &mut v.cur {
                cur.set_lines(text);
            }
        }
    }
    v.scroll = v.scroll.min(v.max_scroll());
    let focused = v.block();
    // The wheel moved the view away from the cursor: it comes along, within its block.
    if let (Some(b), Some(cur)) = (focused, v.cur.as_mut()) {
        let top = v.scroll.max(b.start);
        let bottom = (v.scroll + v.rows).min(b.end).saturating_sub(1);
        let row = b.start + cur.row();
        if top <= bottom && (row < top || row > bottom) {
            cur.go_to_row(row.clamp(top, bottom) - b.start);
        }
    }
    let doc = v.doc.as_ref().expect("built above");
    doc.lines
        .iter()
        .enumerate()
        .skip(v.scroll)
        .take(v.rows)
        .map(|(r, line)| {
            let within = focused.filter(|b| r >= b.start && r < b.end);
            let mut out = match (within, &v.cur) {
                (Some(b), Some(cur)) => mark_cursor(line, r - b.start, cur),
                _ => line.clone(),
            };
            let mark = if within.is_some() {
                Span::styled("▎", st.accent)
            } else {
                Span::raw(" ")
            };
            out.spans.insert(0, mark);
            out
        })
        .collect()
}

/// Columns on the left of the detail for the mark of the block in focus.
const GUTTER: usize = 1;

/// The line with the selection and the cursor drawn on it: the selected characters reversed,
/// the cursor a block (bold and underlined within a selection). An empty line shows the cursor
/// as a space.
fn mark_cursor(line: &Line<'static>, row: usize, cur: &visual::Cursor) -> Line<'static> {
    let at_cursor = |c: usize| row == cur.row() && c == cur.col();
    let on_row = row == cur.row();
    if !on_row && !cur.visual() {
        return line.clone();
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut c = 0;
    let mut push = |ch: char, base: Style, c: usize| {
        let mut style = base;
        if cur.selects(row, c) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        if at_cursor(c) {
            style = if cur.selects(row, c) {
                style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                style.add_modifier(Modifier::REVERSED)
            };
        }
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(ch),
            _ => spans.push(Span::styled(ch.to_string(), style)),
        }
    };
    for span in &line.spans {
        for ch in span.content.chars() {
            push(ch, span.style, c);
            c += 1;
        }
    }
    if on_row && cur.col() >= c {
        push(' ', Style::new(), cur.col());
    }
    Line::from(spans)
}

// ---- footer ----

/// The key hints that fit, most useful first.
fn hints(parts: &[&str], w: usize, st: &Styles) -> Line<'static> {
    let mut text = String::new();
    for part in parts {
        let next = if text.is_empty() {
            (*part).to_string()
        } else {
            format!("{text} · {part}")
        };
        if width(&next) > w {
            break;
        }
        text = next;
    }
    if text.is_empty() {
        text = truncate(parts.first().copied().unwrap_or(""), w);
    }
    Line::from(Span::styled(text, st.muted))
}

fn footer(g: &Github, w: usize, st: &Styles) -> Line<'static> {
    if let Some(note) = &g.note {
        return Line::from(Span::styled(truncate(note, w), st.danger));
    }
    if let Some((_, number, _)) = g.pending {
        return Line::from(Span::styled(
            truncate(&format!("fetching #{number}…"), w),
            st.muted,
        ));
    }
    if g.typing.is_some() {
        return hints(
            &["enter search", "tab complete", "↑↓ choose", "esc cancel", "ctrl-u clear"],
            w,
            st,
        );
    }
    if let Some(info) = &g.info {
        return Line::from(Span::styled(truncate(info, w), st.muted));
    }
    if matches!(g.writing, Some(write::Writing::Comment)) {
        return hints(
            &["ctrl-s post", "enter new line", "tab back to reading", "ctrl-d discard"],
            w,
            st,
        );
    }
    if g.writing.is_some() {
        return hints(&["esc cancel"], w, st);
    }
    match &g.mode {
        Mode::List => hints(
            &[
                "enter open",
                "a attach",
                "s send",
                "↑↓ move",
                "/ search",
                "o open/all",
                "tab switch",
                "r reload",
                "esc close",
            ],
            w,
            st,
        ),
        // Selecting: what to do with it.
        Mode::Detail(v) if v.cur.as_ref().is_some_and(|c| c.visual()) => {
            hints(&["a add to message", "v V o select", "esc leave visual"], w, st)
        }
        Mode::Detail(v) if v.cur.is_some() => hints(
            &["v V select", "hjkl w b e move", "tab write", "esc leave the block"],
            w,
            st,
        ),
        // What the block in focus allows first, then what works everywhere.
        Mode::Detail(v) => {
            use doc::Part;
            let pr = v.kind == Kind::Pr;
            let closed = g
                .details
                .get(&(v.kind, v.number))
                .and_then(|d| d.detail.as_ref())
                .is_some_and(|d| d.state == State::Closed);
            let mut parts = match v.focus {
                Part::Head => vec![
                    if closed { "x reopen" } else { "x close" },
                    "S title",
                    "m edit labels, assignees…",
                ],
                Part::Description | Part::Comment(_) => {
                    vec!["enter select", "E edit", "+ react", "> quote"]
                }
                Part::Review(_) | Part::DiffComment(_) => vec!["enter select", "> quote"],
                Part::Checks => vec!["enter select"],
                Part::Files => vec!["d diff", "enter select"],
            };
            if pr && v.focus == Part::Head {
                parts.push("d diff");
            }
            if v.focus == Part::Head {
                parts.push("enter select");
            }
            parts.push("j k move");
            parts.push(if g.draft.is_empty() { "tab write" } else { "tab write (draft)" });
            parts.extend(["a attach", "s send", "esc back", "q close"]);
            hints(&parts, w, st)
        }
    }
}
