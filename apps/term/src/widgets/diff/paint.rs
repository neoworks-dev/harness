use super::*;

/// What the highlighting worker parses, for the unified lines and for the split rows, and the
/// grammar it starts with: the one for `path` (a pull request's diff: the path in each file's
/// `+++ b/` header). Old and new lines are parsed apart, each in order, and a hunk starts the
/// parse over.
pub(super) fn steps(
    path: Option<&str>,
    lines: &[DiffLine],
    aligned: &Aligned,
) -> (Option<&'static SyntaxReference>, Vec<Step>, Vec<Step>) {
    let header_path = |l: &DiffLine| l.text.strip_prefix("+++ b/").map(str::to_string);
    let first = path
        .map(str::to_string)
        .or_else(|| lines.iter().find_map(header_path));
    let grammar = first.as_deref().and_then(syntax_for);
    let mut current = grammar;
    let uni = lines
        .iter()
        .map(|l| {
            let body = l
                .text
                .get(l.text.chars().next().map_or(0, char::len_utf8)..)
                .unwrap_or("")
                .to_string();
            match l.kind {
                Kind::Header => match header_path(l).filter(|_| path.is_none()) {
                    Some(p) => {
                        current = syntax_for(&p);
                        Step::Reset(current)
                    }
                    None => Step::Skip,
                },
                Kind::Hunk => Step::Reset(current),
                Kind::Context => Step::Line {
                    old: Some(body.clone()),
                    new: Some(body),
                },
                Kind::Add => Step::Line {
                    old: None,
                    new: Some(body),
                },
                Kind::Del => Step::Line {
                    old: Some(body),
                    new: None,
                },
                Kind::Note => Step::Skip,
            }
        })
        .collect();
    let split = aligned
        .rows
        .iter()
        .map(|row| match row {
            Row::Hunk(_) => Step::Reset(grammar),
            Row::Line { left, right, .. } => Step::Line {
                old: left.as_ref().map(|s| s.text.clone()),
                new: right.as_ref().map(|s| s.text.clone()),
            },
            Row::Note(_) => Step::Skip,
        })
        .collect();
    (grammar, uni, split)
}

pub(super) fn with_bg(style: Style, bg: Option<Color>) -> Style {
    match bg {
        Some(c) => style.bg(c),
        None => style,
    }
}

/// `over` mixed into `base` by `pct` percent; None unless both are true colors.
pub(super) fn blend(base: Color, over: Color, pct: u32) -> Option<Color> {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (base, over) else {
        return None;
    };
    let mix = |a: u8, b: u8| ((u32::from(a) * (100 - pct) + u32::from(b) * pct) / 100) as u8;
    Some(Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1)))
}

/// `s` cut or padded to exactly `n` cells.
pub(super) fn fit(s: &str, n: usize) -> String {
    let mut out = truncate(s, n);
    out.push_str(&" ".repeat(n.saturating_sub(width(&out))));
    out
}

/// Cells a text takes as rows of `avail` cells when wrapped (at least one).
pub(super) fn wrapped_rows(cells: usize, avail: usize) -> usize {
    cells.div_ceil(avail.max(1)).max(1)
}

/// A line of `(char, style index)` as screen rows of exactly `avail` cells when `pad`: wrapped
/// at the edge when `wrap` (a wide character that doesn't fit goes to the next row), else one
/// row from cell `skip` on, cut with a `…`.
pub(super) fn fitted(
    chars: Vec<(char, usize)>,
    total: usize,
    skip: usize,
    avail: usize,
    styles: &[Style],
    pad: bool,
    wrap: bool,
) -> Vec<Vec<Span<'static>>> {
    if !wrap || avail == 0 {
        return vec![windowed(chars.into_iter(), total, skip, avail, styles, pad)];
    }
    let mut rows: Vec<Vec<(char, usize)>> = vec![Vec::new()];
    let mut used = 0;
    for (c, id) in chars {
        let w = c.width().unwrap_or(0);
        if used + w > avail && used > 0 {
            rows.push(Vec::new());
            used = 0;
        }
        rows.last_mut().expect("never empty").push((c, id));
        used += w;
    }
    rows.into_iter()
        .map(|row| {
            let w = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
            windowed(row.into_iter(), w, 0, avail, styles, pad)
        })
        .collect()
}

/// A column of a split row where its side has no more to show, `col` cells: the row's tint, or
/// the band when the side has no line at all.
pub(super) fn side_filler(
    present: bool,
    tone: Tone,
    col: usize,
    look: &Look,
) -> Vec<Span<'static>> {
    let bg = if present { look.bg(tone) } else { look.band };
    vec![Span::styled(" ".repeat(col), with_bg(Style::new(), bg))]
}

