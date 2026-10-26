//! The files panel: what git says changed, plus what the agent and its guests touched.

use std::collections::{BTreeMap, HashSet};

use neo_renderer_ratatui::Palette;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::git::{Change, GitStatus, LineCount};
use super::text::{clean_line, truncate_start, width};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl Mark {
    fn glyph(self) -> char {
        match self {
            Mark::Modified => 'M',
            Mark::Added => 'A',
            Mark::Deleted => 'D',
            Mark::Renamed => 'R',
            Mark::Untracked => '?',
            Mark::Conflicted => 'U',
        }
    }

    fn style(self, palette: &Palette) -> Style {
        let fg = |c: Color| Style::new().fg(c);
        match self {
            Mark::Modified | Mark::Renamed => fg(palette.accent),
            Mark::Added => fg(palette.success),
            Mark::Deleted => fg(palette.danger),
            Mark::Untracked => fg(palette.muted),
            Mark::Conflicted => fg(palette.danger).add_modifier(Modifier::BOLD),
        }
    }
}

impl From<Change> for Mark {
    fn from(c: Change) -> Mark {
        match c {
            Change::Modified => Mark::Modified,
            Change::Added => Mark::Added,
            Change::Deleted => Mark::Deleted,
            Change::Renamed => Mark::Renamed,
            Change::Untracked => Mark::Untracked,
            Change::Conflicted => Mark::Conflicted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileEntry {
    /// Relative to the session directory when inside it.
    pub rel: String,
    pub orig: Option<String>,
    pub mark: Mark,
    /// Who changed it in this session: "agent", "claude"…
    pub tags: Vec<String>,
    /// What git counts; none for binary files, directories and files git sees no change in.
    pub lines: Option<LineCount>,
}

#[derive(Debug, Clone)]
struct Touched {
    rel: String,
    tags: Vec<String>,
    /// The first change created the file.
    added: bool,
}

/// What a key in the panel asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileKey {
    Handled,
    Open,
    Leave,
}

#[derive(Debug, Default)]
pub(crate) struct FilesState {
    /// None: not a repository (or not loaded yet).
    status: Option<GitStatus>,
    touched: Vec<Touched>,
    entries: Vec<FileEntry>,
    /// The tree as drawn; `selected` is an index into it.
    rows: Vec<TreeRow>,
    /// Folders folded away, by path.
    collapsed: HashSet<String>,
    /// Folders opened by hand that would start folded (more than `FOLD_FILES` in them).
    opened: HashSet<String>,
    selected: usize,
}

impl FilesState {
    /// A refresh arrived. True when it changes what is shown.
    pub(crate) fn set_status(&mut self, status: Option<GitStatus>) -> bool {
        if self.status == status {
            return false;
        }
        self.status = status;
        self.rebuild();
        true
    }

    /// An agent changed `rel` (as the diff event named it, made relative to the directory).
    pub(crate) fn touch(&mut self, rel: &str, tag: &str, added: bool) {
        match self.touched.iter_mut().find(|t| t.rel == rel) {
            Some(t) => {
                if !t.tags.iter().any(|x| x == tag) {
                    t.tags.push(tag.to_string());
                }
            }
            None => self.touched.push(Touched {
                rel: rel.to_string(),
                tags: vec![tag.to_string()],
                added,
            }),
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let keep = self.rows.get(self.selected).map(|r| r.key(&self.entries));
        let tags_for = |rel: &str| -> Vec<String> {
            let mut tags: Vec<String> = Vec::new();
            for t in &self.touched {
                let hit = t.rel == rel || (rel.ends_with('/') && t.rel.starts_with(rel));
                if hit {
                    for tag in &t.tags {
                        if !tags.contains(tag) {
                            tags.push(tag.clone());
                        }
                    }
                }
            }
            tags
        };
        let mut entries = Vec::new();
        if let Some(status) = &self.status {
            for e in &status.entries {
                let rel = clean_line(&status.rel(&e.path));
                entries.push(FileEntry {
                    tags: tags_for(&rel),
                    orig: e.orig.as_deref().map(|o| clean_line(&status.rel(o))),
                    mark: e.change.into(),
                    lines: status.lines.get(&e.path).copied(),
                    rel,
                });
            }
        }
        for t in &self.touched {
            let listed = entries
                .iter()
                .any(|e| e.rel == t.rel || (e.rel.ends_with('/') && t.rel.starts_with(&e.rel)));
            // In a repository, a file git sees no change in (committed, or changed back) is done.
            if !listed && self.status.is_none() {
                let mark = if t.added { Mark::Added } else { Mark::Modified };
                entries.push(FileEntry {
                    rel: clean_line(&t.rel),
                    orig: None,
                    mark,
                    tags: t.tags.clone(),
                    lines: None,
                });
            }
        }
        self.entries = entries;
        self.rebuild_rows();
        self.selected = keep
            .and_then(|k| self.rows.iter().position(|r| r.key(&self.entries) == k))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
    }

    /// The tree's rows from the entries and which folders are collapsed.
    fn rebuild_rows(&mut self) {
        let mut root = Dir::default();
        for (i, e) in self.entries.iter().enumerate() {
            let trimmed = e.rel.trim_end_matches('/');
            let mut parts: Vec<&str> = trimmed.split('/').collect();
            parts.pop();
            let mut node = &mut root;
            let mut path = String::new();
            for part in parts {
                path.push_str(part);
                path.push('/');
                node = node.dirs.entry(part.to_string()).or_insert_with(|| Dir {
                    path: path.clone(),
                    ..Dir::default()
                });
            }
            node.files.push(i);
        }
        let mut rows = Vec::new();
        let folds = Folds {
            collapsed: &self.collapsed,
            opened: &self.opened,
        };
        flatten(&root, 0, &self.entries, &folds, &mut rows);
        self.rows = rows;
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// The selected file; None when a folder (or nothing) is selected.
    pub(crate) fn selected(&self) -> Option<&FileEntry> {
        match self.rows.get(self.selected)?.kind {
            RowKind::File(i) => self.entries.get(i),
            RowKind::Dir { .. } => None,
        }
    }

    pub(crate) fn in_repo(&self) -> bool {
        self.status.is_some()
    }

    pub(crate) fn branch(&self) -> Option<String> {
        self.status.as_ref().map(GitStatus::branch_label)
    }

    /// Whether the list is worth a column: something besides untracked files no agent touched
    /// (a `.claude/` lying around is not news).
    pub(crate) fn worth_showing(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.mark != Mark::Untracked || !e.tags.is_empty())
    }

    /// Cells the widest row takes.
    pub(crate) fn natural_width(&self, palette: &Palette) -> usize {
        self.rows
            .iter()
            .map(|r| row_width(r, &self.entries, palette))
            .max()
            .unwrap_or(0)
    }

    /// Lines the whole panel (without its title) takes.
    pub(crate) fn natural_height(&self) -> usize {
        usize::from(self.status.is_some()) + self.rows.len()
    }

    /// Folds or unfolds the folder at row `i`.
    fn set_collapsed(&mut self, i: usize, collapse: bool) {
        let Some(TreeRow {
            kind: RowKind::Dir { path, .. },
            ..
        }) = self.rows.get(i)
        else {
            return;
        };
        let path = path.clone();
        if collapse {
            self.opened.remove(&path);
            self.collapsed.insert(path);
        } else {
            self.collapsed.remove(&path);
            self.opened.insert(path);
        }
        self.rebuild_rows();
    }

    pub(crate) fn key(&mut self, key: KeyEvent) -> FileKey {
        let last = self.rows.len().saturating_sub(1);
        let row = self.rows.get(self.selected).cloned();
        let open_dir = match &row {
            Some(TreeRow {
                kind: RowKind::Dir { collapsed, .. },
                ..
            }) => Some(!*collapsed),
            _ => None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return FileKey::Leave,
            KeyCode::Enter | KeyCode::Char(' ') => match open_dir {
                Some(open) => self.set_collapsed(self.selected, open),
                None if row.is_some() => return FileKey::Open,
                None => {}
            },
            KeyCode::Right | KeyCode::Char('l') => match open_dir {
                Some(false) => self.set_collapsed(self.selected, false),
                Some(true) => self.selected = (self.selected + 1).min(last),
                None => {}
            },
            KeyCode::Left | KeyCode::Char('h') => match (open_dir, &row) {
                (Some(true), _) => self.set_collapsed(self.selected, true),
                (_, Some(r)) if r.depth > 0 => {
                    // Up to the folder this row is in.
                    if let Some(parent) = self.rows[..self.selected]
                        .iter()
                        .rposition(|p| p.depth < r.depth)
                    {
                        self.selected = parent;
                    }
                }
                _ => {}
            },
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => self.selected = (self.selected + 10).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = last,
            _ => {}
        }
        FileKey::Handled
    }

    /// At most `height` lines: the tree, `… N more` when it doesn't fit (the branch is in the
    /// status line).
    pub(crate) fn render(
        &self,
        width_cells: usize,
        height: usize,
        focused: bool,
        palette: &Palette,
    ) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let rows = height;
        let total = self.rows.len();
        let (visible, more) = if total > rows && rows >= 2 {
            (rows - 1, true)
        } else {
            (rows.min(total), false)
        };
        let offset = if focused && self.selected >= visible {
            self.selected + 1 - visible
        } else {
            0
        };
        for (i, r) in self.rows.iter().enumerate().skip(offset).take(visible) {
            let selected = focused && i == self.selected;
            out.push(tree_row(r, &self.entries, width_cells, selected, palette));
        }
        if more {
            out.push(Line::from(Span::styled(
                format!("… {} more", total - visible),
                Style::new().fg(palette.muted),
            )));
        }
        out
    }
}

/// A folder while the tree is built.
#[derive(Debug, Default)]
struct Dir {
    /// From the session directory, with a trailing `/`.
    path: String,
    dirs: BTreeMap<String, Dir>,
    /// Indexes into the entries.
    files: Vec<usize>,
}

impl Dir {
    fn file_count(&self) -> usize {
        self.files.len() + self.dirs.values().map(Dir::file_count).sum::<usize>()
    }

    /// What changed in every file below, summed.
    fn counts(&self, entries: &[FileEntry]) -> Option<LineCount> {
        let mut sum: Option<LineCount> = None;
        let mut add = |c: Option<LineCount>| {
            if let Some(c) = c {
                let s = sum.get_or_insert_with(LineCount::default);
                s.added += c.added;
                s.removed += c.removed;
            }
        };
        for &i in &self.files {
            add(entries.get(i).and_then(|e| e.lines));
        }
        for d in self.dirs.values() {
            add(d.counts(entries));
        }
        sum
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RowKind {
    Dir {
        path: String,
        /// What is shown: folders with one folder in them and nothing else are joined
        /// (`apps/term/src/`).
        name: String,
        collapsed: bool,
        counts: Option<LineCount>,
        /// Files below it, all levels.
        files: usize,
    },
    /// An index into the entries.
    File(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeRow {
    depth: usize,
    kind: RowKind,
}

impl TreeRow {
    /// What keeps the selection on a row across rebuilds.
    fn key(&self, entries: &[FileEntry]) -> String {
        match &self.kind {
            RowKind::Dir { path, .. } => format!("d:{path}"),
            RowKind::File(i) => format!("f:{}", entries.get(*i).map_or("", |e| e.rel.as_str())),
        }
    }
}

/// The rows below `dir`: its folders first, then its files, each by name.
/// Which folders are folded: the ones folded by hand, and the ones with more than
/// `FOLD_FILES` files right in them (generated files, snapshots) unless opened by hand.
struct Folds<'a> {
    collapsed: &'a HashSet<String>,
    opened: &'a HashSet<String>,
}

impl Folds<'_> {
    fn folded(&self, dir: &Dir) -> bool {
        self.collapsed.contains(&dir.path)
            || (dir.files.len() > FOLD_FILES && !self.opened.contains(&dir.path))
    }
}

fn flatten(dir: &Dir, depth: usize, entries: &[FileEntry], folds: &Folds, rows: &mut Vec<TreeRow>) {
    for child in dir.dirs.values() {
        let mut node = child;
        let mut name = child.path[dir.path.len()..].to_string();
        while node.files.is_empty() && node.dirs.len() == 1 {
            let Some(only) = node.dirs.values().next() else {
                break;
            };
            name.push_str(&only.path[node.path.len()..]);
            node = only;
        }
        let folded = folds.folded(node);
        rows.push(TreeRow {
            depth,
            kind: RowKind::Dir {
                path: node.path.clone(),
                name,
                collapsed: folded,
                counts: node.counts(entries),
                files: node.file_count(),
            },
        });
        if !folded {
            flatten(node, depth + 1, entries, folds, rows);
        }
    }
    let mut files = dir.files.clone();
    files.sort_by(|a, b| {
        let name = |i: &usize| entries.get(*i).map(|e| e.rel.to_lowercase());
        name(a).cmp(&name(b))
    });
    rows.extend(files.into_iter().map(|i| TreeRow {
        depth,
        kind: RowKind::File(i),
    }));
}

/// `+12 −3` (a zero side left out), in the success and danger colors.
fn line_counts(lines: Option<LineCount>, palette: &Palette) -> Vec<Span<'static>> {
    let Some(LineCount { added, removed }) = lines else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    if added > 0 {
        spans.push(Span::styled(
            format!("+{added}"),
            Style::new().fg(palette.success),
        ));
    }
    if removed > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("−{removed}"),
            Style::new().fg(palette.danger),
        ));
    }
    spans
}

