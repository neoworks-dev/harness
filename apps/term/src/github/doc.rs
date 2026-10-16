//! The detail of one issue or pull request as lines to scroll: title, facts, description,
//! conversation, and for pull requests the reviews, checks and changed files.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::md;
use super::model::{CheckState, Detail, Kind, State, short_age};
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

pub fn build(d: &Detail, w: usize, now_ms: i64, st: &Styles) -> Vec<Line<'static>> {
    let w = w.max(1);
    let mut out: Vec<Line<'static>> = md::paragraph(&d.title, w, bold());
    out.push(fit_line(facts(d, now_ms, st), w));
    if !d.labels.is_empty() {
        out.extend(md::paragraph(
            &format!("labels: {}", d.labels.join(", ")),
            w,
            st.muted,
        ));
    }
    if d.kind == Kind::Pr {
        out.extend(pr_facts(d, w, st));
    }
    out.push(Line::default());
    if d.body.trim().is_empty() {
        out.push(Line::from(Span::styled("no description", st.muted)));
    } else {
        out.extend(md::render(&d.body, w, 0, st));
    }

    out.push(Line::default());
    out.push(heading(&format!("Comments ({})", d.comments.len()), w, st));
    if d.comments.is_empty() {
        out.push(Line::from(Span::styled("no comments", st.muted)));
    }
    for c in &d.comments {
        out.push(byline(&c.author, &[], c.at_ms, now_ms, st));
        out.extend(body(&c.body, w, 2, st));
        out.push(Line::default());
    }
    if d.kind == Kind::Pr {
        out.extend(pr_sections(d, w, now_ms, st));
    }
    while out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    out.into_iter().map(|l| fit_line(l, w)).collect()
}

/// "open  alice · opened 3d ago · updated 2h ago".
fn facts(d: &Detail, now_ms: i64, st: &Styles) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            d.state.word(),
            state_style(d.state, st).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(d.author.clone(), st.accent),
    ];
    let mut when = Vec::new();
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
