//! The sessions sidebar: every session with its live state, title, a line of what was said last
//! and the lines its edits changed, grouped by where it works: a repository (its main checkout),
//! and under it each worktree (`<repo>/.worktrees/<name>`, or any linked worktree, found from
//! its `.git` file). Sessions outside a repository group under their directory's name. Groups
//! are ordered by their latest activity, sessions in a group too; a session that another one
//! delegated to (`parent`) is indented under its parent when they share a group. Fed by
//! `list_sessions` at start and `session.changed` events afterwards. A session that finished a
//! turn while another one was shown stays marked until it is shown. Sessions idle for over a day
//! fold into a `▸ N older` row at the end of their group, which Enter opens and closes; the
//! shown session, a running one and one that finished unseen never fold.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use neo_protocol::{SessionInfo, SessionState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::menu::session_title;
use crate::style::Styles;
use crate::util::{fit_line, truncate, width};

/// Columns of the sidebar including its border and the gap after it: the narrowest, and the
/// widest a wide terminal gives it.
pub const SIDEBAR_WIDTH: u16 = 31;
const SIDEBAR_MAX_WIDTH: u16 = 44;

/// The sidebar's width in a terminal `total` columns wide: a fifth of it, so titles get room.
pub fn sidebar_width(total: u16) -> u16 {
    (total / 5).clamp(SIDEBAR_WIDTH, SIDEBAR_MAX_WIDTH)
}
/// Rows a PgUp / PgDn moves.
const PAGE: usize = 5;
/// Idle longer than this, a session is drawn faded and folds into its group's older row.
const STALE_MS: i64 = 24 * 3600 * 1000;

/// What a key in the focused sidebar asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionsAction {
    Redraw,
    /// Back to the editor (Esc).
    Leave,
    /// Show this session (Enter).
    Open(Box<SessionInfo>),
}

/// Where a session works: the repository (or, outside one, the directory itself) and the
/// worktree of it, if it is not the main checkout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    pub repo: String,
    pub worktree: Option<String>,
}

/// The last path segment.
fn base(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// Finds the place of a working directory: the `.worktrees/<name>` convention first (no disk
/// access), then the nearest `.git` above it: a directory is a main checkout, a file naming
/// `<repo>/.git/worktrees/<id>` a linked worktree (named by its branch). Anything else is its own
/// place.
pub fn place_of(cwd: &str) -> Place {
    let cwd = cwd.trim_end_matches('/');
    if let Some(at) = cwd.find("/.worktrees/") {
        let name = cwd[at + "/.worktrees/".len()..]
            .split('/')
            .next()
            .unwrap_or("");
        if !name.is_empty() {
            return Place {
                repo: cwd[..at].to_string(),
                worktree: Some(name.to_string()),
            };
        }
    }
    for dir in Path::new(cwd).ancestors() {
        let git = dir.join(".git");
        if git.is_dir() {
            return Place {
                repo: dir.to_string_lossy().into_owned(),
                worktree: None,
            };
        }
        if !git.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&git).unwrap_or_default();
        let gitdir = text.trim().strip_prefix("gitdir:").map(str::trim);
        if let Some(gitdir) = gitdir
            && let Some(at) = gitdir.find("/.git/worktrees/")
        {
            let branch = std::fs::read_to_string(Path::new(gitdir).join("HEAD"))
                .ok()
                .and_then(|head| {
                    head.trim()
                        .strip_prefix("ref: refs/heads/")
                        .map(str::to_string)
                });
            let name = branch.unwrap_or_else(|| base(&dir.to_string_lossy()).to_string());
            return Place {
                repo: gitdir[..at].to_string(),
                worktree: Some(name),
            };
        }
        // A submodule or a gitdir elsewhere: its own checkout.
        return Place {
            repo: dir.to_string_lossy().into_owned(),
            worktree: None,
        };
    }
    Place {
        repo: cwd.to_string(),
        worktree: None,
    }
}