/// Below this many cells for the name, the line counts are left out.
const MIN_NAME: usize = 8;
/// A folder with more files than this right in it starts folded.
const FOLD_FILES: usize = 5;

/// The file's name in the tree: the last part of its path (`new/` for a new folder), a rename
/// as `old → new`.
fn file_name(e: &FileEntry) -> String {
    let base = |p: &str| {
        let trimmed = p.trim_end_matches('/');
        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
        if p.ends_with('/') {
            format!("{name}/")
        } else {
            name.to_string()
        }
    };
    match &e.orig {
        Some(o) => format!("{o} → {}", base(&e.rel)),
        None => base(&e.rel),
    }
}

/// A row's pieces before fitting: the lead (indent and glyph), the name and its style, the tag
/// after it and the line counts.
type Pieces = (Span<'static>, String, Style, String, Option<LineCount>);

fn pieces(r: &TreeRow, entries: &[FileEntry], palette: &Palette) -> Option<Pieces> {
    let indent = "  ".repeat(r.depth);
    Some(match &r.kind {
        RowKind::Dir {
            name,
            collapsed,
            counts,
            files,
            ..
        } => {
            let arrow = if *collapsed { "▸ " } else { "▾ " };
            // An open folder's files carry its counts; a folded one says how many there are.
            let lines = if *collapsed { *counts } else { None };
            let tag = if *collapsed {
                format!(" {files} file{}", if *files == 1 { "" } else { "s" })
            } else {
                String::new()
            };
            let lead = Span::styled(format!("{indent}{arrow}"), Style::new().fg(palette.muted));
            (lead, name.clone(), Style::new(), tag, lines)
        }
        RowKind::File(i) => {
            let e = entries.get(*i)?;
            let style = e.mark.style(palette);
            let lead = Span::styled(format!("{indent}{} ", e.mark.glyph()), style);
            let tag = if e.tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", e.tags.join(", "))
            };
            (lead, file_name(e), style, tag, e.lines)
        }
    })
}

