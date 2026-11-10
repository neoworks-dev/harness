//! The detail of one issue or pull request as lines to scroll: title, facts, description,
//! conversation, and for pull requests the reviews, checks and changed files.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::md;
use super::model::{CheckState, Detail, Kind, Reaction, State, emoji_of, short_age};
use crate::style::Styles;
use crate::util::{fit_line, truncate, width};

/// The color of a state word.
pub fn state_style(state: State, st: &Styles) -> Style {
    match state {
        State::Open => st.success,
        State::Closed => st.danger,
        State::Merged => st.accent,
        State::Draft => st.muted,
    }
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// A section title with a rule after it.
fn heading(text: &str, w: usize, st: &Styles) -> Line<'static> {
    let title = truncate(text, w);
    let room = w.saturating_sub(width(&title));
    let mut spans = vec![Span::styled(title, bold())];
    if room >= 2 {
        spans.push(Span::styled(
            format!(" {}", "─".repeat(room - 1)),
            st.border,
        ));
    }
    Line::from(spans)
}

/// Markdown, with a bar down the left side when it is somebody's comment.
fn gutter(text: &str, w: usize, st: &Styles, bar: bool) -> Vec<Line<'static>> {
    if !bar {
        return body(text, w, 0, st);
    }
    body(text, w.saturating_sub(2).max(1), 0, st)
        .into_iter()
        .map(|mut l| {
            l.spans.insert(0, Span::styled("▎ ", st.border));
            l
        })
        .collect()
}

/// Labels as chips, wrapped to the width.
fn chips(
    labels: &[String],
    w: usize,
    st: &Styles,
    colors: &super::view::Colors,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for label in labels {
        let chip = format!(" {} ", truncate(label, w.saturating_sub(2).max(1)));
        let need = width(&chip) + usize::from(used > 0);
        if used > 0 && used + need > w {
            out.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }
        if used > 0 {
            spans.push(Span::raw(" "));
            used += 1;
        }
        used += width(&chip);
        spans.push(Span::styled(
            chip,
            super::view::chip_style(colors.get(label).copied(), st),
        ));
    }
    if !spans.is_empty() {
        out.push(Line::from(spans));
    }
    out
}

/// "alice · 3h ago".
fn byline(author: &str, extra: &[String], at_ms: i64, now_ms: i64, st: &Styles) -> Line<'static> {
    let mut spans = vec![Span::styled(
        author.to_string(),
        st.accent.add_modifier(Modifier::BOLD),
    )];
    let mut rest: Vec<String> = extra.to_vec();
    let age = short_age(at_ms, now_ms);
    if !age.is_empty() {
        rest.push(format!("{age} ago"));
    }
    if !rest.is_empty() {
        spans.push(Span::styled(format!(" · {}", rest.join(" · ")), st.muted));
    }
    Line::from(spans)
}

/// The lines on the raised background across `w`, a space in from the left; as they are when
/// the theme paints none.
fn block(lines: Vec<Line<'static>>, w: usize, st: &Styles) -> Vec<Line<'static>> {
    if st.raised == Color::Reset {
        return lines;
    }
    lines
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw(" "));
            super::view::fill(&mut line, w, st.raised);
            line
        })
        .collect()
}

fn body(text: &str, w: usize, indent: usize, st: &Styles) -> Vec<Line<'static>> {
    let lines = md::render(text, w, indent, st);
    if lines.is_empty() {
        vec![Line::from(Span::styled(
            format!("{}(empty)", " ".repeat(indent)),
            st.muted,
        ))]
    } else {
        lines
    }
}

/// Cells wide an avatar is drawn (a cell is about twice as tall as wide: one row, square).
pub const AVATAR_COLS: u16 = 2;

/// Where an avatar goes in the lines: its line, its column and whose it is.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Avatar {
    pub line: usize,
    pub x: u16,
    pub login: String,
}

/// The picture of somebody on GitHub, small.
pub(super) fn avatar_url(login: &str) -> String {
    format!("https://avatars.githubusercontent.com/{login}?size=64")
}

