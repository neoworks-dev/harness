use super::*;

impl Sessions {
    /// The sidebar's lines for a `w` x `h` area: a header, then the groups' headers and two rows
    /// per session.
    pub fn render(
        &mut self,
        w: usize,
        h: usize,
        look: &Look,
        styles: &Styles,
    ) -> Vec<Line<'static>> {
        let Look {
            current, focused, ..
        } = *look;
        // Which sessions fold depends on the shown one and the time.
        let minute = look.now_ms / 60_000;
        if current != self.current || minute != self.minute {
            self.current = current.to_string();
            self.minute = minute;
            self.build();
        }
        if w < 8 || h == 0 {
            return Vec::new();
        }
        let header_style = if focused {
            styles.accent.add_modifier(Modifier::BOLD)
        } else {
            styles.muted
        };
        let mut header = Line::from(vec![
            Span::raw(" "),
            Span::styled("Sessions", header_style),
            Span::styled(format!(" · {}", self.list.len()), styles.muted),
        ]);
        // How many wait, work and finished unseen, at the right edge when there is room.
        let tally = self.tally(look, styles);
        let tally_w: usize = tally.iter().map(|s| width(&s.content)).sum();
        if tally_w > 0 && header.width() + tally_w + 2 <= w {
            let pad = w - header.width() - tally_w - 1;
            header.spans.push(Span::raw(" ".repeat(pad)));
            header.spans.extend(tally);
        }
        let mut out = vec![header];
        // `+ New chat` under the header, where scrolling doesn't take it.
        if h >= 4 {
            let mut new = Line::from(vec![
                Span::raw("   "),
                Span::styled("+ New chat", styles.accent),
            ]);
            if focused && self.selected == Some(Pick::New) {
                let pad = w.saturating_sub(new.width());
                new.spans.push(Span::raw(" ".repeat(pad)));
                for span in &mut new.spans {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                }
            }
            out.push(new);
        }
        let rows = (h - out.len()).max(2);
        for entry in &mut self.entries {
            if let Entry::Session {
                index,
                child,
                wraps,
                ..
            } = entry
            {
                let info = &self.list[*index];
                *wraps = info.id == current
                    && split_title(&session_title(info), title_room(info, *child, w, look).0)
                        .1
                        .is_some();
            }
        }
        let shown = self.row_of(current);
        let anchor_pick = if focused {
            self.selected.as_ref()
        } else {
            Some(&shown)
        };
        let anchor = self.entries.iter().position(|e| {
            let pick = match e {
                Entry::Session { index, .. } => Pick::Session(self.list[*index].id.clone()),
                Entry::Older { group, .. } => Pick::Older(group.clone()),
                Entry::Cluster { repo, .. } => Pick::Cluster(repo.clone()),
                Entry::Group(_) => return false,
            };
            Some(&pick) == anchor_pick
        });
        if let Some(a) = anchor {
            self.scroll_to(a, rows);
        } else if anchor_pick == Some(&Pick::New) {
            self.top = 0;
        }
        let mut used = 0;
        for entry in self.entries.iter().skip(self.top) {
            if used + entry.height() > rows {
                break;
            }
            used += entry.height();
            match entry {
                Entry::Cluster { repo, name, folded } => {
                    // The repository's name, then a rule to the edge: it parts the projects.
                    // Folded, a `▸` before it and what it holds after it.
                    let mut tail = Vec::new();
                    if let Some(f) = folded {
                        tail.push(Span::styled(format!(" · {}", f.count), styles.muted));
                        if f.status != Status::Idle {
                            tail.push(Span::raw(" "));
                            tail.push(f.status.icon(look.spinner, styles));
                        }
                    }
                    let lead = if folded.is_some() { " ▸ " } else { " " };
                    let tail_w: usize = tail.iter().map(|s| width(&s.content)).sum();
                    let name = truncate(name, w.saturating_sub(width(lead) + tail_w + 2));
                    let rule =
                        "─".repeat(w.saturating_sub(width(lead) + width(&name) + tail_w + 1));
                    let mut line = Line::from(Span::styled(
                        format!("{lead}{name}"),
                        Style::new().add_modifier(Modifier::BOLD),
                    ));
                    line.spans.extend(tail);
                    line.spans.push(Span::raw(" "));
                    line.spans.push(Span::styled(rule, styles.border));
                    if focused && self.selected.as_ref() == Some(&Pick::Cluster(repo.clone())) {
                        for span in &mut line.spans {
                            span.style = span.style.add_modifier(Modifier::REVERSED);
                        }
                    }
                    out.push(line);
                }
                Entry::Group(name) => {
                    out.push(Line::from(vec![
                        Span::styled("   worktree ", styles.muted),
                        Span::raw(truncate(name, w.saturating_sub(12))),
                    ]));
                }
                Entry::Older { group, count, open } => {
                    let selected =
                        focused && self.selected.as_ref() == Some(&Pick::Older(group.clone()));
                    let fold = if *open { "▾" } else { "▸" };
                    let mut line = Line::from(vec![
                        Span::raw("   "),
                        Span::styled(format!("{fold} {count} older"), styles.muted),
                    ]);
                    if selected {
                        let pad = w.saturating_sub(line.width());
                        line.spans.push(Span::raw(" ".repeat(pad)));
                        for span in &mut line.spans {
                            span.style = span.style.add_modifier(Modifier::REVERSED);
                        }
                    }
                    out.push(line);
                    out.push(Line::default());
                }
                Entry::Session { index, child, .. } => {
                    let info = &self.list[*index];
                    let selected =
                        focused && self.selected.as_ref() == Some(&Pick::Session(info.id.clone()));
                    let status = self.status(info);
                    let mut pair = row(info, status, *child, w, look, styles);
                    // A session at work sits on a faint blue band, the whole width.
                    let band = styles
                        .info
                        .fg
                        .filter(|_| status == Status::Running)
                        .and_then(|info| crate::chat::mix(styles.panel, info, RUNNING_TINT));
                    if let Some(bg) = band.filter(|_| !selected) {
                        for line in &mut pair {
                            let pad = w.saturating_sub(line.width());
                            line.spans.push(Span::raw(" ".repeat(pad)));
                            for span in &mut line.spans {
                                span.style = span.style.bg(bg);
                            }
                        }
                    }
                    if selected {
                        for line in &mut pair {
                            let pad = w.saturating_sub(line.width());
                            line.spans.push(Span::raw(" ".repeat(pad)));
                            for span in &mut line.spans {
                                span.style = span.style.add_modifier(Modifier::REVERSED);
                            }
                        }
                    }
                    out.extend(pair);
                    out.push(Line::default());
                }
            }
        }
        out.into_iter().map(|l| fit_line(l, w)).collect()
    }
}