/// Cells a row takes when nothing is cut.
fn row_width(r: &TreeRow, entries: &[FileEntry], palette: &Palette) -> usize {
    let Some((lead, text, _, tag, lines)) = pieces(r, entries, palette) else {
        return 0;
    };
    let counts: usize = line_counts(lines, palette)
        .iter()
        .map(|s| width(&s.content))
        .sum();
    width(&lead.content) + width(&text) + width(&tag) + if counts > 0 { counts + 1 } else { 0 }
}

fn tree_row(
    r: &TreeRow,
    entries: &[FileEntry],
    w: usize,
    selected: bool,
    palette: &Palette,
) -> Line<'static> {
    let Some((lead, text, style, tag, lines)) = pieces(r, entries, palette) else {
        return Line::default();
    };
    // The counts sit at the right edge, a space away from the name.
    let lead_w = width(&lead.content);
    let mut counts = line_counts(lines, palette);
    let mut counts_w: usize = counts.iter().map(|s| width(&s.content)).sum();
    if counts_w > 0 && w.saturating_sub(lead_w + counts_w + 1) < MIN_NAME {
        counts.clear();
        counts_w = 0;
    }
    let avail = w.saturating_sub(lead_w + if counts_w > 0 { counts_w + 1 } else { 0 });
    let mut spans = vec![lead];
    if width(&text) + width(&tag) <= avail {
        spans.push(Span::styled(text, style));
        if !tag.is_empty() {
            spans.push(Span::styled(tag, Style::new().fg(palette.muted)));
        }
    } else {
        spans.push(Span::styled(truncate_start(&text, avail), style));
    }
    if counts_w > 0 {
        let used: usize = spans.iter().map(|s| width(&s.content)).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used + counts_w))));
        spans.extend(counts);
    }
    if selected {
        let used: usize = spans.iter().map(|s| width(&s.content)).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        for s in &mut spans {
            s.style = s.style.bg(palette.surface).add_modifier(Modifier::BOLD);
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::git::{GitEntry, parse_status};

    fn status(entries: Vec<GitEntry>) -> GitStatus {
        GitStatus {
            head: Some("main".into()),
            ahead: 1,
            entries,
            ..GitStatus::default()
        }
    }

    fn entry(path: &str, change: Change) -> GitEntry {
        GitEntry {
            path: path.into(),
            orig: None,
            change,
        }
    }

    #[test]
    fn line_counts_sit_at_the_right_edge() {
        let mut f = FilesState::default();
        let mut s = status(vec![
            entry("src/a.rs", Change::Modified),
            entry("gone.rs", Change::Deleted),
            entry("logo.png", Change::Added),
        ]);
        s.lines.insert(
            "src/a.rs".into(),
            LineCount {
                added: 12,
                removed: 3,
            },
        );
        s.lines.insert(
            "gone.rs".into(),
            LineCount {
                added: 0,
                removed: 40,
            },
        );
        f.set_status(Some(s));
        let p = Palette::default();
        assert_eq!(
            text(&f.render(24, 10, false, &p)),
            [
                "▾ src/",
                "  M a.rs          +12 −3",
                "D gone.rs            −40",
                "A logo.png",
            ]
        );
        let line = f.render(24, 10, false, &p).remove(1);
        let plus = line.spans.iter().find(|s| s.content == "+12").unwrap();
        assert_eq!(plus.style.fg, Some(p.success));
        // Too narrow for both: the name stays, the counts go.
        assert_eq!(text(&f.render(14, 10, false, &p))[1], "  M a.rs");
        // A folded folder carries the counts of what is in it.
        f.key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            text(&f.render(24, 10, false, &p))[0],
            "▸ src/ 1 file     +12 −3"
        );
    }

    #[test]
    fn untracked_files_alone_are_not_worth_a_column_and_rows_say_how_wide_they_are() {
        let mut f = FilesState::default();
        let p = Palette::default();
        f.set_status(Some(status(vec![
            entry(".claude/", Change::Untracked),
            entry(".serena/", Change::Untracked),
        ])));
        assert!(!f.worth_showing());
        // An agent touching one makes it news.
        f.touch(".claude/settings.json", "claude", true);
        assert!(f.worth_showing());
        assert_eq!(f.natural_width(&p), "? .claude/ (claude)".len());

        let mut s = status(vec![entry("src/a.rs", Change::Modified)]);
        s.lines.insert(
            "src/a.rs".into(),
            LineCount {
                added: 12,
                removed: 3,
            },
        );
        let mut f = FilesState::default();
        f.set_status(Some(s));
        assert!(f.worth_showing());
        // "  M a.rs +12 −3", the counts a space after the name.
        assert_eq!(f.natural_width(&p), 15);
    }

    #[test]
    fn a_folder_with_many_changed_files_starts_folded_until_opened() {
        let mut f = FilesState::default();
        let mut entries: Vec<_> = (0..6)
            .map(|i| entry(&format!("fixtures/t/{i}.txt"), Change::Modified))
            .collect();
        entries.push(entry("src/a.rs", Change::Modified));
        f.set_status(Some(status(entries)));
        let shown = |f: &FilesState| text(&f.render(40, 20, false, &Palette::default()));
        assert_eq!(shown(&f), ["▸ fixtures/t/ 6 files", "▾ src/", "  M a.rs"]);
        f.key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(shown(&f).len(), 9, "opened by hand it stays open");
        f.key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(shown(&f)[0], "▸ fixtures/t/ 6 files");
    }

    #[test]
    fn a_tree_that_folds_and_is_walked_with_arrows() {
        let p = Palette::default();
        let mut f = FilesState::default();
        f.set_status(Some(status(vec![
            entry("apps/term/src/app.rs", Change::Modified),
            entry("apps/term/src/menu.rs", Change::Modified),
            entry("apps/term/README.md", Change::Modified),
            entry("Cargo.lock", Change::Modified),
        ])));
        // A folder holding only one folder is joined with it.
        assert_eq!(
            text(&f.render(30, 10, false, &p)),
            [
                "▾ apps/term/",
                "  ▾ src/",
                "    M app.rs",
                "    M menu.rs",
                "  M README.md",
                "M Cargo.lock",
            ]
        );
        let key = |c| KeyEvent::from(c);
        // On a folder Enter folds it; Right opens it again and then goes in.
        assert!(f.selected().is_none(), "apps/term/ is a folder");
        assert_eq!(f.key(key(KeyCode::Enter)), FileKey::Handled);
        assert_eq!(
            text(&f.render(30, 10, false, &p)),
            ["▸ apps/term/ 3 files", "M Cargo.lock"]
        );
        f.key(key(KeyCode::Right)); // opens apps/term/
        f.key(key(KeyCode::Right)); // into it: src/
        f.key(key(KeyCode::Right)); // into src/: app.rs
        assert_eq!(f.selected().unwrap().rel, "apps/term/src/app.rs");
        // Left goes up to the folder, then folds it; Enter on a file opens it.
        f.key(key(KeyCode::Left));
        f.key(key(KeyCode::Left));
        assert_eq!(text(&f.render(30, 10, false, &p))[1], "  ▸ src/ 2 files");
        f.key(key(KeyCode::Char('G')));
        assert_eq!(f.key(key(KeyCode::Enter)), FileKey::Open);
        // The fold survives a refresh.
        f.set_status(f.status.clone().map(|mut s| {
            s.ahead = 2;
            s
        }));
        assert_eq!(text(&f.render(30, 10, false, &p))[1], "  ▸ src/ 2 files");
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn git_entries_with_agent_tags_and_committed_ones_left_out() {
        let mut f = FilesState::default();
        f.touch("src/a.ts", "agent", false);
        f.touch("src/a.ts", "claude", false);
        f.touch("gone.txt", "claude", true);
        assert_eq!(f.entries().len(), 2);
        assert_eq!(f.entries()[0].mark, Mark::Modified);
        assert_eq!(f.entries()[1].mark, Mark::Added);
        assert!(!f.in_repo());
        f.set_status(Some(status(vec![
            entry("src/a.ts", Change::Modified),
            entry("b.txt", Change::Untracked),
        ])));
        let got: Vec<_> = f
            .entries()
            .iter()
            .map(|e| (e.rel.as_str(), e.mark, e.tags.clone()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "src/a.ts",
                    Mark::Modified,
                    vec!["agent".to_string(), "claude".to_string()]
                ),
                ("b.txt", Mark::Untracked, vec![]),
            ]
        );
        // gone.txt: touched, but git sees no change in it any more (committed), so it is done.
        assert_eq!(f.branch().as_deref(), Some("main ↑1"));
        // Same status again is no change.
        assert!(!f.set_status(f.status.clone()));
    }

    #[test]
    fn untracked_directory_takes_the_tags_of_files_in_it() {
        let mut f = FilesState::default();
        f.touch("new/x.rs", "agent", true);
        f.set_status(Some(status(vec![entry("new/", Change::Untracked)])));
        assert_eq!(f.entries().len(), 1);
        assert_eq!(f.entries()[0].tags, ["agent"]);
    }

    #[test]
    fn paths_are_relative_to_the_session_directory() {
        let mut f = FilesState::default();
        let mut s = parse_status(b"# branch.head main\0? src/x.rs\0? top.md\0");
        s.prefix = "src/".into();
        f.set_status(Some(s));
        let rels: Vec<_> = f.entries().iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(rels, ["x.rs", "../top.md"]);
    }

    #[test]
    fn rows_color_by_mark_and_cap_with_more() {
        let p = Palette::default();
        let mut f = FilesState::default();
        let mut old = entry("new name.rs", Change::Renamed);
        old.orig = Some("old.rs".into());
        let all = vec![
            entry("a.ts", Change::Modified),
            entry("b.ts", Change::Added),
            entry("c.ts", Change::Deleted),
            old,
            entry("d.md", Change::Untracked),
            entry("e.txt", Change::Conflicted),
        ];
        f.set_status(Some(status(all)));
        f.touch("a.ts", "agent", false);
        let lines = f.render(30, 20, false, &p);
        assert_eq!(
            text(&lines),
            [
                "M a.ts (agent)",
                "A b.ts",
                "D c.ts",
                "? d.md",
                "U e.txt",
                "R old.rs → new name.rs",
            ]
        );
        assert_eq!(lines[0].spans[1].style.fg, Some(p.accent));
        assert_eq!(lines[0].spans[2].style.fg, Some(p.muted));
        assert_eq!(lines[1].spans[1].style.fg, Some(p.success));
        assert_eq!(lines[2].spans[1].style.fg, Some(p.danger));
        assert_eq!(lines[3].spans[1].style.fg, Some(p.muted));
        assert_eq!(lines[4].spans[1].style.fg, Some(p.danger));
        assert!(
            lines[4].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        // Cap.
        let capped = text(&f.render(30, 4, false, &p));
        assert_eq!(capped, ["M a.ts (agent)", "A b.ts", "D c.ts", "… 3 more"]);
        // Too narrow: tag first, then the front of the path.
        let narrow = text(&f.render(12, 20, false, &p));
        assert_eq!(narrow[0], "M a.ts");
        assert_eq!(narrow[5], "R …w name.rs");
    }

    #[test]
    fn selection_moves_and_scrolls_with_focus() {
        let p = Palette::default();
        let mut f = FilesState::default();
        f.set_status(Some(status(
            (0..6)
                .map(|i| entry(&format!("f{i}"), Change::Modified))
                .collect(),
        )));
        let key = |c| KeyEvent::from(c);
        assert_eq!(f.key(key(KeyCode::Up)), FileKey::Handled);
        assert_eq!(f.selected().unwrap().rel, "f0");
        f.key(key(KeyCode::Char('j')));
        f.key(key(KeyCode::Down));
        assert_eq!(f.selected().unwrap().rel, "f2");
        f.key(key(KeyCode::Char('k')));
        assert_eq!(f.selected().unwrap().rel, "f1");
        f.key(key(KeyCode::Char('G')));
        assert_eq!(f.selected().unwrap().rel, "f5");
        f.key(key(KeyCode::Down));
        assert_eq!(f.selected().unwrap().rel, "f5");
        // Window follows the selection when focused; the selected row is highlighted.
        let lines = f.render(10, 4, true, &p);
        let trimmed: Vec<_> = text(&lines)
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert_eq!(trimmed, ["M f3", "M f4", "M f5", "… 3 more"]);
        let selected = &lines[2].spans;
        assert_eq!(selected[0].style.bg, Some(p.surface));
        assert_eq!(lines[1].spans[0].style.bg, None);
        assert_eq!(text(&f.render(10, 4, false, &p))[0], "M f0");
        f.key(key(KeyCode::Char('g')));
        assert_eq!(f.key(key(KeyCode::Enter)), FileKey::Open);
        assert_eq!(f.key(key(KeyCode::Esc)), FileKey::Leave);
    }

    #[test]
    fn selection_survives_a_refresh_by_path() {
        let mut f = FilesState::default();
        f.set_status(Some(status(vec![
            entry("a", Change::Modified),
            entry("b", Change::Modified),
            entry("c", Change::Modified),
        ])));
        f.key(KeyEvent::from(KeyCode::Char('G')));
        f.set_status(Some(status(vec![
            entry("0", Change::Modified),
            entry("a", Change::Modified),
            entry("c", Change::Modified),
        ])));
        assert_eq!(f.selected().unwrap().rel, "c");
        f.set_status(Some(status(vec![entry("0", Change::Modified)])));
        assert_eq!(f.selected().unwrap().rel, "0");
        f.set_status(Some(status(vec![])));
        assert!(f.selected().is_none());
        assert_eq!(f.key(KeyEvent::from(KeyCode::Enter)), FileKey::Handled);
    }
}
