use super::*;

/// Where the cards of `refs` start in an item's `lines`: the first line, after the last one
/// found, that shows the ref's sha or number as a word of its own. A card not drawn (in a tab
/// not shown, say) is left out.
pub(super) fn ref_lines(refs: &[DevRef], lines: &[Line<'static>]) -> Vec<(usize, DevRef)> {
    let mut out = Vec::new();
    let mut from = 0;
    for dev in refs {
        let token = dev.token();
        let shows = |line: &Line<'_>| {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            text.match_indices(&token).any(|(i, _)| {
                let before = text[..i].chars().next_back();
                let after = text[i + token.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            })
        };
        if let Some(at) = (from..lines.len()).find(|&i| shows(&lines[i])) {
            out.push((at, dev.clone()));
            from = at + 1;
        }
    }
    out
}

/// The chars of `text` in cells `from..to`; a wide char counts when it starts in them.
pub(super) fn cells(text: &str, from: usize, to: usize) -> String {
    let mut col = 0;
    let mut out = String::new();
    for c in text.chars() {
        if col >= to {
            break;
        }
        if col >= from {
            out.push(c);
        }
        col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    }
    out
}

/// The diffs in an item's result, in order (a path twice is summed).
pub(super) fn changed_files(item: &ViewItem) -> Vec<Changed> {
    fn walk(c: &Component, out: &mut Vec<Changed>) {
        match c {
            Component::CardComponent(v) => v.children.iter().for_each(|c| walk(c, out)),
            Component::RowComponent(v) => v.children.iter().for_each(|c| walk(c, out)),
            Component::StackComponent(v) => v.children.iter().for_each(|c| walk(c, out)),
            Component::DiffComponent(v) => {
                let Some(path) = v.path.as_deref().filter(|p| !p.is_empty()) else {
                    return;
                };
                let count = |kind| v.lines.iter().filter(|l| l.kind == kind).count();
                add_changed(
                    out,
                    Changed {
                        path: path.to_string(),
                        added: count(DiffLineKind::Add),
                        removed: count(DiffLineKind::Remove),
                    },
                );
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&item.component, &mut out);
    out
}

/// The first diff in an item that names its file.
pub(super) fn first_diff(c: &Component) -> Option<&DiffComponent> {
    match c {
        Component::CardComponent(v) => v.children.iter().find_map(first_diff),
        Component::RowComponent(v) => v.children.iter().find_map(first_diff),
        Component::StackComponent(v) => v.children.iter().find_map(first_diff),
        Component::DiffComponent(v) if v.path.as_deref().is_some_and(|p| !p.is_empty()) => Some(v),
        _ => None,
    }
}

pub(super) fn add_changed(all: &mut Vec<Changed>, c: Changed) {
    match all.iter_mut().find(|x| x.path == c.path) {
        Some(x) => {
            x.added += c.added;
            x.removed += c.removed;
        }
        None => all.push(c),
    }
}

/// An edit the agent made: a tool call that ran, with a diff, that isn't a question.
pub(super) fn is_edit(item: &ViewItem) -> bool {
    item.kind == ItemKind::Tool
        && item.state == Some(ItemState::Done)
        && !item.r#ref.starts_with("p-")
        && !item.r#ref.starts_with("q-")
        && !changed_files(item).is_empty()
}

/// Each finished turn that changed files: the index of its last item and the files, summed per
/// path. A turn ends where the next user message starts; the last one once it isn't running.
pub(super) fn turn_changes(items: &[ViewItem], running: bool) -> Vec<(usize, Vec<Changed>)> {
    let mut out = Vec::new();
    let mut turn: Vec<Changed> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if item.kind == ItemKind::User && i > 0 && !turn.is_empty() {
            out.push((i - 1, std::mem::take(&mut turn)));
        }
        if is_edit(item) {
            for c in changed_files(item) {
                add_changed(&mut turn, c);
            }
        }
    }
    if !running && !turn.is_empty() {
        out.push((items.len() - 1, turn));
    }
    out
}

/// A turn's changes: `3 files changed  +40 −12`, then each file with its icon and counts, its
/// path relative to `cwd` when it is in there.
pub(super) fn changes_lines(
    changed: &[Changed],
    width_cells: usize,
    opts: &ChatOpts,
    cwd: Option<&str>,
) -> Vec<Line<'static>> {
    let p = &opts.palette;
    let muted = Style::new().fg(p.muted);
    let counts = |added: usize, removed: usize| {
        vec![
            Span::styled(format!("+{added}"), Style::new().fg(p.success)),
            Span::raw(" "),
            Span::styled(format!("−{removed}"), Style::new().fg(p.danger)),
        ]
    };
    let (added, removed) = changed
        .iter()
        .fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
    let n = changed.len();
    let mut head = vec![Span::styled(
        format!("{n} file{} changed  ", if n == 1 { "" } else { "s" }),
        muted,
    )];
    head.extend(counts(added, removed));
    let mut out = vec![Line::from(head)];
    for c in changed {
        let mut spans = vec![Span::raw("  ")];
        if opts.icons {
            spans.push(file_icon(&c.path, p));
            spans.push(Span::raw(" "));
        }
        let tail = counts(c.added, c.removed);
        let used: usize = spans.iter().chain(&tail).map(|s| width(&s.content)).sum();
        let room = width_cells.saturating_sub(used + 2).max(1);
        spans.push(Span::raw(truncate_start(relative_path(&c.path, cwd), room)));
        spans.push(Span::raw("  "));
        spans.extend(tail);
        out.push(Line::from(spans));
    }
    out
}