/// What [`build`] makes.
pub(super) struct Built {
    pub lines: Vec<Line<'static>>,
    pub avatars: Vec<Avatar>,
    /// The first line of each comment.
    pub comments: Vec<usize>,
}

/// The reactions as a line: `👍 2  ❤️ 1`, the ones the user gave in the accent.
fn reactions_line(reactions: &[Reaction], st: &Styles) -> Option<Line<'static>> {
    if reactions.is_empty() {
        return None;
    }
    let mut spans = Vec::new();
    for r in reactions {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        let style = if r.mine { st.accent } else { st.muted };
        spans.push(Span::styled(
            format!("{} {}", emoji_of(&r.content), r.count),
            style,
        ));
    }
    Some(Line::from(spans))
}

/// What is set on the item besides its labels, as a muted paragraph: assignees, milestone,
/// type, the organization's fields, projects and relations. Nothing set, no lines.
fn setup(d: &Detail, fields: &[(String, String)], w: usize, st: &Styles) -> Vec<Line<'static>> {
    let mut parts: Vec<String> = Vec::new();
    let list = |label: &str, items: &[String]| {
        (!items.is_empty()).then(|| format!("{label} {}", items.join(", ")))
    };
    parts.extend(list("assigned to", &d.assignees));
    parts.extend(list("reviewers", &d.reviewers));
    if !d.milestone.is_empty() {
        parts.push(format!("milestone {}", d.milestone));
    }
    if !d.issue_type.is_empty() {
        parts.push(format!("type {}", d.issue_type));
    }
    parts.extend(fields.iter().map(|(n, v)| format!("{n} {v}")));
    parts.extend(list("projects", &d.projects));
    let numbers = |label: &str, items: &[u64]| {
        (!items.is_empty()).then(|| {
            let n: Vec<String> = items.iter().map(|n| format!("#{n}")).collect();
            format!("{label} {}", n.join(", "))
        })
    };
    if d.parent > 0 {
        parts.push(format!("parent #{}", d.parent));
    }
    parts.extend(numbers("blocked by", &d.blocked_by));
    parts.extend(numbers("blocking", &d.blocking));
    if parts.is_empty() {
        return Vec::new();
    }
    md::paragraph(&parts.join(" · "), w, st.muted)
}

/// What the lines of a detail depend on besides the item and the width.
#[derive(Clone, Copy)]
pub(super) struct With<'a> {
    /// The colors of the labels.
    pub colors: &'a super::view::Colors,
    /// The terminal draws avatars: room is left for them.
    pub pictures: bool,
    /// The comment the keys act on, marked.
    pub sel: Option<usize>,
    /// What the organization's issue fields are set to.
    pub fields: &'a [(String, String)],
}

