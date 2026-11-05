//! The files panel: what git says changed, plus what the agent and its guests touched.

use std::collections::{BTreeMap, HashSet};

use neo_renderer_ratatui::{Palette, file_icon};
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
    /// An agent changed it in this session and git sees no change in it any more: committed
    /// (or changed back).
    Committed,
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
            Mark::Committed => '✓',
        }
    }

    fn style(self, palette: &Palette) -> Style {
        let fg = |c: Color| Style::new().fg(c);
        match self {
            Mark::Modified | Mark::Renamed => fg(palette.accent),
            Mark::Added => fg(palette.success),
            Mark::Deleted => fg(palette.danger),
            Mark::Untracked | Mark::Committed => fg(palette.muted),
            Mark::Conflicted => fg(palette.danger).add_modifier(Modifier::BOLD),
        }
    }

    /// The glyph's own style: a committed file's check is green, its name muted.
    fn glyph_style(self, palette: &Palette) -> Style {
        match self {
            Mark::Committed => Style::new().fg(palette.success),
            _ => self.style(palette),
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
    /// Rows show the file type icon.
    pub(crate) icons: bool,
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
            // In a repository, a file git sees no change in was committed (or changed back).
            if !listed {
                let mark = if self.status.is_some() {
                    Mark::Committed
                } else if t.added {
                    Mark::Added
                } else {
                    Mark::Modified
                };
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

    /// The tree's rows from the entries and which folders are collapsed. Untracked files no
    /// agent touched go under one `untracked` row at the end, folded until opened.
    fn rebuild_rows(&mut self) {
        let mut root = Dir::default();
        let mut untracked = Dir {
            path: UNTRACKED.into(),
            ..Dir::default()
        };
        for (i, e) in self.entries.iter().enumerate() {
            let trimmed = e.rel.trim_end_matches('/');
            let mut parts: Vec<&str> = trimmed.split('/').collect();
            parts.pop();
            let mut node = if e.mark == Mark::Untracked && e.tags.is_empty() {
                &mut untracked
            } else {
                &mut root
            };
            let mut path = node.path.clone();
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
        let files = untracked.file_count();
        if files > 0 {
            let folded = folds.folded(&untracked);
            rows.push(TreeRow {
                depth: 0,
                kind: RowKind::Dir {
                    path: UNTRACKED.into(),
                    name: "untracked".into(),
                    collapsed: folded,
                    counts: untracked.counts(&self.entries),
                    files,
                },
            });
            if !folded {
                flatten(&untracked, 1, &self.entries, &folds, &mut rows);
            }
        }
        self.rows = rows;
    }

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
            .map(|r| row_width(r, &self.entries, palette, self.icons))
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
            out.push(tree_row(
                r,
                &self.entries,
                width_cells,
                selected,
                palette,
                self.icons,
            ));
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
        let crowded = dir.files.len() > FOLD_FILES || dir.path == UNTRACKED;
        self.collapsed.contains(&dir.path) || (crowded && !self.opened.contains(&dir.path))
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
/// The path of the group the untracked files no agent touched are in (no real path has `?/`
/// at its start).
const UNTRACKED: &str = "?/";

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

/// A row's pieces before fitting: the lead (indent, glyph and the file type icon), the name and
/// its style, the tag after it and the line counts.
type Pieces = (Vec<Span<'static>>, String, Style, String, Option<LineCount>);

fn pieces(r: &TreeRow, entries: &[FileEntry], palette: &Palette, icons: bool) -> Option<Pieces> {
    let (mut lead, text, style, tag, lines) = plain_pieces(r, entries, palette)?;
    if icons {
        let path = match &r.kind {
            RowKind::Dir { path, .. } => path.as_str(),
            RowKind::File(i) => entries.get(*i).map_or("", |e| e.rel.as_str()),
        };
        lead.push(file_icon(path, palette));
        lead.push(Span::raw(" "));
    }
    Some((lead, text, style, tag, lines))
}

fn plain_pieces(r: &TreeRow, entries: &[FileEntry], palette: &Palette) -> Option<Pieces> {
    let indent = "  ".repeat(r.depth);
    Some(match &r.kind {
        RowKind::Dir {
            path,
            name,
            collapsed,
            counts,
            files,
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
            // The untracked group reads like its files: muted.
            let style = if path == UNTRACKED {
                Mark::Untracked.style(palette)
            } else {
                Style::new()
            };
            (vec![lead], name.clone(), style, tag, lines)
        }
        RowKind::File(i) => {
            let e = entries.get(*i)?;
            let style = e.mark.style(palette);
            let lead = Span::styled(
                format!("{indent}{} ", e.mark.glyph()),
                e.mark.glyph_style(palette),
            );
            let mut said = e.tags.clone();
            if e.mark == Mark::Committed {
                said.push("committed".into());
            }
            let tag = if said.is_empty() {
                String::new()
            } else {
                format!(" ({})", said.join(", "))
            };
            (vec![lead], file_name(e), style, tag, e.lines)
        }
    })
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// Cells a row takes when nothing is cut.
fn row_width(r: &TreeRow, entries: &[FileEntry], palette: &Palette, icons: bool) -> usize {
    let Some((lead, text, _, tag, lines)) = pieces(r, entries, palette, icons) else {
        return 0;
    };
    let counts = spans_width(&line_counts(lines, palette));
    spans_width(&lead) + width(&text) + width(&tag) + if counts > 0 { counts + 1 } else { 0 }
}

fn tree_row(
    r: &TreeRow,
    entries: &[FileEntry],
    w: usize,
    selected: bool,
    palette: &Palette,
    icons: bool,
) -> Line<'static> {
    let Some((lead, text, style, tag, lines)) = pieces(r, entries, palette, icons) else {
        return Line::default();
    };
    // The counts sit at the right edge, a space away from the name.
    let lead_w = spans_width(&lead);
    let mut counts = line_counts(lines, palette);
    let mut counts_w: usize = counts.iter().map(|s| width(&s.content)).sum();
    if counts_w > 0 && w.saturating_sub(lead_w + counts_w + 1) < MIN_NAME {
        counts.clear();
        counts_w = 0;
    }
    let avail = w.saturating_sub(lead_w + if counts_w > 0 { counts_w + 1 } else { 0 });
    let mut spans = lead;
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
mod tests;