/// One line-group of the drawn list.
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    /// A repository (or directory) name.
    Cluster(String),
    /// A worktree of the cluster above.
    Group(String),
    /// A session: index into the list, whether it is drawn as a child of the one above, and
    /// whether it has a second row (a preview, a queue or changed lines).
    Session {
        index: usize,
        child: bool,
        tall: bool,
    },
    /// The group's sessions idle for over a day: how many, and whether they are shown (after it).
    Older {
        group: String,
        count: usize,
        open: bool,
    },
}

impl Entry {
    fn height(&self) -> usize {
        match self {
            // Title, preview and a blank row, so sessions read apart.
            Entry::Session { tall: true, .. } => 3,
            Entry::Session { .. } => 2,
            Entry::Older { .. } => 2,
            _ => 1,
        }
    }

    /// A row the keys can be on.
    fn pickable(&self) -> bool {
        matches!(self, Entry::Session { .. } | Entry::Older { .. })
    }
}

/// A row the keys can be on: a session (its id) or a group's older row (the group's key).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pick {
    Session(String),
    Older(String),
}

#[derive(Debug, Default)]
pub struct Sessions {
    /// Most recent activity first.
    list: Vec<SessionInfo>,
    /// Where each cwd works, found once.
    places: HashMap<String, Place>,
    /// The list as drawn: headers and sessions in order.
    entries: Vec<Entry>,
    /// The rows the keys move over in drawn order, and the positions in it where a group starts.
    visual: Vec<Pick>,
    group_starts: Vec<usize>,
    /// The row the keys are on while the sidebar has focus.
    selected: Option<Pick>,
    /// The groups whose older sessions are shown.
    opened: HashSet<String>,
    /// What the folding was worked out for: the shown session, and the minute.
    current: String,
    minute: i64,
    /// First entry drawn.
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

