//! The sessions sidebar: every session with its live state, title, a line of what was said last
//! and the lines its edits changed. Fed by `list_sessions` at start and `session.changed`
//! events afterwards; newest activity first. A session that finished a turn while another one
//! was shown stays marked until it is shown.

use std::collections::HashSet;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use suite_protocol::{SessionInfo, SessionState};

use crate::menu::session_title;
use crate::style::Styles;
use crate::util::{fit_line, truncate, width};

/// Columns of the sidebar including its border and the gap after it.
pub const SIDEBAR_WIDTH: u16 = 31;
/// Rows a PgUp / PgDn moves.
const PAGE: usize = 5;

/// What a key in the focused sidebar asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionsAction {
    Redraw,
    /// Back to the editor (Esc).
    Leave,
    /// Show this session (Enter).
    Open(Box<SessionInfo>),
}

#[derive(Debug, Default)]
pub struct Sessions {
    /// Most recent activity first.
    list: Vec<SessionInfo>,
    /// The row the keys are on while the sidebar has focus.
    selected: Option<String>,
    /// First row drawn.
    top: usize,
    /// Finished a turn since it was last shown.
    finished: HashSet<String>,
}

/// How the list is drawn this frame.
pub struct Look<'a> {
    /// The shown session.
    pub current: &'a str,
    /// Draws the selected row reversed.
    pub focused: bool,
    pub spinner: char,
    pub now_ms: i64,
}

impl Sessions {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn get(&self, id: &str) -> Option<&SessionInfo> {
        self.list.iter().find(|s| s.id == id)
    }

    /// Whether any session is working (the spinner needs frames).
    pub fn any_running(&self) -> bool {
        self.list.iter().any(|s| s.state == SessionState::Running)
    }

    /// Replaces the whole list (`list_sessions`).
    pub fn set_all(&mut self, list: Vec<SessionInfo>) {
        for info in &list {
            self.note_finished(info);
        }
        self.list = list;
        self.sort();
    }

    /// Updates a session, or adds it (`session.changed`, a session just created).
    pub fn upsert(&mut self, info: SessionInfo) {
        self.note_finished(&info);
        match self.list.iter_mut().find(|s| s.id == info.id) {
            Some(slot) => *slot = info,
            None => self.list.push(info),
        }
        self.sort();
    }

    /// A session that was running and isn't any more finished a turn.
    fn note_finished(&mut self, info: &SessionInfo) {
        let was_running = self
            .get(&info.id)
            .is_some_and(|old| old.state == SessionState::Running);
        if was_running && info.state != SessionState::Running {
            self.finished.insert(info.id.clone());
        }
    }

    /// Whether this session finished a turn since it was last shown.
    pub fn is_finished(&self, id: &str) -> bool {
        self.finished.contains(id)
    }