/// The picked file's line: reversed and bold across the whole pane, so it can't be missed
/// whatever the colors (`NO_COLOR` too).
pub(super) fn picked_line(line: &Line<'static>, width_cells: usize) -> Line<'static> {
    let mut line = line.clone();
    let used: usize = line.spans.iter().map(|s| width(&s.content)).sum();
    line.spans
        .push(Span::raw(" ".repeat(width_cells.saturating_sub(used))));
    for s in &mut line.spans {
        s.style = s.style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
    }
    line
}

/// A line of the picked reply: tinted with the accent across the pane (its code keeps its own
/// background). Without true colors the first line is drawn like a picked file's.
pub(super) fn picked_reply_line(
    line: &Line<'static>,
    first: bool,
    width_cells: usize,
    p: &Palette,
) -> Line<'static> {
    let Some(tint) = mix(p.bg, p.accent, 0.15) else {
        return if first {
            picked_line(line, width_cells)
        } else {
            line.clone()
        };
    };
    let mut line = line.clone();
    let used: usize = line.spans.iter().map(|s| width(&s.content)).sum();
    line.spans
        .push(Span::raw(" ".repeat(width_cells.saturating_sub(used))));
    for s in &mut line.spans {
        if s.style
            .bg
            .is_none_or(|bg| bg == ratatui::style::Color::Reset)
        {
            s.style = s.style.bg(tint);
        }
    }
    line
}