/// Whether a session has a second row: a question for the user, a turn to look at, something
/// waiting, a preview or changed lines.
pub(super) fn tall(info: &SessionInfo) -> bool {
    let preview = info
        .preview
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty());
    let changed = info.added.unwrap_or(0) > 0 || info.removed.unwrap_or(0) > 0;
    let unseen = info.unseen == Some(true);
    info.asking.unwrap_or(0) > 0 || unseen || info.queued > 0 || preview || changed
}

/// Its status, the title and how long ago it was active, then (when there is any) `needs you` or
/// `your turn`, what is queued, the preview and the changed lines. The shown session has a bar in
/// the margin and a bold accent title; one waiting for an answer a bold amber title, one that
/// finished unseen a bold one; one delegated to by another a `↳` before the title.
pub(super) fn row(
    info: &SessionInfo,
    status: Status,
    child: bool,
    w: usize,
    look: &Look,
    styles: &Styles,
) -> Vec<Line<'static>> {
    let current = info.id == look.current;
    let bar = Span::styled(if current { "▎" } else { " " }, styles.accent);
    let lead = |second: Span<'static>| vec![bar.clone(), second, Span::raw(" ")];
    // Sessions idle for over a day fade, so the ones in use stand out.
    let stale = !current && status == Status::Idle && look.now_ms - info.last_activity > STALE_MS;

    let (title_room, age, left) = title_room(info, child, w, look);
    let age_w = age.as_deref().map_or(0, width);
    // The shown session's title goes on under itself rather than be cut.
    let (title, rest) = match split_title(&session_title(info), title_room) {
        (first, Some(rest)) if current => (first, Some(truncate(&rest, left))),
        _ => (truncate(&session_title(info), title_room), None),
    };
    let title_style = match status {
        _ if current => styles.accent.add_modifier(Modifier::BOLD),
        Status::Asking => styles.warning.add_modifier(Modifier::BOLD),
        Status::Finished => Style::new().add_modifier(Modifier::BOLD),
        Status::Idle if stale => styles.muted,
        _ => Style::new(),
    };
    let mut first = lead(status.icon(look.spinner, styles));
    if child {
        first.push(Span::styled("↳ ", styles.muted));
    }
    if let Some(slot) = look.slot(&info.id) {
        first.push(Span::styled(slot, styles.accent));
    }
    let title_w = width(&title);
    first.push(Span::styled(title, title_style));
    if let Some(age) = age {
        first.push(Span::raw(" ".repeat(left - title_w - age_w)));
        first.push(Span::styled(age, styles.muted));
    }
    let mut lines = vec![Line::from(first)];
    if let Some(rest) = rest {
        let mut next = lead(Span::raw(" "));
        if child {
            next.push(Span::raw("  "));
        }
        next.push(Span::styled(rest, title_style));
        lines.push(Line::from(next));
    }
    if !tall(info) {
        return lines;
    }

    let mut second = lead(Span::raw(" "));
    if child {
        second.push(Span::raw("  "));
    }
    let mut left = left;
    let stat = changed(info, styles);
    let stat_w: usize = stat.iter().map(|s| width(&s.content)).sum();
    let preview = info.preview.as_deref().unwrap_or("");
    let preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut labels = Vec::new();
    if info.asking.unwrap_or(0) > 0 {
        labels.push(("needs you".to_string(), styles.warning));
    } else if status == Status::Finished {
        labels.push(("your turn".to_string(), styles.success));
    }
    if info.queued > 0 {
        labels.push((format!("{} queued", info.queued), styles.accent));
    }
    let start = second.len();
    for (label, style) in labels {
        let sep = if second.len() > start { 3 } else { 0 };
        if sep + width(&label) + 1 >= left {
            break;
        }
        if sep > 0 {
            second.push(Span::styled(" · ", styles.muted));
        }
        left -= sep + width(&label);
        second.push(Span::styled(label, style));
    }
    // A dot between the labels and what follows them.
    if second.len() > start && (stat_w > 0 || !preview.is_empty()) && left > 3 {
        second.push(Span::styled(" · ", styles.muted));
        left -= 3;
    }
    if stat_w > 0 && stat_w + 2 <= left {
        let preview = truncate(&preview, left - stat_w - 1);
        let pad = left - width(&preview) - stat_w;
        second.push(Span::styled(preview, styles.muted));
        second.push(Span::raw(" ".repeat(pad)));
        second.extend(stat);
    } else {
        second.push(Span::styled(truncate(&preview, left), styles.muted));
    }
    // A stale title is muted (above); its second row fades further, so the title stays readable.
    if stale {
        for span in &mut second {
            span.style = span.style.add_modifier(Modifier::DIM);
        }
    }
    lines.push(Line::from(second));
    lines
}