    fn sort(&mut self) {
        self.list.sort_by(|a, b| {
            b.last_activity
                .cmp(&a.last_activity)
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    fn index_of(&self, id: Option<&str>) -> Option<usize> {
        let id = id?;
        self.list.iter().position(|s| s.id == id)
    }

    /// The sidebar gets focus: the keys start on the shown session.
    pub fn focus(&mut self, current: &str) {
        self.selected = self
            .index_of(Some(current))
            .or((!self.list.is_empty()).then_some(0))
            .map(|i| self.list[i].id.clone());
    }

    fn move_to(&mut self, index: usize) {
        if let Some(s) = self.list.get(index) {
            self.selected = Some(s.id.clone());
        }
    }

    fn move_by(&mut self, delta: isize) {
        if self.list.is_empty() {
            return;
        }
        let at = self.index_of(self.selected.as_deref()).unwrap_or(0) as isize;
        let last = self.list.len() as isize - 1;
        self.move_to((at + delta).clamp(0, last) as usize);
    }

    pub fn key(&mut self, key: KeyEvent) -> SessionsAction {
        if key.kind == KeyEventKind::Release {
            return SessionsAction::Redraw;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return SessionsAction::Redraw;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-(PAGE as isize)),
            KeyCode::PageDown => self.move_by(PAGE as isize),
            KeyCode::Home | KeyCode::Char('g') => self.move_to(0),
            KeyCode::End | KeyCode::Char('G') => self.move_to(self.list.len().saturating_sub(1)),
            KeyCode::Enter => {
                if let Some(i) = self.index_of(self.selected.as_deref()) {
                    return SessionsAction::Open(Box::new(self.list[i].clone()));
                }
                return SessionsAction::Leave;
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                return SessionsAction::Leave;
            }
            _ => {}
        }
        SessionsAction::Redraw
    }

    /// The sidebar's lines for a `w` x `h` area: a header, then two rows per session. Showing a
    /// session clears its finished mark.
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
        self.finished.remove(current);
        if w < 8 || h == 0 {
            return Vec::new();
        }
        let header_style = if focused {
            styles.accent.add_modifier(Modifier::BOLD)
        } else {
            styles.muted
        };
        let mut header = vec![
            Span::styled("Sessions", header_style),
            Span::styled(format!(" · {}", self.list.len()), styles.muted),
        ];
        if !self.finished.is_empty() {
            header.push(Span::styled(
                format!(" · {} done", self.finished.len()),
                styles.success,
            ));
        }
        let mut out = vec![Line::from(header)];
        let rows = ((h - 1) / 2).max(1);
        let anchor = if focused {
            self.index_of(self.selected.as_deref())
        } else {
            self.index_of(Some(current))
        };
        if let Some(a) = anchor {
            if a < self.top {
                self.top = a;
            } else if a >= self.top + rows {
                self.top = a + 1 - rows;
            }
        }
        self.top = self.top.min(self.list.len().saturating_sub(rows));
        for info in self.list.iter().skip(self.top).take(rows) {
            let selected = focused && self.selected.as_deref() == Some(info.id.as_str());
            let mark = if info.id == current {
                Mark::Current
            } else if self.finished.contains(&info.id) {
                Mark::Finished
            } else {
                Mark::None
            };
            let mut pair = row(info, mark, w, look, styles);
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
        }
        out.into_iter().map(|l| fit_line(l, w)).collect()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    /// The shown session.
    Current,
    /// Finished a turn since it was last shown.
    Finished,
    None,
}

/// Title and changed lines, then the preview and how long ago it was active. The shown session
/// has a bar in the margin and a bold accent title; a running one has the spinner; one that
/// finished unseen a green dot and a bold title.
fn row(
    info: &SessionInfo,
    mark: Mark,
    w: usize,
    look: &Look,
    styles: &Styles,
) -> [Line<'static>; 2] {
    let current = mark == Mark::Current;
    let bar = Span::styled(if current { "▎" } else { " " }, styles.accent);
    let state = if info.state == SessionState::Running {
        Span::styled(look.spinner.to_string(), styles.accent)
    } else if mark == Mark::Finished {
        Span::styled("●", styles.success)
    } else {
        Span::raw(" ")
    };
    let lead = |second: Span<'static>| vec![bar.clone(), second, Span::raw(" ")];
    let room = w.saturating_sub(3);

    let stat = changed(info, styles);
    let stat_w: usize = stat.iter().map(|s| width(&s.content)).sum();
    let title_room = if stat_w > 0 {
        room.saturating_sub(stat_w + 1)
    } else {
        room
    };
    let title = truncate(&session_title(info), title_room);
    let title_style = match mark {
        Mark::Current => styles.accent.add_modifier(Modifier::BOLD),
        Mark::Finished => ratatui::style::Style::new().add_modifier(Modifier::BOLD),
        Mark::None => ratatui::style::Style::new(),
    };
    let pad = room.saturating_sub(width(&title) + stat_w);
    let mut first = lead(state);
    first.push(Span::styled(title, title_style));
    if stat_w > 0 {
        first.push(Span::raw(" ".repeat(pad)));
        first.extend(stat);
    }

    let mut second = lead(Span::raw(" "));
    let mut left = room;
    if info.queued > 0 {
        let queued = format!("{} queued", info.queued);
        if width(&queued) + 1 < room {
            left -= width(&queued) + 3;
            second.push(Span::styled(queued, styles.accent));
            second.push(Span::styled(" · ", styles.muted));
        }
    }
    let age = age(info.last_activity, look.now_ms);
    let age_w = width(&age);
    let preview = info.preview.as_deref().unwrap_or("");
    let preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    if age_w + 2 <= left {
        let preview = truncate(&preview, left - age_w - 1);
        let pad = left - width(&preview) - age_w;
        second.push(Span::styled(preview, styles.muted));
        second.push(Span::raw(" ".repeat(pad)));
        second.push(Span::styled(age, styles.muted));
    } else {
        second.push(Span::styled(truncate(&preview, left), styles.muted));
    }
    [Line::from(first), Line::from(second)]
}

/// How long ago, short: now, 5m, 3h, 2d.
fn age(then_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// `+12 −3` in green and red; a zero side is left out, and nothing when both are.
fn changed(info: &SessionInfo, styles: &Styles) -> Vec<Span<'static>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use suite_renderer_ratatui::Palette;

    fn info(id: &str, title: &str, last: i64) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            title: (!title.is_empty()).then(|| title.into()),
            cwd: "/w".into(),
            created_at: 0,
            last_activity: last,
            last_seq: 1,
            state: SessionState::Idle,
            queued: 0,
            preview: None,
            added: None,
            removed: None,
        }
    }

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn styles() -> Styles {
        Styles::new(&Palette::default(), false)
    }