/// A tool call that can fold into a run: finished, not a question (`p-`/`q-`, which say what was
/// answered), not an edit (what was written stays in sight) and not a command (each says what it
/// was for, on a line of its own).
pub(super) fn foldable(
    item: &ViewItem,
    buttons: &HashMap<String, Vec<ButtonRef>>,
    docs: &HashMap<String, Vec<UiDocInfo>>,
) -> bool {
    item.kind == ItemKind::Tool
        && matches!(item.state, Some(ItemState::Done | ItemState::Failed))
        && !buttons.contains_key(&item.r#ref)
        && !docs
            .get(&item.r#ref)
            .is_some_and(|d| d.iter().any(|d| d.interactive))
        && !item.r#ref.starts_with("p-")
        && !item.r#ref.starts_with("q-")
        && item.command.is_none()
        && changed_files(item).is_empty()
}

/// A folded run: `15 tool calls · 7 fetch, 5 read, 3 search · 1 failed`, or `12 read` when all
/// are one tool; all muted but the failures.
pub(super) fn folded_line(run: &[&ViewItem], width: usize, p: &Palette) -> Line<'static> {
    let mut verbs: Vec<(String, usize)> = Vec::new();
    for item in run {
        let verb = item
            .label
            .as_deref()
            .unwrap_or("tool")
            .split_whitespace()
            .next()
            .unwrap_or("tool")
            .trim_end_matches(':')
            .to_string();
        match verbs.iter_mut().find(|(v, _)| *v == verb) {
            Some((_, n)) => *n += 1,
            None => verbs.push((verb, 1)),
        }
    }
    // Most first; ties in the order they came.
    verbs.sort_by_key(|v| std::cmp::Reverse(v.1));
    let counts: Vec<String> = verbs.iter().map(|(v, n)| format!("{n} {v}")).collect();
    let failed = run
        .iter()
        .filter(|i| i.state == Some(ItemState::Failed))
        .count();
    let muted = Style::new().fg(p.muted);
    let mut text = match counts.as_slice() {
        [one] => one.clone(),
        _ => format!("{} tool calls · {}", run.len(), counts.join(", ")),
    };
    let tail = (failed > 0).then(|| format!(" · {failed} failed"));
    let room = width.saturating_sub(tail.as_ref().map_or(0, |t| t.chars().count()));
    if text.chars().count() > room {
        text = text
            .chars()
            .take(room.saturating_sub(1))
            .collect::<String>()
            + "…";
    }
    let mut spans = vec![Span::styled(text, muted)];
    if let Some(tail) = tail {
        spans.push(Span::styled(tail, Style::new().fg(p.danger)));
    }
    Line::from(spans)
}

/// A permission question that was answered with an allow: the session view marks it done.
pub(super) fn is_allowed_question(item: &ViewItem) -> bool {
    item.kind == ItemKind::Tool
        && item.r#ref.starts_with("p-")
        && item.state == Some(ItemState::Done)
}

/// How long the light takes to sweep across the thinking line.
pub(super) const SWEEP_MS: i64 = 1600;

/// The thinking line with a light sweeping over it: muted text, the accent color where the light
/// is. Without true colors the lit characters are the accent color and bold.
pub(super) fn thinking_line(line: &Line<'static>, now_ms: i64, p: &Palette) -> Line<'static> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as f32;
    // From before the start to past the end, so the light comes in and leaves.
    let phase = now_ms.rem_euclid(SWEEP_MS) as f32 / SWEEP_MS as f32;
    let center = -4.0 + phase * (n + 8.0);
    let spans = chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let light = (1.0 - (i as f32 - center).abs() / 4.0).max(0.0);
            let style = match mix(p.muted, p.accent, light) {
                Some(color) => Style::new().fg(color),
                None if light > 0.5 => Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
                None => Style::new().fg(p.muted),
            };
            Span::styled(c.to_string(), style)
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

/// The line with `hint` (how long it has run, [`BACKGROUND_HINT`]) at its end, the line cut to
/// make room when it is long.
pub(super) fn with_hint(
    line: Line<'static>,
    hint: &str,
    width: usize,
    p: &Palette,
) -> Line<'static> {
    let hint = Span::styled(hint.to_string(), Style::new().fg(p.muted));
    let room = width.saturating_sub(hint.width());
    if room < 10 {
        return line;
    }
    let mut line = if line.width() > room {
        crate::util::fit_line(line, room)
    } else {
        line
    };
    line.spans.push(hint);
    line
}

/// How far the light of a sweep reaches either side of its center, in columns.
pub(super) const SWEEP_REACH: f32 = 6.0;

/// The line with a soft light breathing across it, left to right: each character keeps its own
/// style, its color drawn towards the accent where the light is. Without true colors the lit
/// characters are bold.
pub fn sweep(line: &Line<'static>, now_ms: i64, p: &Palette) -> Line<'static> {
    let n = line.width() as f32;
    let phase = now_ms.rem_euclid(SWEEP_MS) as f32 / SWEEP_MS as f32;
    let center = -SWEEP_REACH + phase * (n + 2.0 * SWEEP_REACH);
    let mut col = 0.0;
    let mut spans = Vec::new();
    for span in &line.spans {
        let base = span
            .style
            .fg
            .filter(|c| *c != ratatui::style::Color::Reset)
            .unwrap_or(p.fg);
        for c in span.content.chars() {
            let x = (col - center) / SWEEP_REACH;
            // A bell, not a cone: the light swells and fades instead of edging along.
            let light = if x.abs() < 1.0 {
                (1.0 + (x * std::f32::consts::PI).cos()) / 2.0
            } else {
                0.0
            };
            let style = match mix(base, p.accent, light) {
                Some(color) => span.style.fg(color),
                None if light > 0.5 => span.style.add_modifier(Modifier::BOLD),
                None => span.style,
            };
            spans.push(Span::styled(c.to_string(), style));
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as f32;
        }
    }
    Line::from(spans).style(line.style)
}

/// `b` mixed into `a` by `t` (0 to 1); None unless both are true colors.
pub fn mix(
    a: ratatui::style::Color,
    b: ratatui::style::Color,
    t: f32,
) -> Option<ratatui::style::Color> {
    use ratatui::style::Color;
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (a, b) else {
        return None;
    };
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Some(Color::Rgb(m(r0, r1), m(g0, g1), m(b0, b1)))
}

/// The docs of an item that have anything to focus.
pub(super) fn focusable_docs(item: &ViewItem) -> Vec<UiDocInfo> {
    neoui_docs(item)
        .into_iter()
        .filter(|d| d.focusable)
        .collect()
}

pub(super) fn render(
    item: &ViewItem,
    width: u16,
    key: &CacheKey,
    app: Option<&str>,
    cwd: Option<&str>,
    lit: bool,
) -> (Vec<Line<'static>>, Vec<ImageSlot>, Option<usize>) {
    let opts = Options {
        plain_code: !lit,
        split_diffs: key.split_diffs,
        open_edits: key.open_edit,
        expanded: key.expanded,
        show_thoughts: key.thoughts,
        focused_button: key.focused,
        hide_keys: false,
        prose_width: Some(PROSE_WIDTH),
        preview_lines: Some(PREVIEW_LINES),
        shown_above: false,
        theme: key.palette,
        images: key.images.as_ref().map(|i| ImageOptions {
            max_rows: (if key.expanded {
                IMAGE_PX_EXPANDED
            } else {
                IMAGE_PX
            } / if i.cell.1 > 0 { i.cell.1 } else { CELL_PX })
            .max(4),
            cell: i.cell,
            status: i.status.clone(),
            charts: i.charts,
        }),
        neoui: key.ui.clone().unwrap_or_default(),
        app: app.map(str::to_string),
        file_icons: key.icons,
        cwd: cwd.map(str::to_string),
        // The chat keeps what it drew: the countdown is the panel's, drawn every second.
        now_ms: None,
    };
    if key.focused.is_none() && !key.linked {
        let out = render_item_images(item, width, &opts);
        return (out.lines, out.images, out.focus_line);
    }
    // The prompt being answered, and the item the terminal panel is at, get a bar in the margin.
    let inner = width.saturating_sub(2).max(1);
    let out = render_item_images(item, inner, &opts);
    let lines = out
        .lines
        .into_iter()
        .map(|line| barred(line, Some(key.marker)))
        .collect();
    let mut slots = out.images;
    for slot in &mut slots {
        slot.x += 2;
    }
    (lines, slots, out.focus_line)
}

/// A line of the prompt being answered: the bar in the margin first. None where something
/// else draws the bar (the input box's).
pub fn barred(line: Line<'static>, marker: Option<Style>) -> Line<'static> {
    let Some(marker) = marker else {
        return line;
    };
    let mut spans = vec![Span::styled("▎ ", marker)];
    spans.extend(line.spans);
    Line::from(spans)
}

/// How the chat draws, for a prompt drawn in the input area to look the same: an edit asked
/// about reads like the edit once made.
#[derive(Debug, Clone, Copy)]
pub struct PanelLook<'a> {
    pub palette: Palette,
    pub icons: bool,
    pub split_diffs: bool,
    /// Paths in it are shown relative to it.
    pub cwd: Option<&'a str>,
    /// The chat shows the card in full (Ctrl-O): the panel's cut says so.
    pub shown_above: bool,
    /// The wall clock: a deadline on a button counts down.
    pub now_ms: i64,
}

/// The prompt as the input area shows it: the item with the n-th button focused and at most
/// `preview` lines of any diff or code, each line with the bar in the margin (none without a
/// marker). `hide_keys` draws the buttons without their keys.
pub fn panel_lines(
    item: &ViewItem,
    width: u16,
    focused: usize,
    preview: usize,
    hide_keys: bool,
    look: &PanelLook<'_>,
    marker: Option<Style>,
) -> Vec<Line<'static>> {
    let opts = Options {
        expanded: false,
        show_thoughts: false,
        focused_button: Some(focused),
        hide_keys,
        prose_width: Some(PROSE_WIDTH),
        preview_lines: Some(preview),
        shown_above: look.shown_above,
        theme: look.palette,
        images: None,
        neoui: HashMap::new(),
        app: None,
        file_icons: look.icons,
        cwd: look.cwd.map(str::to_string),
        plain_code: false,
        split_diffs: look.split_diffs,
        open_edits: false,
        now_ms: Some(look.now_ms),
    };
    let bar = if marker.is_some() { 2 } else { 0 };
    neo_renderer_ratatui::render_item(item, width.saturating_sub(bar).max(1), &opts)
        .into_iter()
        .map(|line| barred(line, marker))
        .collect()
}

/// The one-line input for the answer's note: `note › text` with a cursor, or a hint while empty.
/// A long note shows its end.
pub fn note_line(text: &str, width: usize, accent: Style) -> Line<'static> {
    input_line(
        "note › ",
        "For the agent · Enter Send · Esc Cancel",
        text,
        width,
        accent,
    )
}