/// One column of a split row, each screen row exactly `col` cells: the line number, a sign, the
/// text (wrapped, or from cell `skip` on) with the cursor and selection on it; a blank filler
/// when this side has no line. `cols` is the gutter, the column's width, `skip` and `wrap`.
pub(super) fn side_rows(
    side: Option<&Side>,
    classes: &[Class],
    tone: Tone,
    cols: (usize, usize, usize, bool),
    marks: Marks,
    look: &Look,
) -> Vec<Vec<Span<'static>>> {
    let (gutter, col, skip, wrap) = cols;
    let Some(side) = side else {
        return vec![side_filler(false, tone, col, look)];
    };
    let base = look.base(tone);
    let bg = with_bg(Style::new(), look.bg(tone));
    let sign = match tone {
        Tone::Ctx => " ",
        Tone::Del => "-",
        Tone::Add => "+",
    };
    let avail = col.saturating_sub(gutter + 3);
    let hl = side.hl.filter(|_| tone != Tone::Ctx);
    // Highlighted code keeps the change's color as a background tint.
    let mut styles = vec![base, look.changed(tone), bg.patch(look.muted)];
    let mut chars: Vec<(char, usize)> = side
        .text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let changed = usize::from(hl.is_some_and(|(a, b)| (a..b).contains(&i)));
            let id = match look.code(classes.get(i).copied()) {
                Some(code) => {
                    let style = styles[changed].patch(code);
                    style_id(&mut styles, style)
                }
                None => changed,
            };
            (c, id)
        })
        .collect();
    let eol = side
        .no_eol
        .then_some(NO_EOL)
        .into_iter()
        .flat_map(str::chars);
    let extra = paint(&mut chars, &mut styles, marks, 0, look.selection);
    let total = width(&side.text) + extra + if side.no_eol { width(NO_EOL) } else { 0 };
    let chars = chars.into_iter().chain(eol.map(|c| (c, 2))).collect();
    fitted(chars, total, skip, avail, &styles, true, wrap)
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            // The number and the sign on the first row; under them the text goes on.
            let mut spans = if i == 0 {
                vec![
                    Span::styled(format!("{:>gutter$} ", side.no), bg.patch(look.muted)),
                    Span::styled(sign, base),
                    Span::styled(" ", bg),
                ]
            } else {
                vec![Span::styled(" ".repeat(gutter + 3), bg)]
            };
            spans.extend(text);
            spans
        })
        .collect()
}

/// Paints the selection and the cursor of `m` on a row's chars, whose text starts at char
/// `offset`; a cursor or selection past the end gets a blank after the text. The cells added.
pub(super) fn paint(
    chars: &mut Vec<(char, usize)>,
    styles: &mut Vec<Style>,
    m: Marks,
    offset: usize,
    selection: Style,
) -> usize {
    if m == Marks::default() {
        return 0;
    }
    let len = chars.len().saturating_sub(offset);
    let need = m
        .cursor
        .map_or(0, |c| c + 1)
        .max(m.sel.map_or(0, |(_, b)| b));
    let extra = usize::from(need > len);
    if extra == 1 {
        chars.push((' ', 0));
    }
    let end = len + extra;
    let mut over = |at: usize, patch: Style| {
        if let Some(ch) = chars.get_mut(offset + at) {
            let style = styles[ch.1].patch(patch);
            ch.1 = style_id(styles, style);
        }
    };
    if let Some((a, b)) = m.sel {
        for at in a..b.min(end) {
            over(at, selection);
        }
    }
    if let Some(c) = m.cursor {
        over(
            c.min(end.saturating_sub(1)),
            Style::new().add_modifier(Modifier::REVERSED),
        );
    }
    extra
}

/// The index of `style` in `styles`, added when it isn't there yet.
pub(super) fn style_id(styles: &mut Vec<Style>, style: Style) -> usize {
    match styles.iter().position(|s| *s == style) {
        Some(i) => i,
        None => {
            styles.push(style);
            styles.len() - 1
        }
    }
}

/// The cells `skip..skip + avail` of a line of `(char, style index)`, as spans. A wide
/// character cut by the left edge shows as blanks; when the line goes on past the right edge
/// the last cell is a `…`. Padded to `avail` cells with the first style when `pad`.
pub(super) fn windowed(
    chars: impl Iterator<Item = (char, usize)>,
    total: usize,
    skip: usize,
    avail: usize,
    styles: &[Style],
    pad: bool,
) -> Vec<Span<'static>> {
    if avail == 0 {
        return Vec::new();
    }
    let more = total > skip + avail;
    let limit = if more { avail - 1 } else { avail };
    let mut runs: Vec<(String, usize)> = Vec::new();
    let mut push = |s: &str, id: usize| match runs.last_mut() {
        Some((text, last)) if *last == id => text.push_str(s),
        _ => runs.push((s.to_string(), id)),
    };
    let (mut pos, mut used) = (0, 0);
    for (c, id) in chars {
        let w = c.width().unwrap_or(0);
        let start = pos;
        pos += w;
        if start < skip {
            if w > 0 && pos > skip {
                let n = (pos - skip).min(limit - used);
                push(&" ".repeat(n), id);
                used += n;
            }
            continue;
        }
        if used + w > limit {
            break;
        }
        push(c.encode_utf8(&mut [0; 4]), id);
        used += w;
    }
    if more {
        push("…", 2);
        used += 1;
    }
    if pad && used < avail {
        push(&" ".repeat(avail - used), 0);
    }
    runs.into_iter()
        .map(|(text, id)| Span::styled(text, styles[id]))
        .collect()
}