/// The lines of the detail, and where the avatars of the people in it go when `pictures` (the
/// terminal can draw them): the creator's beside the state, each commenter's before their name.
/// The comment `sel` is marked with the accent.
pub(super) fn build(d: &Detail, w: usize, now_ms: i64, st: &Styles, with: &With) -> Built {
    let With {
        colors,
        pictures,
        sel,
        fields,
    } = *with;
    let w = w.max(1);
    let mut avatars = Vec::new();
    let mut comment_lines = Vec::new();
    // The content sits on a background of its own, a space in from each side, so the headings
    // and the gaps between the parts stand out from it.
    let panel = st.raised != Color::Reset;
    let inner = if panel { w.saturating_sub(2).max(1) } else { w };
    let pad = u16::from(panel);
    let mut head: Vec<Line<'static>> = md::paragraph(&d.title, inner, bold());
    if pictures && !d.author.is_empty() {
        // After the state word and its space.
        let badge = width(&format!(" {} ", d.state.word())) as u16;
        avatars.push(Avatar {
            line: head.len(),
            x: pad + badge + 1,
            login: d.author.clone(),
        });
    }
    head.push(fit_line(facts(d, now_ms, st, pictures), inner));
    if !d.labels.is_empty() {
        head.extend(chips(&d.labels, inner, st, colors));
    }
    head.extend(setup(d, fields, inner, st));
    if d.kind == Kind::Pr {
        head.extend(pr_facts(d, inner, st));
    }
    let mut out = block(head, w, st);
    out.push(Line::default());
    out.push(heading("Description", w, st));
    if d.body.trim().is_empty() {
        out.push(Line::from(Span::styled("no description", st.muted)));
    } else {
        let mut lines = gutter(&d.body, inner, st, false);
        lines.extend(reactions_line(&d.reactions, st));
        out.extend(block(lines, w, st));
    }
    if d.body.trim().is_empty() && !d.reactions.is_empty() {
        out.extend(block(reactions_line(&d.reactions, st).into_iter().collect(), w, st));
    }

    out.push(Line::default());
    out.push(heading(&format!("Comments ({})", d.comments.len()), w, st));
    if d.comments.is_empty() {
        out.push(Line::from(Span::styled("no comments", st.muted)));
    }
    for (i, c) in d.comments.iter().enumerate() {
        comment_lines.push(out.len());
        let mut by = byline(&c.author, &[], c.at_ms, now_ms, st);
        if pictures {
            by.spans
                .insert(0, Span::raw(" ".repeat(usize::from(AVATAR_COLS) + 1)));
            avatars.push(Avatar {
                line: out.len(),
                x: pad,
                login: c.author.clone(),
            });
        }
        let mut lines = vec![fit_line(by, inner)];
        let mut text = gutter(&c.body, inner, st, true);
        if let Some(reactions) = reactions_line(&c.reactions, st) {
            let mut reactions = reactions;
            reactions.spans.insert(0, Span::styled("▎ ", st.border));
            text.push(reactions);
        }
        if sel == Some(i) {
            // The comment the keys act on: its bar in the accent.
            for line in &mut text {
                if let Some(bar) = line.spans.first_mut() {
                    bar.style = st.accent;
                }
            }
        }
        lines.extend(text);
        out.extend(block(lines, w, st));
        out.push(Line::default());
    }
    if d.kind == Kind::Pr {
        out.extend(pr_sections(d, w, now_ms, st));
    }
    while out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    Built {
        lines: out.into_iter().map(|l| fit_line(l, w)).collect(),
        avatars,
        comments: comment_lines,
    }
}

/// "open  alice · opened 3d ago · updated 2h ago".
fn facts(d: &Detail, now_ms: i64, st: &Styles, pictures: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!(" {} ", d.state.word()),
            state_style(d.state, st).add_modifier(Modifier::BOLD | Modifier::REVERSED),
        ),
        Span::raw(" "),
    ];
    if pictures {
        // Room for the author's avatar, which the view draws over it.
        spans.push(Span::raw(" ".repeat(usize::from(AVATAR_COLS) + 1)));
    }
    spans.push(Span::styled(d.author.clone(), st.accent));
    let mut when = Vec::new();
    if d.state != State::Open && !d.state_reason.is_empty() {
        when.push(d.state_reason.clone());
    }
    if d.created_ms > 0 {
        when.push(format!("opened {} ago", short_age(d.created_ms, now_ms)));
    }
    if d.updated_ms > d.created_ms {
        when.push(format!("updated {} ago", short_age(d.updated_ms, now_ms)));
    }
    if !when.is_empty() {
        spans.push(Span::styled(format!(" · {}", when.join(" · ")), st.muted));
    }
    Line::from(spans)
}