/// The one-line filter of a long choice: `filter › text` with a cursor, or a hint while empty.
pub fn filter_line(text: &str, width: usize, accent: Style) -> Line<'static> {
    input_line("filter › ", "type to find an option", text, width, accent)
}

pub(super) fn input_line(
    lead: &str,
    hint: &str,
    text: &str,
    width: usize,
    accent: Style,
) -> Line<'static> {
    let mut spans = vec![Span::styled(lead.to_string(), accent)];
    if text.is_empty() {
        spans.push(Span::styled(
            " ",
            Style::new().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(
            hint.to_string(),
            Style::new().add_modifier(Modifier::DIM),
        ));
    } else {
        let room = width.saturating_sub(lead.chars().count() + 1).max(1);
        let chars: Vec<char> = text.chars().collect();
        let shown: String = if chars.len() > room {
            std::iter::once('…')
                .chain(chars[chars.len() - (room - 1)..].iter().copied())
                .collect()
        } else {
            text.to_string()
        };
        spans.push(Span::raw(shown));
        spans.push(Span::styled(
            " ",
            Style::new().add_modifier(Modifier::REVERSED),
        ));
    }
    Line::from(spans)
}

/// The line a prompt leaves in the chat while it is answered below the chat.
pub(super) fn panel_stub(item: &ViewItem, opts: &ChatOpts) -> Line<'static> {
    let label = item.label.as_deref().unwrap_or("a question");
    Line::from(vec![
        Span::styled(label.to_string(), Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" · answer below", Style::new().fg(opts.palette.muted)),
    ])
}
