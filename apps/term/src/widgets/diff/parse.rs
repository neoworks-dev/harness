use super::*;

/// Lines with their meaning, and the number of added and removed lines.
pub(crate) fn parse_diff(text: &str) -> (Vec<DiffLine>, usize, usize) {
    let mut lines = Vec::new();
    let (mut adds, mut dels) = (0, 0);
    let mut in_hunk = false;
    for raw in text.lines().take(MAX_LINES) {
        let kind = if raw.starts_with("diff ") {
            in_hunk = false;
            Kind::Header
        } else if raw.starts_with("@@") {
            in_hunk = true;
            Kind::Hunk
        } else if !in_hunk || raw.starts_with('\\') {
            Kind::Header
        } else if raw.starts_with('+') {
            adds += 1;
            Kind::Add
        } else if raw.starts_with('-') {
            dels += 1;
            Kind::Del
        } else {
            Kind::Context
        };
        lines.push(DiffLine {
            text: clean_line(raw),
            kind,
        });
    }
    if text.lines().nth(MAX_LINES).is_some() {
        lines.push(DiffLine {
            text: format!("… cut after {MAX_LINES} lines"),
            kind: Kind::Note,
        });
    }
    (lines, adds, dels)
}

/// The line number of every unified line, counted from the hunk headers: the new file's for
/// context and additions, the old file's for removals.
pub(super) fn line_numbers(lines: &[DiffLine]) -> Vec<Option<usize>> {
    let (mut old, mut new) = (1, 1);
    lines
        .iter()
        .map(|l| match l.kind {
            Kind::Hunk => {
                (old, new) = hunk_start(&l.text);
                None
            }
            Kind::Context => {
                old += 1;
                new += 1;
                Some(new - 1)
            }
            Kind::Add => {
                new += 1;
                Some(new - 1)
            }
            Kind::Del => {
                old += 1;
                Some(old - 1)
            }
            Kind::Header | Kind::Note => None,
        })
        .collect()
}

/// The old and new first line numbers of `@@ -a,b +c,d @@`.
pub(super) fn hunk_start(header: &str) -> (usize, usize) {
    let mut nums = header
        .split_whitespace()
        .filter(|t| t.starts_with(['-', '+']))
        .map(|t| t[1..].split(',').next().and_then(|n| n.parse().ok()));
    (
        nums.next().flatten().unwrap_or(1),
        nums.next().flatten().unwrap_or(1),
    )
}

/// Which run the last diff line belongs to (a `\ No newline` line refers to it).
#[derive(Clone, Copy)]
pub(super) enum Last {
    None,
    Del,
    Add,
    Ctx,
}

/// Pairs the pending removals and additions row by row.
pub(super) fn flush(rows: &mut Vec<Row>, dels: &mut Vec<Side>, adds: &mut Vec<Side>) {
    let (dels, adds) = (std::mem::take(dels), std::mem::take(adds));
    let n = dels.len().max(adds.len());
    let (mut dels, mut adds) = (dels.into_iter(), adds.into_iter());
    for _ in 0..n {
        let (mut left, mut right) = (dels.next(), adds.next());
        if let (Some(l), Some(r)) = (&mut left, &mut right) {
            (l.hl, r.hl) = intra(&l.text, &r.text);
        }
        rows.push(Row::Line {
            left,
            right,
            changed: true,
        });
    }
}

/// The unified diff as aligned rows: context on both sides; in a hunk each run of removals is
/// paired row by row with the run of additions after it, the shorter side left blank.
pub(crate) fn align(text: &str) -> Aligned {
    let mut a = Aligned::default();
    let (mut old_no, mut new_no) = (1, 1);
    let mut in_hunk = false;
    let (mut dels, mut adds) = (Vec::new(), Vec::new());
    let mut last = Last::None;
    let side = |no: usize, text: &str| Side {
        no,
        text: clean_line(text),
        hl: None,
        no_eol: false,
    };
    for raw in text.lines().take(MAX_LINES) {
        if raw.starts_with("diff ") {
            flush(&mut a.rows, &mut dels, &mut adds);
            in_hunk = false;
        } else if raw.starts_with("@@") {
            flush(&mut a.rows, &mut dels, &mut adds);
            in_hunk = true;
            last = Last::None;
            (old_no, new_no) = hunk_start(raw);
            a.hunks.push(a.rows.len());
            a.rows.push(Row::Hunk(clean_line(raw)));
        } else if !in_hunk {
            a.new_file |= raw.starts_with("new file mode");
            a.deleted |= raw.starts_with("deleted file mode");
        } else if raw.starts_with('\\') {
            let marked: Vec<&mut Side> = match last {
                Last::Del => dels.last_mut().into_iter().collect(),
                Last::Add => adds.last_mut().into_iter().collect(),
                Last::Ctx => match a.rows.last_mut() {
                    Some(Row::Line { left, right, .. }) => {
                        left.iter_mut().chain(right.iter_mut()).collect()
                    }
                    _ => Vec::new(),
                },
                Last::None => Vec::new(),
            };
            for s in marked {
                s.no_eol = true;
            }
        } else if let Some(rest) = raw.strip_prefix('-') {
            if !adds.is_empty() {
                flush(&mut a.rows, &mut dels, &mut adds);
            }
            dels.push(side(old_no, rest));
            old_no += 1;
            last = Last::Del;
        } else if let Some(rest) = raw.strip_prefix('+') {
            adds.push(side(new_no, rest));
            new_no += 1;
            last = Last::Add;
        } else {
            flush(&mut a.rows, &mut dels, &mut adds);
            let rest = raw.strip_prefix(' ').unwrap_or(raw);
            a.rows.push(Row::Line {
                left: Some(side(old_no, rest)),
                right: Some(side(new_no, rest)),
                changed: false,
            });
            old_no += 1;
            new_no += 1;
            last = Last::Ctx;
        }
    }
    flush(&mut a.rows, &mut dels, &mut adds);
    if text.lines().nth(MAX_LINES).is_some() {
        a.rows
            .push(Row::Note(format!("… cut after {MAX_LINES} lines")));
    }
    let mut most = 0;
    for row in &a.rows {
        if let Row::Line { left, right, .. } = row {
            for s in left.iter().chain(right) {
                most = most.max(s.no);
                let eol = if s.no_eol { width(NO_EOL) } else { 0 };
                a.max_w = a.max_w.max(width(&s.text) + eol);
            }
        }
    }
    a.gutter = most.to_string().len().max(3);
    a
}

pub(super) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The parts that differ between a removed and the added line that replaces it, as char ranges:
/// what is left after the common start and end, widened to whole words. None for a side when
/// nothing of it remains or all of it does.
pub(super) fn intra(old: &str, new: &str) -> (Option<Range>, Option<Range>) {
    let (a, b): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let most = a.len().min(b.len());
    let mut p = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut s = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(most - p)
        .take_while(|(x, y)| x == y)
        .count();
    while p > 0
        && is_word(a[p - 1])
        && (a.get(p).is_some_and(|&c| is_word(c)) || b.get(p).is_some_and(|&c| is_word(c)))
    {
        p -= 1;
    }
    while s > 0 && is_word(a[a.len() - s]) {
        let (ia, ib) = (a.len() - s, b.len() - s);
        if (ia > 0 && is_word(a[ia - 1])) || (ib > 0 && is_word(b[ib - 1])) {
            s -= 1;
        } else {
            break;
        }
    }
    let range = |len: usize| {
        let r = (p, len - s);
        (r.0 < r.1 && r != (0, len)).then_some(r)
    };
    (range(a.len()), range(b.len()))
}
