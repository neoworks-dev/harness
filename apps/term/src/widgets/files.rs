//! The files panel: what git says changed, plus what the agent and its guests touched.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use suite_renderer_ratatui::Palette;

use super::git::{Change, GitStatus};
use super::text::{clean_line, truncate_start, width};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
    /// Touched by an agent, but git sees no difference (committed since).
    Clean,
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
            Mark::Clean => '·',
        }
    }

    fn style(self, palette: &Palette) -> Style {
        let fg = |c: Color| Style::new().fg(c);
        match self {
            Mark::Modified | Mark::Renamed => fg(palette.accent),
            Mark::Added => fg(palette.success),
            Mark::Deleted => fg(palette.danger),
            Mark::Untracked | Mark::Clean => fg(palette.muted),
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
        let keep = self.selected().map(|e| e.rel.clone());
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
                    rel,
                });
            }
        }
        for t in &self.touched {
            let listed = entries
                .iter()
                .any(|e| e.rel == t.rel || (e.rel.ends_with('/') && t.rel.starts_with(&e.rel)));
            if !listed {
                let mark = match (&self.status, t.added) {
                    (Some(_), _) => Mark::Clean,
                    (None, true) => Mark::Added,
                    (None, false) => Mark::Modified,
                };
                entries.push(FileEntry {
                    rel: clean_line(&t.rel),
                    orig: None,
                    mark,
                    tags: t.tags.clone(),
                });
            }
        }
        self.entries = entries;
        self.selected = keep
            .and_then(|k| self.entries.iter().position(|e| e.rel == k))
            .unwrap_or(self.selected)
            .min(self.entries.len().saturating_sub(1));
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    pub(crate) fn selected(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected)
    }

    pub(crate) fn in_repo(&self) -> bool {
        self.status.is_some()
    }

    pub(crate) fn branch(&self) -> Option<String> {
        self.status.as_ref().map(GitStatus::branch_label)
    }

    /// Whether the panel has anything to list.
    pub(crate) fn has_any(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Lines the whole panel (without its title) takes.
    pub(crate) fn natural_height(&self) -> usize {
        usize::from(self.status.is_some()) + self.entries.len()
    }

    pub(crate) fn key(&mut self, key: KeyEvent) -> FileKey {
        let last = self.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return FileKey::Leave,
            KeyCode::Enter if !self.entries.is_empty() => return FileKey::Open,
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

    /// At most `height` lines: the branch, the entries, `… N more` when they don't fit.
    pub(crate) fn render(
        &self,
        width_cells: usize,
        height: usize,
        focused: bool,
        palette: &Palette,
    ) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut rows = height;
        if let Some(status) = &self.status {
            if height == 0 {
                return out;
            }
            out.push(Line::from(Span::styled(
                truncate_start(&clean_line(&status.branch_label()), width_cells),
                Style::new().fg(palette.muted),
            )));
            rows -= 1;
        }
        let total = self.entries.len();
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
        for (i, e) in self.entries.iter().enumerate().skip(offset).take(visible) {
            out.push(row(e, width_cells, focused && i == self.selected, palette));
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

fn row(e: &FileEntry, w: usize, selected: bool, palette: &Palette) -> Line<'static> {
    let style = e.mark.style(palette);
    let text = match &e.orig {
        Some(o) => format!("{o} → {}", e.rel),
        None => e.rel.clone(),
    };
    let avail = w.saturating_sub(2);
    let tag = if e.tags.is_empty() {
        String::new()
    } else {
        format!(" ({})", e.tags.join(", "))
    };
    let mut spans = vec![Span::styled(format!("{} ", e.mark.glyph()), style)];
    if width(&text) + width(&tag) <= avail {
        spans.push(Span::styled(text, style));
        if !tag.is_empty() {
            spans.push(Span::styled(tag, Style::new().fg(palette.muted)));
        }
    } else {
        spans.push(Span::styled(truncate_start(&text, avail), style));
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

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn git_entries_with_agent_tags_and_touched_only() {
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
                ("gone.txt", Mark::Clean, vec!["claude".to_string()]),
            ]
        );
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
                "main ↑1",
                "M a.ts (agent)",
                "A b.ts",
                "D c.ts",
                "R old.rs → new name.rs",
                "? d.md",
                "U e.txt",
            ]
        );
        assert_eq!(lines[1].spans[1].style.fg, Some(p.accent));
        assert_eq!(lines[1].spans[2].style.fg, Some(p.muted));
        assert_eq!(lines[2].spans[1].style.fg, Some(p.success));
        assert_eq!(lines[3].spans[1].style.fg, Some(p.danger));
        assert_eq!(lines[5].spans[1].style.fg, Some(p.muted));
        assert_eq!(lines[6].spans[1].style.fg, Some(p.danger));
        assert!(
            lines[6].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        // Cap.
        let capped = text(&f.render(30, 4, false, &p));
        assert_eq!(capped, ["main ↑1", "M a.ts (agent)", "A b.ts", "… 4 more"]);
        // Too narrow: tag first, then the front of the path.
        let narrow = text(&f.render(12, 20, false, &p));
        assert_eq!(narrow[1], "M a.ts");
        assert_eq!(narrow[4], "R …w name.rs");
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
        assert_eq!(trimmed, ["main ↑1", "M f4", "M f5", "… 4 more"]);
        let selected = &lines[2].spans;
        assert_eq!(selected[0].style.bg, Some(p.surface));
        assert_eq!(lines[1].spans[0].style.bg, None);
        assert_eq!(text(&f.render(10, 4, false, &p))[1], "M f0");
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
