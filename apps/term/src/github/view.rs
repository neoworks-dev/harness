//! Drawing the GitHub view: the list (tabs, filter line, rows, key hints) and the detail.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::doc::{self, state_style};
use super::gh::GhError;
use super::model::{Kind, Summary, short_age};
use super::{Doc, Github, Mode, Repo};
use crate::style::Styles;
use crate::util::{fit_line, split_line, truncate, width};
use crate::widgets::DiffView;

/// Columns of a row that are fixed: marker, number, state.
const STATE_W: usize = 6;
/// A title is not worth less than this; narrower and the extra columns go.
const TITLE_MIN: usize = 24;
const LABELS_MAX: usize = 24;
const AUTHOR_MAX: usize = 16;

pub(super) fn draw(g: &mut Github, frame: &mut Frame, area: Rect, now_ms: i64, st: &Styles) {
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
    // The body first: it learns how many rows there are, which the head reports on.
    let body = match g.mode {
        Mode::List => list_body(g, w, body_h, now_ms, st),
        Mode::Detail(_) => detail_body(g, w, body_h, now_ms, st),
    };
    let head = match g.mode {
        Mode::List => list_head(g, w, st),
        Mode::Detail(_) => detail_head(g, w, st),
    };
    let mut lines = head;
    let head_h = lines.len();
    lines.extend(body);
    lines.resize(head_h + body_h, Line::default());
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
    if cursor && let Some(text) = &g.typing {
        let x = (2 + width(text)).min(w - 1) as u16;
        frame.set_cursor_position(Position::new(area.x + x, area.y + 1));
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
    // Keep the selection in view.
    let rows = h.max(1);
    data.selected = data.selected.min(data.rows.len() - 1);
    if data.selected < data.top {
        data.top = data.selected;
    } else if data.selected >= data.top + rows {
        data.top = data.selected + 1 - rows;
    }
    data.top = data.top.min(data.rows.len().saturating_sub(rows));
    let layout = Columns::new(&data.rows, w);
    data.rows
        .iter()
        .enumerate()
        .skip(data.top)
        .take(rows)
        .map(|(i, s)| layout.row(s, i == data.selected, now_ms, st))
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

/// Which optional columns fit at this width, and how wide each is.
pub(super) struct Columns {
    number: usize,
    labels: usize,
    author: usize,
    comments: usize,
    age: usize,
    width: usize,
}

impl Columns {
    pub(super) fn new(rows: &[Summary], width: usize) -> Columns {
        let number = rows
            .iter()
            .map(|r| r.number.to_string().len() + 1)
            .max()
            .unwrap_or(2);
        let labels = rows
            .iter()
            .map(|r| crate::util::width(&r.labels.join(", ")))
            .max()
            .unwrap_or(0)
            .min(LABELS_MAX);
        let author = rows
            .iter()
            .map(|r| crate::util::width(&r.author))
            .max()
            .unwrap_or(0)
            .min(AUTHOR_MAX);
        let comments = rows
            .iter()
            .map(|r| format!("✎{}", r.comments).chars().count())
            .max()
            .unwrap_or(2);
        let mut c = Columns {
            number,
            labels,
            author,
            comments,
            age: 4,
            width,
        };
        // Drop columns, labels first, until the title has room.
        let fixed = 2 + number + 1 + STATE_W + 1;
        let mut level: u8 = 3;
        while level > 0 && fixed + c.right(level) + TITLE_MIN > width {
            level -= 1;
        }
        if fixed + c.right(0) + TITLE_MIN > width {
            c.age = 0;
        }
        if level < 3 {
            c.labels = 0;
        }
        if level < 2 {
            c.author = 0;
        }
        if level < 1 {
            c.comments = 0;
        }
        c
    }

    /// Width of the columns right of the title at a level (3: all, 0: only the age).
    fn right(&self, level: u8) -> usize {
        let mut w = 2 + self.age;
        if level >= 1 {
            w += 2 + self.comments;
        }
        if level >= 2 {
            w += 2 + self.author;
        }
        if level >= 3 && self.labels > 0 {
            w += 2 + self.labels;
        }
        w
    }

    pub(super) fn row(
        &self,
        s: &Summary,
        selected: bool,
        now_ms: i64,
        st: &Styles,
    ) -> Line<'static> {
        let fixed = 2 + self.number + 1 + STATE_W + 1;
        let right = self.right_width();
        let title_w = self.width.saturating_sub(fixed + right).max(1);
        let title_style = if selected {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let mut spans = vec![
            Span::styled(if selected { "› " } else { "  " }, st.accent),
            Span::styled(
                format!("{:>w$} ", format!("#{}", s.number), w = self.number),
                st.muted,
            ),
            Span::styled(
                format!("{:<w$} ", s.state.word(), w = STATE_W),
                state_style(s.state, st),
            ),
            Span::styled(pad(&truncate(&s.title, title_w), title_w), title_style),
        ];
        if self.labels > 0 {
            spans.push(Span::styled(
                format!(
                    "  {}",
                    pad(&truncate(&s.labels.join(", "), self.labels), self.labels)
                ),
                st.muted,
            ));
        }
        if self.author > 0 {
            spans.push(Span::raw(format!(
                "  {}",
                pad(&truncate(&s.author, self.author), self.author)
            )));
        }
        if self.comments > 0 {
            let c = if s.comments > 0 {
                format!("✎{}", s.comments)
            } else {
                String::new()
            };
            spans.push(Span::styled(
                format!("  {:>w$}", c, w = self.comments),
                st.muted,
            ));
        }
        if self.age > 0 {
            spans.push(Span::styled(
                format!("  {:>w$}", short_age(s.updated_ms, now_ms), w = self.age),
                st.muted,
            ));
        }
        Line::from(spans)
    }

    fn right_width(&self) -> usize {
        let mut w = 0;
        if self.labels > 0 {
            w += 2 + self.labels;
        }
        if self.author > 0 {
            w += 2 + self.author;
        }
        if self.comments > 0 {
            w += 2 + self.comments;
        }
        if self.age > 0 {
            w += 2 + self.age;
        }
        w
    }
}

fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(width(s))))
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
) -> Vec<Line<'static>> {
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
    let stale = !v
        .doc
        .as_ref()
        .is_some_and(|d| d.width == w && d.version == data.version && d.minute == minute);
    if stale {
        v.doc = Some(Doc {
            width: w,
            version: data.version,
            minute,
            lines: doc::build(detail, w, now_ms, st),
        });
    }
    let lines = &v.doc.as_ref().expect("built above").lines;
    v.scroll = v.scroll.min(lines.len().saturating_sub(v.rows));
    lines.iter().skip(v.scroll).take(v.rows).cloned().collect()
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
        return hints(&["enter search", "esc cancel", "ctrl-u clear"], w, st);
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
        Mode::Detail(v) if v.kind == Kind::Pr => hints(
            &[
                "enter attach",
                "s send",
                "d diff",
                "↑↓ scroll",
                "r reload",
                "esc back",
                "q close",
            ],
            w,
            st,
        ),
        Mode::Detail(_) => hints(
            &[
                "enter attach",
                "s send",
                "↑↓ scroll",
                "r reload",
                "esc back",
                "q close",
            ],
            w,
            st,
        ),
    }
}