    /// The latest sessions working where `cwd` does (the same repository), but not `except`.
    pub fn recent_at(&mut self, cwd: &str, except: &str, n: usize) -> Vec<SessionInfo> {
        let repo = self
            .places
            .entry(cwd.to_string())
            .or_insert_with(|| place_of(cwd))
            .repo
            .clone();
        self.list
            .iter()
            .filter(|s| s.id != except)
            .filter(|s| self.places.get(&s.cwd).is_some_and(|p| p.repo == repo))
            .take(n)
            .cloned()
            .collect()
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

    /// Drops a session the daemon deleted (`session.removed`).
    pub fn remove(&mut self, id: &str) {
        self.list.retain(|s| s.id != id);
        self.finished.remove(id);
        if self.selected == Some(Pick::Session(id.to_string())) {
            self.selected = None;
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
        for info in &self.list {
            if !self.places.contains_key(&info.cwd) {
                self.places.insert(info.cwd.clone(), place_of(&info.cwd));
            }
        }
        self.build();
    }

    /// Groups the (recency-sorted) list: repositories by their latest activity, in each the main
    /// checkout and then its worktrees by theirs, in each the sessions by theirs with children
    /// after their parent.
    fn build(&mut self) {
        type Group = Vec<usize>;
        /// A repository: its latest activity, the main checkout's sessions and the worktrees'.
        #[derive(Default)]
        struct Cluster<'a> {
            latest: i64,
            main: Group,
            worktrees: HashMap<&'a str, (i64, Group)>,
        }
        let mut clusters: HashMap<&str, Cluster> = HashMap::new();
        for (index, info) in self.list.iter().enumerate() {
            let Some(place) = self.places.get(&info.cwd) else {
                continue;
            };
            let cluster = clusters.entry(&place.repo).or_default();
            cluster.latest = cluster.latest.max(info.last_activity);
            match &place.worktree {
                None => cluster.main.push(index),
                Some(name) => {
                    let group = cluster.worktrees.entry(name).or_default();
                    group.0 = group.0.max(info.last_activity);
                    group.1.push(index);
                }
            }
        }
        let mut ordered: Vec<_> = clusters.into_iter().collect();
        ordered.sort_by(|a, b| b.1.latest.cmp(&a.1.latest).then_with(|| a.0.cmp(b.0)));
        let now = self.minute * 60_000;
        // Idle for over a day, and nothing to see on it: it folds.
        let folds = |info: &SessionInfo| {
            now - info.last_activity > STALE_MS
                && info.id != self.current
                && info.state != SessionState::Running
                && !self.finished.contains(&info.id)
        };
        let mut entries = Vec::new();
        let mut starts = Vec::new();
        let mut visual = Vec::new();
        let mut put = |key: String, group: &Group, entries: &mut Vec<Entry>| {
            if group.is_empty() {
                return;
            }
            starts.push(visual.len());
            let (old, mut recent): (Vec<_>, Vec<_>) = self
                .family(group)
                .into_iter()
                .partition(|&(index, _)| folds(&self.list[index]));
            // Folding one session saves nothing: it shows.
            let old = if old.len() < 2 {
                recent.extend(old);
                Vec::new()
            } else {
                old
            };
            let session = |index: usize, child: bool| Entry::Session {
                index,
                child,
                tall: tall(&self.list[index]),
            };
            for (index, child) in recent {
                visual.push(Pick::Session(self.list[index].id.clone()));
                entries.push(session(index, child));
            }
            if old.is_empty() {
                return;
            }
            let open = self.opened.contains(&key);
            visual.push(Pick::Older(key.clone()));
            entries.push(Entry::Older {
                group: key,
                count: old.len(),
                open,
            });
            if open {
                for (index, child) in old {
                    visual.push(Pick::Session(self.list[index].id.clone()));
                    entries.push(session(index, child));
                }
            }
        };
        for (repo, cluster) in ordered {
            entries.push(Entry::Cluster(base(repo).to_string()));
            put(repo.to_string(), &cluster.main, &mut entries);
            let mut trees: Vec<_> = cluster.worktrees.into_iter().collect();
            trees.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(b.0)));
            for (name, (_, group)) in trees {
                entries.push(Entry::Group(name.to_string()));
                put(format!("{repo}\0{name}"), &group, &mut entries);
            }
        }
        self.entries = entries;
        self.visual = visual;
        self.group_starts = starts;
    }

    /// A group's sessions in drawn order: each session whose parent is in the group right after
    /// that parent (indented), the others by recency.
    fn family(&self, group: &[usize]) -> Vec<(usize, bool)> {
        let id_of = |i: usize| self.list[i].id.as_str();
        let parent_in_group = |i: usize| {
            self.list[i]
                .parent
                .as_deref()
                .is_some_and(|p| group.iter().any(|&j| id_of(j) == p))
        };
        let mut out = Vec::new();
        for &i in group.iter().filter(|&&i| !parent_in_group(i)) {
            // A session that delegated here but whose parent is elsewhere still wears the mark.
            out.push((i, self.list[i].parent.is_some()));
            let id = id_of(i);
            for &j in group {
                if self.list[j].parent.as_deref() == Some(id) {
                    out.push((j, true));
                }
            }
        }
        out
    }

    fn visual_pos(&self, pick: Option<&Pick>) -> Option<usize> {
        let pick = pick?;
        self.visual.iter().position(|p| p == pick)
    }

    /// The sidebar gets focus: the keys start on the shown session.
    pub fn focus(&mut self, current: &str) {
        let at = self
            .visual_pos(Some(&Pick::Session(current.to_string())))
            .or((!self.visual.is_empty()).then_some(0));
        self.selected = at.map(|p| self.visual[p].clone());
    }

    fn move_to(&mut self, pos: usize) {
        if let Some(pick) = self.visual.get(pos) {
            self.selected = Some(pick.clone());
        }
    }

    fn move_by(&mut self, delta: isize) {
        if self.visual.is_empty() {
            return;
        }
        let at = self.visual_pos(self.selected.as_ref()).unwrap_or(0) as isize;
        let last = self.visual.len() as isize - 1;
        self.move_to((at + delta).clamp(0, last) as usize);
    }

    /// To the first session of the previous or next group (`[` and `]`).
    fn jump_group(&mut self, delta: isize) {
        let at = self.visual_pos(self.selected.as_ref()).unwrap_or(0);
        let current = self
            .group_starts
            .iter()
            .rposition(|&s| s <= at)
            .unwrap_or(0) as isize;
        let target = (current + delta).clamp(0, self.group_starts.len() as isize - 1);
        if let Some(&start) = self.group_starts.get(target as usize) {
            self.move_to(start);
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> SessionsAction {
        if key.kind == KeyEventKind::Release {
            return SessionsAction::Redraw;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Up => self.jump_group(-1),
                KeyCode::Down => self.jump_group(1),
                _ => {}
            }
            return SessionsAction::Redraw;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Char('[') => self.jump_group(-1),
            KeyCode::Char(']') => self.jump_group(1),
            KeyCode::PageUp => self.move_by(-(PAGE as isize)),
            KeyCode::PageDown => self.move_by(PAGE as isize),
            KeyCode::Home | KeyCode::Char('g') => self.move_to(0),
            KeyCode::End | KeyCode::Char('G') => self.move_to(self.visual.len().saturating_sub(1)),
            KeyCode::Enter => match self.selected.clone() {
                Some(Pick::Session(id)) => {
                    if let Some(info) = self.get(&id) {
                        return SessionsAction::Open(Box::new(info.clone()));
                    }
                    return SessionsAction::Leave;
                }
                Some(Pick::Older(group)) => {
                    if !self.opened.remove(&group) {
                        self.opened.insert(group);
                    }
                    self.build();
                }
                None => return SessionsAction::Leave,
            },
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                return SessionsAction::Leave;
            }
            _ => {}
        }
        SessionsAction::Redraw
    }

    /// Scrolls so that the anchored session (with the headers above it, when it is the first of
    /// its group) is inside `rows` lines.
    fn scroll_to(&mut self, anchor: usize, rows: usize) {
        let mut from = anchor;
        while from > 0 && !self.entries[from - 1].pickable() {
            from -= 1;
        }
        if from < self.top || self.top > anchor {
            self.top = from;
        }
        let used = |top: usize, entries: &[Entry]| -> usize {
            entries[top..=anchor].iter().map(Entry::height).sum()
        };
        while self.top < anchor && used(self.top, &self.entries) > rows {
            self.top += 1;
        }
        self.top = self.top.min(self.entries.len().saturating_sub(1));
    }

    /// The sidebar's lines for a `w` x `h` area: a header, then the groups' headers and two rows
    /// per session. Showing a session clears its finished mark.
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
        let mut header = vec![
            Span::raw(" "),
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
        let rows = (h - 1).max(2);
        let shown = Pick::Session(current.to_string());
        let anchor_pick = if focused {
            self.selected.as_ref()
        } else {
            Some(&shown)
        };
        let anchor = self.entries.iter().position(|e| {
            let pick = match e {
                Entry::Session { index, .. } => Pick::Session(self.list[*index].id.clone()),
                Entry::Older { group, .. } => Pick::Older(group.clone()),
                _ => return false,
            };
            Some(&pick) == anchor_pick
        });
        if let Some(a) = anchor {
            self.scroll_to(a, rows);
        }
        let mut used = 0;
        for entry in self.entries.iter().skip(self.top) {
            if used + entry.height() > rows {
                break;
            }
            used += entry.height();
            match entry {
                Entry::Cluster(name) => {
                    // The repository's name, then a rule to the edge.
                    let name = truncate(name, w.saturating_sub(3));
                    let rule = "─".repeat(w.saturating_sub(width(&name) + 2));
                    out.push(Line::from(vec![
                        Span::styled(
                            format!(" {name} "),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(rule, styles.border),
                    ]));
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
                    let mark = if info.id == current {
                        Mark::Current
                    } else if self.finished.contains(&info.id) {
                        Mark::Finished
                    } else {
                        Mark::None
                    };
                    let mut pair = row(info, mark, *child, w, look, styles);
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

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    /// The shown session.
    Current,
    /// Finished a turn since it was last shown.
    Finished,
    None,
}

/// Whether a session has a second row: something waiting, a preview or changed lines.
fn tall(info: &SessionInfo) -> bool {
    let preview = info
        .preview
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty());
    let changed = info.added.unwrap_or(0) > 0 || info.removed.unwrap_or(0) > 0;
    info.queued > 0 || preview || changed
}

/// Title and how long ago it was active, then (when there is any) what is queued, the preview
/// and the changed lines. The shown session has a bar in the margin and a bold accent title; a
/// running one has the spinner; one that finished unseen a green dot and a bold title; one
/// delegated to by another a `↳` before the title.
fn row(
    info: &SessionInfo,
    mark: Mark,
    child: bool,
    w: usize,
    look: &Look,
    styles: &Styles,
) -> Vec<Line<'static>> {
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
    let marker_w = if child { 2 } else { 0 };
    // Sessions idle for over a day fade, so the ones in use stand out.
    let stale = mark == Mark::None
        && info.state != SessionState::Running
        && look.now_ms - info.last_activity > STALE_MS;

    let age = age(info.last_activity, look.now_ms);
    let age_w = width(&age);
    let left = room.saturating_sub(marker_w);
    let (title_room, age) = if age_w + 2 <= left {
        (left - age_w - 1, Some(age))
    } else {
        (left, None)
    };
    let title = truncate(&session_title(info), title_room);
    let title_style = match mark {
        Mark::Current => styles.accent.add_modifier(Modifier::BOLD),
        Mark::Finished => Style::new().add_modifier(Modifier::BOLD),
        Mark::None if stale => styles.muted,
        Mark::None => Style::new(),
    };
    let mut first = lead(state);
    if child {
        first.push(Span::styled("↳ ", styles.muted));
    }
    let title_w = width(&title);
    first.push(Span::styled(title, title_style));
    if let Some(age) = age {
        first.push(Span::raw(" ".repeat(left - title_w - age_w)));
        first.push(Span::styled(age, styles.muted));
    }
    if !tall(info) {
        return vec![Line::from(first)];
    }

    let mut second = lead(Span::raw(" "));
    if child {
        second.push(Span::raw("  "));
    }
    let mut left = left;
    if info.queued > 0 {
        let queued = format!("{} queued", info.queued);
        if width(&queued) + 1 < left {
            left -= width(&queued) + 3;
            second.push(Span::styled(queued, styles.accent));
            second.push(Span::styled(" · ", styles.muted));
        }
    }
    let stat = changed(info, styles);
    let stat_w: usize = stat.iter().map(|s| width(&s.content)).sum();
    let preview = info.preview.as_deref().unwrap_or("");
    let preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");
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
    vec![Line::from(first), Line::from(second)]
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
    use neo_renderer_ratatui::Palette;

    fn info(id: &str, title: &str, last: i64) -> SessionInfo {
        SessionInfo {
            harness: None,
            parent: None,
            id: id.into(),
            title: (!title.is_empty()).then(|| title.into()),
            cwd: "/w".into(),
            created_at: 0,
            last_activity: last,
            last_seq: 1,
            state: SessionState::Idle,
            queued: 0,
            waiting: None,
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
                " Sessions · 3",
                " w ──────────────────────────",
                " ◐ Fix the login form     now",
                "   2 queued · Looking… +12 −3",
                "",
                "▎  New Session            now",
                "▎  Done. I changed the fo… +7",
                "",
                "   Docs                   now",
                "                          −40",
                "",
            ]
        );
        // Green and red for the changed lines, the spinner in the accent color.
        let st = styles();
        assert_eq!(lines[2].spans[1].style, st.accent);
        let spans = &lines[3].spans;
        assert_eq!(
            spans.iter().find(|s| s.content == "+12").unwrap().style,
            st.success
        );
        assert_eq!(
            spans.iter().find(|s| s.content == "−3").unwrap().style,
            st.danger
        );
        assert_eq!(
            lines[5].spans[0].content, "▎",
            "the shown session has a bar"
        );
    }

    #[test]
    fn sessions_idle_for_over_a_day_fade() {
        let mut s = Sessions::default();
        let day = 24 * 3600 * 1000;
        let mut old = info("old", "Stale", day);
        old.preview = Some("said long ago".into());
        s.set_all(vec![info("new", "Fresh", 10 * day), old]);
        let mut l = look("x", false);
        l.now_ms = 10 * day + 1000;
        s.render(29, 20, &l, &styles());
        s.opened.insert("/w".into());
        s.build();
        let lines = s.render(29, 20, &l, &styles());
        let title = |text: &str| {
            lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .find(|s| s.content.contains(text))
                .unwrap()
                .style
        };
        assert!(!title("Fresh").add_modifier.contains(Modifier::DIM));
        assert_eq!(title("Stale"), styles().muted);
        assert!(title("said long ago").add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn sessions_idle_for_over_a_day_fold_into_an_older_row_that_enter_opens() {
        let mut s = Sessions::default();
        let day = 24 * 3600 * 1000;
        let mut running = info("run", "Busy", day);
        running.state = SessionState::Running;
        s.set_all(vec![
            info("new", "Fresh", 10 * day),
            info("old", "Stale", 2 * day),
            info("older", "Staler", day),
            info("shown", "Shown", day),
            running,
        ]);
        let mut l = look("shown", false);
        l.now_ms = 10 * day + 1000;
        let rows = |s: &mut Sessions, l: &Look| {
            text(&s.render(29, 40, l, &styles()))
                .iter()
                .map(|l| l.trim().trim_start_matches('▎').trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with("Sessions") && !l.ends_with('─'))
                // Without the age.
                .map(|l| {
                    l.trim_end_matches(|c: char| c.is_ascii_digit() || c == 'd')
                        .trim_end_matches("now")
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        // The shown and the running session stay; the two idle ones fold.
        assert_eq!(rows(&mut s, &l), ["Fresh", "◐ Busy", "Shown", "▸ 2 older"]);
        l.focused = true;
        s.focus("shown");
        s.key(press(KeyCode::Down));
        assert_eq!(s.selected, Some(Pick::Older("/w".into())));
        assert_eq!(s.key(press(KeyCode::Enter)), SessionsAction::Redraw);
        assert_eq!(
            rows(&mut s, &l),
            ["Fresh", "◐ Busy", "Shown", "▾ 2 older", "Stale", "Staler"]
        );
        s.key(press(KeyCode::Down));
        match s.key(press(KeyCode::Enter)) {
            SessionsAction::Open(i) => assert_eq!(i.id, "old"),
            other => panic!("{other:?}"),
        }
        s.key(press(KeyCode::Up));
        s.key(press(KeyCode::Enter));
        assert_eq!(rows(&mut s, &l), ["Fresh", "◐ Busy", "Shown", "▸ 2 older"]);
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
        assert_eq!(lines[0].to_string(), " Sessions · 2 · 1 done");
        assert_eq!(lines[2].spans[1].content, "●");
        assert_eq!(lines[2].spans[1].style, styles().success);
        assert!(
            lines[2].spans[3]
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
        assert_eq!(lines[0].to_string(), " Sessions · 2");
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
                .map(|i| SessionInfo {
                    preview: Some("said".into()),
                    ..info(&format!("s{i}"), &format!("title {i}"), 100 - i)
                })
                .collect(),
        );
        // Room for 3 sessions: header, the group's name and 9 rows.
        let shown = |s: &mut Sessions, current: &str, focused: bool| {
            text(&s.render(29, 11, &look(current, focused), &styles()))
                .iter()
                .filter(|l| l.contains("title"))
                .map(|l| {
                    let l = l.trim().trim_start_matches('▎').trim();
                    l.trim_end_matches("now").trim().to_string()
                })
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
        let mut b = info("b", "two", 20);
        b.preview = Some("said".into());
        s.set_all(vec![info("a", "one", 30), b]);
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

    // ---- groups ----

    fn at(id: &str, title: &str, cwd: &str, last: i64) -> SessionInfo {
        SessionInfo {
            cwd: cwd.into(),
            ..info(id, title, last)
        }
    }

    /// Two repositories; the first has a main checkout and two worktrees.
    fn grouped() -> Sessions {
        let mut s = Sessions::default();
        let mut login = at("c", "Fix the login", "/w/site/.worktrees/fix-login", 50);
        login.state = SessionState::Running;
        login.preview = Some("Editing the form".into());
        let mut child = at("d", "Write tests", "/w/site/.worktrees/fix-login", 45);
        child.parent = Some("c".into());
        s.set_all(vec![
            at("a", "Plan the release", "/w/site", 10),
            at("b", "Docs", "/w/site", 20),
            login,
            child,
            at("e", "Dark mode", "/w/site/.worktrees/dark/src", 30),
            at("f", "Scratch", "/w/notes", 40),
        ]);
        s
    }

    #[test]
    fn sessions_group_by_repository_and_worktree_newest_group_first() {
        let mut s = grouped();
        let lines = text(&s.render(29, 40, &look("a", false), &styles()));
        let got: Vec<_> = lines.iter().map(|l| l.trim_end().to_string()).collect();
        assert_eq!(
            got,
            [
                " Sessions · 6",
                " site ───────────────────────",
                "   Docs                   now",
                "",
                "▎  Plan the release       now",
                "",
                "   worktree fix-login",
                " ◐ Fix the login          now",
                "   Editing the form",
                "",
                "   ↳ Write tests          now",
                "",
                "   worktree dark",
                "   Dark mode              now",
                "",
                " notes ──────────────────────",
                "   Scratch                now",
                "",
            ]
        );
    }

    #[test]
    fn the_cluster_with_the_latest_activity_leads_and_worktrees_follow_their_repo() {
        let mut s = grouped();
        // The notes directory is quiet; the site repo has the newest session in a worktree.
        let mut f = at("f", "Scratch", "/w/notes", 5);
        f.preview = None;
        s.upsert(f);
        let order = |s: &Sessions| {
            s.entries
                .iter()
                .filter_map(|e| match e {
                    Entry::Cluster(n) => Some(n.clone()),
                    Entry::Group(n) => Some(format!("  {n}")),
                    Entry::Session { .. } | Entry::Older { .. } => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&s), ["site", "  fix-login", "  dark", "notes"]);
        // A newer session in the notes directory moves its cluster up.
        s.upsert(at("f", "Scratch", "/w/notes", 99));
        assert_eq!(order(&s), ["notes", "site", "  fix-login", "  dark"]);
        // Within a repo, the worktree with the newest activity comes first.
        s.upsert(at("e", "Dark mode", "/w/site/.worktrees/dark", 200));
        assert_eq!(order(&s), ["site", "  dark", "  fix-login", "notes"]);
    }

    #[test]
    fn keys_skip_headers_and_jump_between_groups() {
        let mut s = grouped();
        s.focus("b");
        let selected = |s: &Sessions| match s.selected.clone().unwrap() {
            Pick::Session(id) => id,
            Pick::Older(group) => group,
        };
        let mut walk = vec![selected(&s)];
        for _ in 0..5 {
            s.key(press(KeyCode::Down));
            walk.push(selected(&s));
        }
        // Main checkout (b, a: newest first), the worktree's session and its child (c, d), then
        // dark (e) and the notes (f): the order on screen.
        assert_eq!(walk, ["b", "a", "c", "d", "e", "f"]);
        s.key(press(KeyCode::Down));
        assert_eq!(selected(&s), "f", "stops at the end");

        // Group jumps: first session of the previous / next group.
        s.focus("a");
        s.key(press(KeyCode::Char(']')));
        assert_eq!(selected(&s), "c");
        s.key(press(KeyCode::Down));
        s.key(press(KeyCode::Char(']')));
        assert_eq!(selected(&s), "e");
        s.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
        assert_eq!(selected(&s), "f");
        s.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
        assert_eq!(selected(&s), "f", "no group after the last");
        s.key(press(KeyCode::Char('[')));
        assert_eq!(selected(&s), "e");
        s.key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
        assert_eq!(selected(&s), "c");
        s.key(press(KeyCode::Char('[')));
        assert_eq!(selected(&s), "b");
        // Enter opens what is selected.
        match s.key(press(KeyCode::Enter)) {
            SessionsAction::Open(i) => assert_eq!(i.id, "b"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_child_of_a_session_elsewhere_is_marked_but_stays_in_its_own_group() {
        let mut s = Sessions::default();
        let mut child = at("k", "Run the build", "/w/site/.worktrees/ci", 20);
        child.parent = Some("p".into());
        s.set_all(vec![at("p", "Main chat", "/w/site", 30), child]);
        let lines = text(&s.render(29, 20, &look("p", false), &styles()));
        let got: Vec<_> = lines.iter().map(|l| l.trim_end().to_string()).collect();
        assert_eq!(
            got,
            [
                " Sessions · 2",
                " site ───────────────────────",
                "▎  Main chat              now",
                "",
                "   worktree ci",
                "   ↳ Run the build        now",
                "",
            ]
        );
    }

    #[test]
    fn the_list_scrolls_to_show_the_selected_sessions_group_header() {
        let mut s = grouped();
        s.focus("e");
        // 4 lines: a group header and one session at a time fit with the cluster before.
        let got = text(&s.render(29, 4, &look("a", true), &styles()));
        let got: Vec<_> = got.iter().map(|l| l.trim_end().to_string()).collect();
        assert_eq!(
            got,
            [
                " Sessions · 6",
                "   worktree dark",
                "   Dark mode              now",
                ""
            ]
        );
        s.focus("f");
        let got = text(&s.render(29, 4, &look("a", true), &styles()));
        assert_eq!(got[1].trim_end(), " notes ──────────────────────");
        assert_eq!(got[2].trim_end(), "   Scratch                now");
    }

    #[test]
    fn places_come_from_the_worktrees_convention_and_from_git_files() {
        let root = std::env::temp_dir().join(format!("neo-term-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("app");
        let linked = root.join("app-hotfix");
        let admin = repo.join(".git/worktrees/app-hotfix");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(admin.join("HEAD"), "ref: refs/heads/hotfix-1\n").unwrap();
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
        let (repo_s, linked_s) = (repo.to_string_lossy(), linked.to_string_lossy());

        // A subdirectory belongs to the checkout around it.
        let main = place_of(&format!("{repo_s}/src"));
        assert_eq!((main.repo.as_str(), main.worktree), (&*repo_s, None));
        // A linked worktree elsewhere groups under its repository, named by its branch.
        let other = place_of(&linked_s);
        assert_eq!(other.repo, repo_s);
        assert_eq!(other.worktree.as_deref(), Some("hotfix-1"));
        // The convention needs no disk access.
        let named = place_of("/nowhere/proj/.worktrees/fix-login/crates/a");
        assert_eq!(named.repo, "/nowhere/proj");
        assert_eq!(named.worktree.as_deref(), Some("fix-login"));
        // Outside any repository: the directory itself.
        let plain = place_of("/nowhere/notes/");
        assert_eq!(plain.repo, "/nowhere/notes");
        assert_eq!(plain.worktree, None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