    /// Activity times in the tests are a few ms, so they all read "now".
    fn look(current: &str, focused: bool) -> Look<'_> {
        Look {
            current,
            focused,
            spinner: '◐',
            now_ms: 60_000,
        }
    }

    #[test]
    fn newest_activity_first_and_changes_update_in_place() {
        let mut s = Sessions::default();
        s.set_all(vec![info("a", "old", 10), info("b", "new", 30)]);
        let ids = |s: &Sessions| s.list.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&s), ["b", "a"]);
        s.upsert(info("a", "old", 40));
        assert_eq!(ids(&s), ["a", "b"]);
        s.upsert(info("c", "fresh", 20));
        assert_eq!(ids(&s), ["a", "b", "c"]);
        assert_eq!(s.len(), 3);
        assert!(!s.any_running());
        let mut running = info("c", "fresh", 50);
        running.state = SessionState::Running;
        s.upsert(running);
        assert!(s.any_running());
        assert_eq!(ids(&s), ["c", "a", "b"]);
    }

    #[test]
    fn rows_show_state_title_preview_and_changed_lines() {
        let mut s = Sessions::default();
        let mut a = info("a", "Fix the login form", 30);
        a.state = SessionState::Running;
        a.queued = 2;
        a.preview = Some("Looking at the validation".into());
        a.added = Some(12);
        a.removed = Some(3);
        let mut b = info("b", "", 20);
        b.preview = Some("Done. I changed the form and ran the tests twice".into());
        b.added = Some(7);
        let mut c = info("cafe0123456", "Docs", 10);
        c.removed = Some(40);
        s.set_all(vec![a, b, c]);
        let lines = s.render(29, 12, &look("b", false), &styles());
        let got: Vec<_> = text(&lines)
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert_eq!(
            got,
            [
                "Sessions · 3",
                " ◐ Fix the login form  +12 −3",
                "   2 queued · Looking at… now",
                "▎  session b               +7",
                "▎  Done. I changed the f… now",
                "   Docs                   −40",
                "                          now",
            ]
        );
        // Green and red for the changed lines, the spinner in the accent color.
        let st = styles();
        let spans = &lines[1].spans;
        assert_eq!(spans[1].style, st.accent);
        assert_eq!(
            spans.iter().find(|s| s.content == "+12").unwrap().style,
            st.success
        );
        assert_eq!(
            spans.iter().find(|s| s.content == "−3").unwrap().style,
            st.danger
        );
        assert_eq!(
            lines[3].spans[0].content, "▎",
            "the shown session has a bar"
        );
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(age(0, 59_000), "now");
        assert_eq!(age(0, 5 * 60_000), "5m");
        assert_eq!(age(0, 3 * 3_600_000), "3h");
        assert_eq!(age(0, 2 * 86_400_000), "2d");
    }

    #[test]
    fn a_session_that_finished_unseen_is_marked_until_shown() {
        let mut s = Sessions::default();
        let mut a = info("a", "Background job", 10);
        a.state = SessionState::Running;
        s.set_all(vec![a.clone(), info("b", "Here", 20)]);
        assert!(!s.is_finished("a"));
        a.state = SessionState::Idle;
        a.last_activity = 30;
        s.upsert(a);
        assert!(s.is_finished("a"));
        let lines = s.render(29, 9, &look("b", false), &styles());
        assert_eq!(lines[0].to_string(), "Sessions · 2 · 1 done");
        assert_eq!(lines[1].spans[1].content, "●");
        assert_eq!(lines[1].spans[1].style, styles().success);
        assert!(
            lines[1].spans[3]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        // Shown once, the mark is gone.
        s.render(29, 9, &look("a", false), &styles());
        assert!(!s.is_finished("a"));
        // The shown session finishing doesn't mark it.
        let mut b = info("b", "Here", 40);
        b.state = SessionState::Running;
        s.upsert(b.clone());
        b.state = SessionState::Idle;
        s.upsert(b);
        let lines = s.render(29, 9, &look("b", false), &styles());
        assert_eq!(lines[0].to_string(), "Sessions · 2");
    }

    #[test]
    fn keys_move_open_and_leave() {
        let mut s = Sessions::default();
        s.set_all(vec![
            info("a", "one", 30),
            info("b", "two", 20),
            info("c", "three", 10),
        ]);
        s.focus("b");
        assert_eq!(s.key(press(KeyCode::Char('j'))), SessionsAction::Redraw);
        match s.key(press(KeyCode::Enter)) {
            SessionsAction::Open(i) => assert_eq!(i.id, "c"),
            other => panic!("{other:?}"),
        }
        s.key(press(KeyCode::Down));
        match s.key(press(KeyCode::Enter)) {
            SessionsAction::Open(i) => assert_eq!(i.id, "c", "stops at the end"),
            other => panic!("{other:?}"),
        }
        s.key(press(KeyCode::Char('g')));
        s.key(press(KeyCode::Char('k')));
        match s.key(press(KeyCode::Enter)) {
            SessionsAction::Open(i) => assert_eq!(i.id, "a", "stops at the start"),
            other => panic!("{other:?}"),
        }
        s.key(press(KeyCode::Char('G')));
        match s.key(press(KeyCode::Enter)) {
            SessionsAction::Open(i) => assert_eq!(i.id, "c"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.key(press(KeyCode::Esc)), SessionsAction::Leave);
        assert_eq!(s.key(press(KeyCode::Char('x'))), SessionsAction::Redraw);
    }

    #[test]
    fn the_list_scrolls_to_the_selected_or_shown_session() {
        let mut s = Sessions::default();
        s.set_all(
            (0..10)
                .map(|i| info(&format!("s{i}"), &format!("title {i}"), 100 - i))
                .collect(),
        );
        // Room for 3 sessions: header + 6 rows.
        let shown = |s: &mut Sessions, current: &str, focused: bool| {
            text(&s.render(29, 7, &look(current, focused), &styles()))
                .iter()
                .filter(|l| l.contains("title"))
                .map(|l| l.trim().trim_start_matches('▎').trim().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(&mut s, "s0", false),
            ["title 0", "title 1", "title 2"]
        );
        assert_eq!(
            shown(&mut s, "s7", false),
            ["title 5", "title 6", "title 7"]
        );
        s.focus("s7");
        for _ in 0..2 {
            s.key(press(KeyCode::Down));
        }
        assert_eq!(shown(&mut s, "s7", true), ["title 7", "title 8", "title 9"]);
        for _ in 0..9 {
            s.key(press(KeyCode::Up));
        }
        assert_eq!(shown(&mut s, "s7", true), ["title 0", "title 1", "title 2"]);
    }

    #[test]
    fn the_selected_row_is_reversed_only_while_focused() {
        let mut s = Sessions::default();
        s.set_all(vec![info("a", "one", 30), info("b", "two", 20)]);
        s.focus("b");
        let reversed = |lines: &[Line<'_>]| {
            lines
                .iter()
                .filter(|l| {
                    l.spans
                        .iter()
                        .any(|sp| sp.style.add_modifier.contains(Modifier::REVERSED))
                })
                .count()
        };
        assert_eq!(reversed(&s.render(29, 9, &look("a", true), &styles())), 2);
        assert_eq!(reversed(&s.render(29, 9, &look("a", false), &styles())), 0);
    }

    #[test]
    fn nothing_overflows_narrow_or_short_areas() {
        let mut s = Sessions::default();
        let mut a = info("a", "A very long title that cannot possibly fit", 30);
        a.preview = Some("日本語のプレビューもあります。とても長い".into());
        a.added = Some(123_456);
        a.removed = Some(98_765);
        a.queued = 12;
        s.set_all(vec![a]);
        for w in [8, 12, 20, 29] {
            for h in [1, 2, 3, 9] {
                for line in s.render(w, h, &look("a", true), &styles()) {
                    assert!(line.width() <= w, "{w}x{h}: {line}");
                }
            }
        }
    }
}