/// Branches, size, review decision and the checks in one line each.
fn pr_facts(d: &Detail, w: usize, st: &Styles) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut line = Vec::new();
    if !d.head.is_empty() {
        line.push(Span::raw(format!("{} → {}  ", d.head, d.base)));
    }
    line.push(Span::styled(format!("+{}", d.additions), st.success));
    line.push(Span::raw(" "));
    line.push(Span::styled(format!("−{}", d.deletions), st.danger));
    line.push(Span::styled(
        format!(" in {} files", d.files.len()),
        st.muted,
    ));
    out.push(fit_line(Line::from(line), w));
    let mut status = Vec::new();
    if !d.decision.is_empty() {
        let style = match d.decision.as_str() {
            "APPROVED" => st.success,
            "CHANGES_REQUESTED" => st.danger,
            _ => st.muted,
        };
        status.push(Span::styled(
            format!("review: {}", d.decision.to_lowercase().replace('_', " ")),
            style,
        ));
    }
    if !d.checks.is_empty() {
        let c = d.check_counts();
        if !status.is_empty() {
            status.push(Span::raw("  "));
        }
        status.push(Span::raw("checks: "));
        status.push(Span::styled(format!("✓{}", c.pass), st.success));
        if c.fail > 0 {
            status.push(Span::styled(format!(" ✗{}", c.fail), st.danger));
        }
        if c.pending > 0 {
            status.push(Span::styled(format!(" ◌{}", c.pending), st.muted));
        }
        if c.skipped > 0 {
            status.push(Span::styled(format!(" –{}", c.skipped), st.muted));
        }
    }
    if !status.is_empty() {
        out.push(fit_line(Line::from(status), w));
    }
    out
}

/// Reviews, comments on the diff, checks and files.
fn pr_sections(d: &Detail, w: usize, now_ms: i64, st: &Styles) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    // A review that only says "commented" with no text is noise.
    let reviews: Vec<_> = d
        .reviews
        .iter()
        .filter(|r| !(r.state == "COMMENTED" && r.body.trim().is_empty()))
        .collect();
    if !reviews.is_empty() {
        out.push(heading(&format!("Reviews ({})", reviews.len()), w, st));
        for r in reviews {
            let (word, style) = match r.state.as_str() {
                "APPROVED" => ("approved", st.success),
                "CHANGES_REQUESTED" => ("requested changes", st.danger),
                "DISMISSED" => ("dismissed", st.muted),
                "PENDING" => ("pending", st.muted),
                _ => ("commented", st.muted),
            };
            let mut head = byline(&r.author, &[], r.at_ms, now_ms, st);
            head.spans
                .insert(1, Span::styled(format!(" {word}"), style));
            out.push(fit_line(head, w));
            if !r.body.trim().is_empty() {
                out.extend(body(&r.body, w, 2, st));
            }
            out.push(Line::default());
        }
    }
    if !d.review_comments.is_empty() {
        out.push(heading(
            &format!("Comments on the diff ({})", d.review_comments.len()),
            w,
            st,
        ));
        for c in &d.review_comments {
            let place = match c.line {
                Some(l) => format!("{}:{l}", c.path),
                None => c.path.clone(),
            };
            let indent = if c.reply { 4 } else { 2 };
            out.push(fit_line(
                byline(&c.author, &[place], c.at_ms, now_ms, st),
                w,
            ));
            out.extend(body(&c.body, w, indent, st));
            out.push(Line::default());
        }
    }
    if !d.checks.is_empty() {
        out.push(heading(&format!("Checks ({})", d.checks.len()), w, st));
        let c = d.check_counts();
        for check in d
            .checks
            .iter()
            .filter(|c| matches!(c.state, CheckState::Fail | CheckState::Pending))
        {
            let (mark, style) = if check.state == CheckState::Fail {
                ("✗", st.danger)
            } else {
                ("◌", st.muted)
            };
            out.push(fit_line(
                Line::from(vec![
                    Span::styled(format!("{mark} "), style),
                    Span::raw(check.name.clone()),
                ]),
                w,
            ));
        }
        let mut tail = vec![Span::styled(format!("✓ {} passed", c.pass), st.success)];
        if c.skipped > 0 {
            tail.push(Span::styled(format!("  – {} skipped", c.skipped), st.muted));
        }
        out.push(Line::from(tail));
        out.push(Line::default());
    }
    if !d.files.is_empty() {
        out.push(heading(&format!("Files ({})", d.files.len()), w, st));
        for f in &d.files {
            out.push(fit_line(
                Line::from(vec![
                    Span::styled(format!("{:>5}", format!("+{}", f.additions)), st.success),
                    Span::styled(format!("{:>6} ", format!("−{}", f.deletions)), st.danger),
                    Span::raw(f.path.clone()),
                ]),
                w,
            ));
        }
    }
    out
}