/// A row's room in `w` cells: for the title, the age beside it (when it fits), and in all after
/// the margin, the child marker and the slot's number.
pub(super) fn title_room(
    info: &SessionInfo,
    child: bool,
    w: usize,
    look: &Look,
) -> (usize, Option<String>, usize) {
    let left = w
        .saturating_sub(3)
        .saturating_sub(if child { 2 } else { 0 })
        .saturating_sub(look.slot(&info.id).as_deref().map_or(0, width));
    let age = age(info.last_activity, look.now_ms);
    let age_w = width(&age);
    if age_w + 2 <= left {
        (left - age_w - 1, Some(age), left)
    } else {
        (left, None, left)
    }
}

/// `title` broken after the last word that fits in `room`, the rest when there is any; whole
/// when it fits or its first word doesn't.
pub(super) fn split_title(title: &str, room: usize) -> (String, Option<String>) {
    if width(title) <= room {
        return (title.to_string(), None);
    }
    let mut first = String::new();
    let mut words = title.split_whitespace().peekable();
    while let Some(word) = words.peek() {
        let w = width(&first) + usize::from(!first.is_empty()) + width(word);
        if w > room {
            break;
        }
        if !first.is_empty() {
            first.push(' ');
        }
        first.push_str(word);
        words.next();
    }
    if first.is_empty() {
        return (title.to_string(), None);
    }
    let rest: Vec<&str> = words.collect();
    (first, Some(rest.join(" ")))
}

/// How long ago, short: now, 5m, 3h, 2d.
pub(super) fn age(then_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// `+12 −3` in green and red; a zero side is left out, and nothing when both are.
pub(super) fn changed(info: &SessionInfo, styles: &Styles) -> Vec<Span<'static>> {
    let added = info.added.unwrap_or(0).max(0);
    let removed = info.removed.unwrap_or(0).max(0);
    let mut out = Vec::new();
    if added > 0 {
        out.push(Span::styled(format!("+{added}"), styles.success));
    }
    if removed > 0 {
        if added > 0 {
            out.push(Span::raw(" "));
        }
        out.push(Span::styled(format!("−{removed}"), styles.danger));
    }
    out
}
