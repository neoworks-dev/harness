//! The sessions sidebar: every session with its live state, title, a line of what was said last
//! and the lines its edits changed, grouped by where it works: a repository (its main checkout),
//! and under it each worktree (`<repo>/.worktrees/<name>`, or any linked worktree, found from
//! its `.git` file). Sessions outside a repository group under their directory's name. Groups
//! are ordered by their latest activity, sessions in a group too, except that where a session
//! works or waits for an answer comes first; a session that another one delegated to (`parent`)
//! is indented under its parent, with no blank row between, when they share a group, and leaves
//! the list once its work is done (unless it is the one shown). Fed by
//! `list_sessions` at start and `session.changed` events afterwards. Each session's status leads
//! its row in its own color, the most pressing one when several hold: `!` it waits for an answer
//! (amber), a spinner it is working (blue), `●` it finished a turn nobody has looked at yet, the
//! daemon's `unseen` (green, `your turn` on its second line); the header counts each. Sessions idle for over a day fold into a `▸ N older` row at the end of their group,
//! which Enter opens and closes; the shown session and one with a status never fold. Enter on a
//! repository's header folds the whole of it, worktrees too, into that one line (`▸ name · N`,
//! with the most pressing status in it); Enter again unfolds it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use neo_protocol::{SessionInfo, SessionState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::menu::session_title;
use crate::style::Styles;
use crate::util::{fit_line, truncate, width};

// What the sidebar draws.
mod draw;
use draw::*;

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
/// How much of the info blue a working session's band has over the pane's background.
const RUNNING_TINT: f32 = 0.14;

/// What a key in the focused sidebar asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionsAction {
    Redraw,
    /// Back to the editor (Esc).
    Leave,
    /// Show this session (Enter).
    Open(Box<SessionInfo>),
    /// Start a new chat (Enter on `+ New chat`, or n).
    New,
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
    /// A repository (or directory): its path, its name and, when it is folded, what is in it.
    Cluster {
        repo: String,
        name: String,
        folded: Option<Folded>,
    },
    /// A worktree of the cluster above.
    Group(String),
    /// A session: index into the list, whether it is drawn as a child of the one above,
    /// whether it has a second row (a preview, a queue or changed lines), whether its title
    /// takes two rows (the shown session's, when it doesn't fit; set as it is drawn), and
    /// whether a blank row follows it (not before its own child or its sibling).
    Session {
        index: usize,
        child: bool,
        tall: bool,
        wraps: bool,
        gap: bool,
    },
    /// The group's sessions idle for over a day: how many, and whether they are shown (after it).
    Older {
        group: String,
        count: usize,
        open: bool,
    },
}

/// What a folded repository holds: how many sessions, and the most pressing status among them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Folded {
    count: usize,
    status: Status,
}

/// What a session's row says before its title; the first that holds, when several do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// A permission request or a choice is open for the user.
    Asking,
    Running,
    /// Finished a turn since it was last shown.
    Finished,
    Idle,
}

impl Status {
    /// `!` in amber, the spinner in blue, `●` in green, and a blank for nothing.
    pub fn icon(self, spinner: char, styles: &Styles) -> Span<'static> {
        match self {
            Status::Asking => Span::styled("!", styles.warning.add_modifier(Modifier::BOLD)),
            Status::Running => Span::styled(spinner.to_string(), styles.info),
            Status::Finished => Span::styled("●", styles.success),
            Status::Idle => Span::raw(" "),
        }
    }
}

impl Entry {
    fn height(&self) -> usize {
        match self {
            // Title, preview and a blank row, so sessions read apart.
            Entry::Session {
                tall, wraps, gap, ..
            } => 1 + usize::from(*tall) + usize::from(*wraps) + usize::from(*gap),
            Entry::Older { .. } => 2,
            _ => 1,
        }
    }

    /// A row of the list itself, not a header over one.
    fn is_row(&self) -> bool {
        matches!(self, Entry::Session { .. } | Entry::Older { .. })
    }
}

/// A row the keys can be on: `+ New chat`, a repository's header (its path), a session (its id)
/// or a group's older row (the group's key).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pick {
    New,
    Cluster(String),
    Session(String),
    Older(String),
}

#[derive(Debug, Default)]
pub struct Sessions {
    /// By place, latest first.
    list: Vec<SessionInfo>,
    /// Each session's place: its activity when first seen. New messages don't move it, so open
    /// chats stay where they are while they talk.
    places_at: HashMap<String, i64>,
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
    /// The repositories folded into their header.
    folded: HashSet<String>,
    /// What the folding was worked out for: the shown session, and the minute.
    current: String,
    minute: i64,
    /// First entry drawn.
    top: usize,
}

/// How the list is drawn this frame.
pub struct Look<'a> {
    /// The shown session.
    pub current: &'a str,
    /// Draws the selected row reversed.
    pub focused: bool,
    pub spinner: char,
    pub now_ms: i64,
    /// The marked sessions (Alt-m), slot 1 first: each has its slot's number before its title.
    pub marks: &'a [String],
}

impl Look<'_> {
    /// `1 ` before the title of the session in slot 1, and so on.
    fn slot(&self, id: &str) -> Option<String> {
        let at = self.marks.iter().position(|m| m == id)?;
        Some(format!("{} ", at + 1))
    }
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
        let mut recent: Vec<_> = self
            .list
            .iter()
            .filter(|s| s.id != except)
            .filter(|s| self.places.get(&s.cwd).is_some_and(|p| p.repo == repo))
            .cloned()
            .collect();
        recent.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        recent.truncate(n);
        recent
    }

    /// Whether any session is working (the spinner needs frames).
    pub fn any_running(&self) -> bool {
        self.list.iter().any(|s| s.state == SessionState::Running)
    }

    /// Replaces the whole list (`list_sessions`).
    pub fn set_all(&mut self, list: Vec<SessionInfo>) {
        self.list = list;
        self.sort();
    }

    /// Updates a session, or adds it (`session.changed`, a session just created).
    pub fn upsert(&mut self, info: SessionInfo) {
        match self.list.iter_mut().find(|s| s.id == info.id) {
            Some(slot) => *slot = info,
            None => self.list.push(info),
        }
        self.sort();
    }

    /// Drops a session the daemon deleted (`session.removed`).
    pub fn remove(&mut self, id: &str) {
        self.list.retain(|s| s.id != id);
        self.places_at.remove(id);
        if self.selected == Some(Pick::Session(id.to_string())) {
            self.selected = None;
        }
        self.sort();
    }

    /// What the session's row leads with: the most pressing of waiting, working and finished.
    /// Finished is the daemon's `unseen`: a turn ended since anybody had it open; the shown one
    /// is being looked at.
    pub fn status(&self, info: &SessionInfo) -> Status {
        if info.asking.unwrap_or(0) > 0 {
            Status::Asking
        } else if info.state == SessionState::Running {
            Status::Running
        } else if info.unseen == Some(true) && info.id != self.current {
            Status::Finished
        } else {
            Status::Idle
        }
    }

    /// The header's count of each status, `!1 ◐2 ●3`, each in its color; empty when none hold.
    fn tally(&self, look: &Look, styles: &Styles) -> Vec<Span<'static>> {
        let mut out = Vec::new();
        for status in [Status::Asking, Status::Running, Status::Finished] {
            let n = self
                .list
                .iter()
                .filter(|i| !self.gone(i) && self.status(i) == status)
                .count();
            if n == 0 {
                continue;
            }
            if !out.is_empty() {
                out.push(Span::raw(" "));
            }
            let icon = status.icon(look.spinner, styles);
            let count = Span::styled(n.to_string(), icon.style);
            out.extend([icon, count]);
        }
        out
    }

    fn sort(&mut self) {
        for info in &self.list {
            self.places_at
                .entry(info.id.clone())
                .or_insert(info.last_activity);
        }
        let at = |info: &SessionInfo| self.places_at[&info.id];
        self.list
            .sort_by(|a, b| at(b).cmp(&at(a)).then_with(|| a.id.cmp(&b.id)));
        for info in &self.list {
            if !self.places.contains_key(&info.cwd) {
                self.places.insert(info.cwd.clone(), place_of(&info.cwd));
            }
        }
        self.build();
    }

    /// Groups the (place-sorted) list: repositories by their latest place, in each the main
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
            if self.gone(info) {
                continue;
            }
            let at = self.places_at[&info.id];
            let cluster = clusters.entry(&place.repo).or_default();
            cluster.latest = cluster.latest.max(at);
            match &place.worktree {
                None => cluster.main.push(index),
                Some(name) => {
                    let group = cluster.worktrees.entry(name).or_default();
                    group.0 = group.0.max(at);
                    group.1.push(index);
                }
            }
        }
        // Where something works or waits for the user comes first, so it never scrolls away.
        let busy = |group: &Group| group.iter().any(|&i| self.busy(&self.list[i]));
        let mut ordered: Vec<_> = clusters.into_iter().collect();
        ordered.sort_by(|a, b| {
            let busy_of = |c: &Cluster| busy(&c.main) || c.worktrees.values().any(|g| busy(&g.1));
            busy_of(&b.1)
                .cmp(&busy_of(&a.1))
                .then_with(|| b.1.latest.cmp(&a.1.latest))
                .then_with(|| a.0.cmp(b.0))
        });
        let now = self.minute * 60_000;
        // Idle for over a day, and nothing to see on it: it folds.
        let folds = |info: &SessionInfo| {
            now - info.last_activity > STALE_MS
                && info.id != self.current
                && self.status(info) == Status::Idle
        };
        let mut entries = Vec::new();
        let mut starts = Vec::new();
        // `+ New chat`, pinned under the header, comes first.
        let mut visual = vec![Pick::New];
        let put = |key: String,
                   group: &Group,
                   entries: &mut Vec<Entry>,
                   visual: &mut Vec<Pick>,
                   starts: &mut Vec<usize>| {
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
                wraps: false,
                gap: true,
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
            let folded = self.folded.contains(repo).then(|| {
                let all: Vec<_> = cluster
                    .main
                    .iter()
                    .chain(cluster.worktrees.values().flat_map(|(_, g)| g))
                    .map(|&i| &self.list[i])
                    .collect();
                Folded {
                    count: all.len(),
                    status: all
                        .iter()
                        .map(|i| self.status(i))
                        .min()
                        .unwrap_or(Status::Idle),
                }
            });
            if folded.is_some() {
                // A folded repository is a group of its own for `[` and `]`: its header.
                starts.push(visual.len());
            }
            visual.push(Pick::Cluster(repo.to_string()));
            entries.push(Entry::Cluster {
                repo: repo.to_string(),
                name: base(repo).to_string(),
                folded,
            });
            if folded.is_some() {
                continue;
            }
            put(
                repo.to_string(),
                &cluster.main,
                &mut entries,
                &mut visual,
                &mut starts,
            );
            let mut trees: Vec<_> = cluster.worktrees.into_iter().collect();
            trees.sort_by(|a, b| {
                busy(&b.1.1)
                    .cmp(&busy(&a.1.1))
                    .then_with(|| b.1.0.cmp(&a.1.0))
                    .then_with(|| a.0.cmp(b.0))
            });
            for (name, (_, group)) in trees {
                entries.push(Entry::Group(name.to_string()));
                put(
                    format!("{repo}\0{name}"),
                    &group,
                    &mut entries,
                    &mut visual,
                    &mut starts,
                );
            }
        }
        // No blank row between a session and its child, nor between two children of one parent.
        for at in 0..entries.len().saturating_sub(1) {
            let (Entry::Session { index: a, .. }, Entry::Session { index: b, child, .. }) =
                (&entries[at], &entries[at + 1])
            else {
                continue;
            };
            let (a, b) = (&self.list[*a], &self.list[*b]);
            let joined = *child
                && (b.parent.as_deref() == Some(a.id.as_str())
                    || (a.parent.is_some() && a.parent == b.parent));
            if let Entry::Session { gap, .. } = &mut entries[at] {
                *gap = !joined;
            }
        }
        self.entries = entries;
        self.visual = visual;
        self.group_starts = starts;
    }

    /// How many sessions the list holds: not the delegated ones that are done.
    fn shown_count(&self) -> usize {
        self.list.iter().filter(|i| !self.gone(i)).count()
    }

    /// Working, or waiting for an answer.
    fn busy(&self, info: &SessionInfo) -> bool {
        matches!(self.status(info), Status::Asking | Status::Running)
    }

    /// A delegated session whose work is done: it leaves the list unless it is the one shown.
    fn gone(&self, info: &SessionInfo) -> bool {
        info.parent.is_some() && info.id != self.current && !self.busy(info)
    }

    /// A group's sessions in drawn order: each session whose parent is in the group right after
    /// that parent (indented), the others by recency, those at work (or theirs) first.
    fn family(&self, group: &[usize]) -> Vec<(usize, bool)> {
        let id_of = |i: usize| self.list[i].id.as_str();
        let parent_in_group = |i: usize| {
            self.list[i]
                .parent
                .as_deref()
                .is_some_and(|p| group.iter().any(|&j| id_of(j) == p))
        };
        let busy = |i: usize| {
            self.busy(&self.list[i])
                || group.iter().any(|&j| {
                    self.list[j].parent.as_deref() == Some(id_of(i)) && self.busy(&self.list[j])
                })
        };
        let mut roots: Vec<usize> = group
            .iter()
            .copied()
            .filter(|&i| !parent_in_group(i))
            .collect();
        roots.sort_by_key(|&i| !busy(i));
        let mut out = Vec::new();
        for i in roots {
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

    /// The row standing for a session: itself, or its repository's header when that is folded.
    fn row_of(&self, id: &str) -> Pick {
        let folded = self
            .get(id)
            .and_then(|info| self.places.get(&info.cwd))
            .filter(|place| self.folded.contains(&place.repo));
        match folded {
            Some(place) => Pick::Cluster(place.repo.clone()),
            None => Pick::Session(id.to_string()),
        }
    }

    /// The sidebar gets focus: the keys start on the shown session.
    pub fn focus(&mut self, current: &str) {
        let at = self
            .visual_pos(Some(&self.row_of(current)))
            .or((!self.visual.is_empty()).then_some(0));
        self.selected = at.map(|p| self.visual[p].clone());
    }

    fn move_to(&mut self, pos: usize) {
        if let Some(pick) = self.visual.get(pos) {
            self.selected = Some(pick.clone());
        }
    }

    /// The selection is on the list's last row: Down goes on to what is under the list.
    pub fn at_end(&self) -> bool {
        let at = self.visual_pos(self.selected.as_ref()).unwrap_or(0);
        at + 1 >= self.visual.len()
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
        // On `+ New chat`, before the first group.
        let current = self
            .group_starts
            .iter()
            .rposition(|&s| s <= at)
            .map_or(-1, |g| g as isize);
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
            KeyCode::Char('n') => return SessionsAction::New,
            KeyCode::Enter => match self.selected.clone() {
                Some(Pick::New) => return SessionsAction::New,
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
                Some(Pick::Cluster(repo)) => {
                    if !self.folded.remove(&repo) {
                        self.folded.insert(repo);
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
        while from > 0 && !self.entries[from - 1].is_row() {
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
}

#[cfg(test)]
mod tests;
