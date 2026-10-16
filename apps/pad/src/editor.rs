//! The open files and what happens to them: opening, saving, reloading when the file changed on
//! disk, diagnostics. No GPUI in here; things the app has to do afterwards come back as
//! [`Effect`]s.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Pos, display_byte, display_line};

/// Bigger files aren't opened: this is a stand-in, not a text editor for logs.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// What the app does next, after the model changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Ask the lsp service about this file (on a worker) and report with
    /// [`Editor::finish_diagnose`]; call [`Editor::begin_diagnose`] first.
    Diagnose(PathBuf),
    /// Tell the bus the file changed on disk (`file.changed`).
    FileChanged(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    pub fn parse(text: &str) -> Severity {
        match text {
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            "hint" => Severity::Hint,
            _ => Severity::Error,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }
}

/// One finding of a language server. Line and column are 1-based, as the lsp service sends them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DiagState {
    /// Nothing asked yet.
    #[default]
    Idle,
    Checking,
    /// The list is what the language server said about the text on disk.
    Done,
    /// The text was edited since the check: positions may be off.
    Stale,
    /// The lsp service couldn't answer (not running, no server for the language, …).
    Unavailable(String),
}

#[derive(Debug)]
pub struct Doc {
    path: PathBuf,
    canonical: PathBuf,
    pub buffer: Buffer,
    saved_revision: u64,
    /// The file changed on disk while there were unsaved edits.
    pub changed_on_disk: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub diag_state: DiagState,
    /// A line (0-based) the agent pointed at with `reveal`; stays until the cursor moves on.
    pub highlight: Option<usize>,
    language: &'static str,
}

impl Doc {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dirty(&self) -> bool {
        self.buffer.revision() != self.saved_revision
    }

    pub fn language(&self) -> &'static str {
        self.language
    }

    /// The file name for a tab.
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into(),
        )
    }

    /// "modified", "changed on disk", "saved".
    pub fn state(&self) -> &'static str {
        match (self.dirty(), self.changed_on_disk) {
            (true, true) => "modified, changed on disk",
            (true, false) => "modified",
            (false, true) => "changed on disk",
            (false, false) => "saved",
        }
    }

    fn is(&self, path: &Path) -> bool {
        self.path == path || self.canonical == canonical(path)
    }

    /// Diagnostics on a line (0-based).
    pub fn diagnostics_on(&self, line: usize) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(move |d| d.line.saturating_sub(1) == line)
    }

    /// The worst severity on a line, for the gutter.
    pub fn worst_on(&self, line: usize) -> Option<Severity> {
        self.diagnostics_on(line)
            .map(|d| d.severity)
            .min_by_key(|s| match s {
                Severity::Error => 0,
                Severity::Warning => 1,
                Severity::Info => 2,
                Severity::Hint => 3,
            })
    }

    /// What to draw differently on a line, as ranges of the drawn text ([`display_line`]).
    pub fn marks(&self, line: usize) -> Vec<(std::ops::Range<usize>, Mark)> {
        let text = self.buffer.line(line);
        let len = display_line(text).len();
        let mut marks = Vec::new();
        if let Some((from, to)) = self.buffer.selection()
            && from.line <= line
            && line <= to.line
        {
            let start = if from.line == line {
                display_byte(text, from.col)
            } else {
                0
            };
            // A selection running on past the line's end also covers its (drawn) line break.
            let end = if to.line == line {
                display_byte(text, to.col)
            } else {
                len + 1
            };
            if start < end {
                marks.push((start..end, Mark::Selected));
            }
        }
        for d in self.diagnostics_on(line) {
            let start = display_byte(text, d.column.saturating_sub(1)).min(len);
            // To the end of the word the problem starts in (the lsp service sends no end).
            let word: usize = text
                .chars()
                .skip(d.column.saturating_sub(1))
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .map(char::len_utf8)
                .sum();
            let end = (start + word.max(1)).min(len.max(start + 1));
            let mark = match d.severity {
                Severity::Error => Mark::Error,
                Severity::Warning => Mark::Warning,
                Severity::Info | Severity::Hint => Mark::Info,
            };
            marks.push((start..end, mark));
        }
        if self.buffer.cursor().line == line {
            let at = display_byte(text, self.buffer.cursor().col);
            marks.push((at..at + 1, Mark::Cursor));
        }
        marks
    }
}

/// How a stretch of a line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Selected,
    Error,
    Warning,
    Info,
    Cursor,
}

/// Overlapping marks cut into consecutive pieces, each with every mark that covers it.
/// `len` is the length of the drawn text including a trailing space for a caret past the end.
pub fn segments(
    len: usize,
    marks: &[(std::ops::Range<usize>, Mark)],
) -> Vec<(std::ops::Range<usize>, Vec<Mark>)> {
    let mut cuts: Vec<usize> = vec![0, len];
    for (range, _) in marks {
        cuts.push(range.start.min(len));
        cuts.push(range.end.min(len));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for pair in cuts.windows(2) {
        let range = pair[0]..pair[1];
        let covering: Vec<Mark> = marks
            .iter()
            .filter(|(r, _)| r.start <= range.start && range.end <= r.end)
            .map(|(_, m)| *m)
            .collect();
        if !covering.is_empty() {
            out.push((range, covering));
        }
    }
    out
}

/// `bash`-style: the language name for a path, by extension.
pub fn language_of(path: &Path) -> &'static str {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "go" => "go",
        "rs" => "rust",
        "py" => "python",
        "json" | "jsonc" => "json",
        "md" | "markdown" => "markdown",
        "css" => "css",
        "html" | "htm" => "html",
        "svg" | "xml" => "xml",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "zsh" => "shellscript",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        "txt" => "plaintext",
        _ if name == "Makefile" => "makefile",
        _ => "plaintext",
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// `~/x` and relative paths made absolute (relative ones against `base`), `.`/`..` folded.
pub fn resolve_path(text: &str, base: &Path) -> PathBuf {
    let text = text.trim();
    let expanded = match text.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(text)),
        None if text == "~" => std::env::var_os("HOME").map_or_else(|| text.into(), PathBuf::from),
        None => PathBuf::from(text),
    };
    let full = if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    };
    let mut out = PathBuf::new();
    for part in full.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn read_text(path: &Path) -> Result<String, String> {
    let meta =
        std::fs::metadata(path).map_err(|err| format!("{}: {}", path.display(), why(&err)))?;
    if meta.is_dir() {
        return Err(format!("{} is a directory.", path.display()));
    }
    if meta.len() > MAX_FILE {
        return Err(format!(
            "{} is too big for Pad ({} MB).",
            path.display(),
            meta.len() / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {}", path.display(), why(&err)))?;
    String::from_utf8(bytes).map_err(|_| format!("{} is not a UTF-8 text file.", path.display()))
}

fn why(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "no such file".into(),
        std::io::ErrorKind::PermissionDenied => "permission denied".into(),
        _ => err.to_string(),
    }
}

/// What `open` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub path: PathBuf,
    pub lines: usize,
    pub language: &'static str,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct Editor {
    docs: Vec<Doc>,
    current: Option<usize>,
}

impl Editor {
    pub fn docs(&self) -> &[Doc] {
        &self.docs
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current(&self) -> Option<&Doc> {
        self.current.and_then(|i| self.docs.get(i))
    }

    pub fn current_mut(&mut self) -> Option<&mut Doc> {
        self.current.and_then(|i| self.docs.get_mut(i))
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        self.docs.iter().position(|d| d.is(path))
    }

    pub fn switch(&mut self, index: usize) {
        if index < self.docs.len() {
            self.current = Some(index);
        }
    }

    pub fn cycle(&mut self, step: isize) {
        if self.docs.is_empty() {
            return;
        }
        let n = self.docs.len() as isize;
        let at = self.current.unwrap_or(0) as isize;
        self.current = Some((at + step).rem_euclid(n) as usize);
    }

    /// Opens the file (or switches to it when it is open already, picking up its changes on
    /// disk if it has no edits) and puts the cursor on `line` (1-based) if given.
    pub fn open(
        &mut self,
        path: &Path,
        line: Option<usize>,
    ) -> Result<(Opened, Vec<Effect>), String> {
        let mut effects = Vec::new();
        let index = match self.find(path) {
            Some(index) => {
                effects.extend(self.refresh(index));
                index
            }
            None => {
                let text = read_text(path)?;
                let buffer = Buffer::from_text(&text);
                let revision = buffer.revision();
                self.docs.push(Doc {
                    path: path.to_path_buf(),
                    canonical: canonical(path),
                    buffer,
                    saved_revision: revision,
                    changed_on_disk: false,
                    diagnostics: Vec::new(),
                    diag_state: DiagState::Idle,
                    highlight: None,
                    language: language_of(path),
                });
                effects.push(Effect::Diagnose(path.to_path_buf()));
                self.docs.len() - 1
            }
        };
        self.current = Some(index);
        let doc = &mut self.docs[index];
        if let Some(line) = line {
            doc.buffer
                .set_cursor(Pos::new(line.saturating_sub(1), 0), false);
            doc.highlight = None;
        }
        Ok((
            Opened {
                path: doc.path.clone(),
                lines: doc.buffer.line_count(),
                language: doc.language,
                line: doc.buffer.cursor().line + 1,
            },
            effects,
        ))
    }

    /// Closes a file. Refuses (returns false) when it has unsaved edits and `force` is off.
    pub fn close(&mut self, index: usize, force: bool) -> bool {
        let Some(doc) = self.docs.get(index) else {
            return true;
        };
        if doc.dirty() && !force {
            return false;
        }
        self.docs.remove(index);
        let n = self.docs.len();
        let at = self.current.unwrap_or(0);
        self.current = (n > 0).then(|| match at.cmp(&index) {
            std::cmp::Ordering::Greater => at - 1,
            std::cmp::Ordering::Equal => index.min(n - 1),
            std::cmp::Ordering::Less => at,
        });
        true
    }

    /// Writes the file and tells the bus. Saving over a file that changed on disk is the user's
    /// call to make, so it just does.
    pub fn save(&mut self, index: usize) -> Result<Vec<Effect>, String> {
        let doc = self.docs.get_mut(index).ok_or("No file is open.")?;
        std::fs::write(&doc.path, doc.buffer.file_text())
            .map_err(|err| format!("Cannot save {}: {}", doc.path.display(), why(&err)))?;
        doc.saved_revision = doc.buffer.revision();
        doc.changed_on_disk = false;
        Ok(vec![
            Effect::FileChanged(doc.path.clone()),
            Effect::Diagnose(doc.path.clone()),
        ])
    }

    /// Throws the edits away and reads the file again.
    pub fn reload(&mut self, index: usize) -> Result<Vec<Effect>, String> {
        let doc = self.docs.get_mut(index).ok_or("No file is open.")?;
        let text = read_text(&doc.path)?;
        doc.buffer.reload(&text);
        doc.saved_revision = doc.buffer.revision();
        doc.changed_on_disk = false;
        doc.highlight = None;
        Ok(vec![Effect::Diagnose(doc.path.clone())])
    }

    /// Re-reads a file with no edits if its text on disk differs; with edits it only notes that
    /// the disk has something else. A file that can't be read is left alone.
    fn refresh(&mut self, index: usize) -> Vec<Effect> {
        let Some(doc) = self.docs.get_mut(index) else {
            return Vec::new();
        };
        let Ok(text) = read_text(&doc.path) else {
            return Vec::new();
        };
        if doc.dirty() {
            doc.changed_on_disk = Buffer::from_text(&text).file_text() != doc.buffer.file_text();
            return Vec::new();
        }
        if text == doc.buffer.file_text() {
            return Vec::new();
        }
        doc.buffer.reload(&text);
        doc.saved_revision = doc.buffer.revision();
        doc.changed_on_disk = false;
        vec![Effect::Diagnose(doc.path.clone())]
    }

    /// `file.changed` from the bus (the agent edited something, or another app saved).
    pub fn on_file_changed(&mut self, path: &Path) -> Vec<Effect> {
        let Some(index) = self.find(path) else {
            return Vec::new();
        };
        self.refresh(index)
    }

    // --- diagnostics ---

    /// Marks the file as being checked; the revision to hand back to [`Editor::finish_diagnose`].
    pub fn begin_diagnose(&mut self, path: &Path) -> Option<u64> {
        let index = self.find(path)?;
        let doc = &mut self.docs[index];
        doc.diag_state = DiagState::Checking;
        Some(doc.buffer.revision())
    }

    /// Stores what the lsp service said. An answer for text that was edited since is kept only
    /// as "stale".
    pub fn finish_diagnose(
        &mut self,
        path: &Path,
        revision: u64,
        result: Result<Vec<Diagnostic>, String>,
    ) {
        let Some(index) = self.find(path) else {
            return;
        };
        let doc = &mut self.docs[index];
        if doc.diag_state != DiagState::Checking {
            return; // an answer to an older question
        }
        match result {
            Ok(list) => {
                doc.diagnostics = list;
                doc.diag_state = if doc.buffer.revision() == revision {
                    DiagState::Done
                } else {
                    DiagState::Stale
                };
            }
            Err(why) => {
                doc.diagnostics.clear();
                doc.diag_state = DiagState::Unavailable(why);
            }
        }
    }

    /// Edits make the diagnostics stale; call after anything that changed the text.
    pub fn note_edit(&mut self) {
        if let Some(doc) = self.current_mut()
            && matches!(doc.diag_state, DiagState::Done)
        {
            doc.diag_state = DiagState::Stale;
        }
    }

    // --- what the agent and the bus see ---

    /// The presence context: the current file and line.
    pub fn context(&self) -> Option<BTreeMap<String, String>> {
        let doc = self.current()?;
        let mut context = BTreeMap::new();
        context.insert("file".to_string(), doc.path.display().to_string());
        context.insert(
            "line".to_string(),
            (doc.buffer.cursor().line + 1).to_string(),
        );
        Some(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("suite-pad-editor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write");
        path
    }

    #[test]
    fn opening_reports_the_file_and_asks_for_diagnostics() {
        let dir = dir("open");
        let path = file(
            &dir,
            "Header.tsx",
            "export const a = 1;\nexport const b = 2;\n",
        );
        let mut editor = Editor::default();
        let (opened, effects) = editor.open(&path, Some(2)).expect("open");
        assert_eq!(
            opened,
            Opened {
                path: path.clone(),
                lines: 2,
                language: "typescriptreact",
                line: 2
            }
        );
        assert_eq!(effects, [Effect::Diagnose(path.clone())]);
        assert_eq!(
            editor.current().expect("current").buffer.cursor(),
            Pos::new(1, 0)
        );
        let context = editor.context().expect("context");
        assert_eq!(context["file"], path.display().to_string());
        assert_eq!(context["line"], "2");

        // Opening it again switches to it instead of adding a second copy.
        let (_, effects) = editor.open(&path, None).expect("reopen");
        assert!(effects.is_empty());
        assert_eq!(editor.docs().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opening_refuses_what_is_not_text() {
        let dir = dir("refuse");
        let mut editor = Editor::default();
        let missing = editor.open(&dir.join("nope.txt"), None).unwrap_err();
        assert!(missing.contains("no such file"), "{missing}");
        assert!(editor.open(&dir, None).unwrap_err().contains("directory"));
        let binary = dir.join("a.bin");
        std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x80]).expect("write");
        assert!(editor.open(&binary, None).unwrap_err().contains("UTF-8"));
        assert!(editor.docs().is_empty());
        assert_eq!(editor.context(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saving_writes_the_file_and_announces_it() {
        let dir = dir("save");
        let path = file(&dir, "a.txt", "one\ntwo\n");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        let doc = editor.current_mut().expect("doc");
        assert!(!doc.dirty());
        doc.buffer.move_doc_end(false);
        doc.buffer.insert("three");
        assert!(doc.dirty());
        assert_eq!(doc.state(), "modified");

        let effects = editor.save(0).expect("save");
        assert_eq!(
            effects,
            [
                Effect::FileChanged(path.clone()),
                Effect::Diagnose(path.clone())
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "one\ntwothree\n"
        );
        assert!(!editor.current().expect("doc").dirty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn undoing_back_to_the_saved_text_is_clean() {
        let dir = dir("undo");
        let path = file(&dir, "a.txt", "x");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        let doc = editor.current_mut().expect("doc");
        doc.buffer.insert("y");
        assert!(doc.dirty());
        doc.buffer.undo();
        assert!(!doc.dirty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_clean_file_reloads_when_it_changes_on_disk() {
        let dir = dir("reload");
        let path = file(&dir, "a.txt", "one\ntwo\nthree\n");
        let mut editor = Editor::default();
        editor.open(&path, Some(3)).expect("open");

        std::fs::write(&path, "one\n2\nthree\nfour\n").expect("agent edit");
        let effects = editor.on_file_changed(&path);
        assert_eq!(effects, [Effect::Diagnose(path.clone())]);
        let doc = editor.current().expect("doc");
        assert_eq!(doc.buffer.text(), "one\n2\nthree\nfour");
        assert_eq!(doc.buffer.cursor().line, 2, "the cursor stays");
        assert!(!doc.dirty() && !doc.changed_on_disk);

        // Same content: nothing to do. Other files: ignored.
        assert!(editor.on_file_changed(&path).is_empty());
        assert!(editor.on_file_changed(&dir.join("other.txt")).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_modified_file_is_flagged_instead_of_reloaded() {
        let dir = dir("conflict");
        let path = file(&dir, "a.txt", "one\n");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        editor.current_mut().expect("doc").buffer.insert("my ");

        std::fs::write(&path, "agent\n").expect("agent edit");
        assert!(editor.on_file_changed(&path).is_empty());
        let doc = editor.current().expect("doc");
        assert_eq!(doc.buffer.text(), "my one", "my edits are untouched");
        assert!(doc.dirty() && doc.changed_on_disk);
        assert_eq!(doc.state(), "modified, changed on disk");

        // Saving over it is allowed and clears the flag; so does reloading.
        editor.save(0).expect("save");
        assert!(!editor.current().expect("doc").changed_on_disk);
        std::fs::write(&path, "agent again\n").expect("agent edit");
        editor.current_mut().expect("doc").buffer.insert("x");
        editor.on_file_changed(&path);
        assert!(editor.current().expect("doc").changed_on_disk);
        let effects = editor.reload(0).expect("reload");
        assert_eq!(effects, [Effect::Diagnose(path.clone())]);
        let doc = editor.current().expect("doc");
        assert_eq!(doc.buffer.text(), "agent again");
        assert!(!doc.dirty() && !doc.changed_on_disk);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_change_that_equals_my_edits_is_no_conflict() {
        let dir = dir("same");
        let path = file(&dir, "a.txt", "a\n");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        editor.current_mut().expect("doc").buffer.insert("b");
        std::fs::write(&path, "ba\n").expect("write");
        editor.on_file_changed(&path);
        assert!(!editor.current().expect("doc").changed_on_disk);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_symlinked_path_finds_the_open_file() {
        let dir = dir("link");
        let path = file(&dir, "real.txt", "a\n");
        let link = dir.join("link.txt");
        std::os::unix::fs::symlink(&path, &link).expect("symlink");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        assert_eq!(editor.find(&link), Some(0));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn closing_keeps_unsaved_work_unless_forced() {
        let dir = dir("close");
        let a = file(&dir, "a.txt", "a");
        let b = file(&dir, "b.txt", "b");
        let mut editor = Editor::default();
        editor.open(&a, None).expect("open");
        editor.open(&b, None).expect("open");
        assert_eq!(editor.current_index(), Some(1));
        editor.current_mut().expect("doc").buffer.insert("x");
        assert!(!editor.close(1, false));
        assert!(editor.close(1, true));
        assert_eq!(editor.current_index(), Some(0));
        assert!(editor.close(0, false));
        assert_eq!(editor.current_index(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    fn diag(line: usize, column: usize, severity: Severity) -> Diagnostic {
        Diagnostic {
            line,
            column,
            severity,
            message: "m".into(),
            source: "ts".into(),
        }
    }

    #[test]
    fn diagnostics_arrive_for_the_text_they_were_asked_about() {
        let dir = dir("diag");
        let path = file(&dir, "a.ts", "let foo = bar;\nlet z;\n");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        let revision = editor.begin_diagnose(&path).expect("begin");
        assert_eq!(
            editor.current().expect("doc").diag_state,
            DiagState::Checking
        );
        editor.finish_diagnose(&path, revision, Ok(vec![diag(1, 11, Severity::Error)]));
        let doc = editor.current().expect("doc");
        assert_eq!(doc.diag_state, DiagState::Done);
        assert_eq!(doc.worst_on(0), Some(Severity::Error));
        assert_eq!(doc.worst_on(1), None);

        // `bar` is underlined (3 characters), the caret is a mark of its own.
        let marks = doc.marks(0);
        assert!(marks.contains(&(10..13, Mark::Error)), "{marks:?}");
        assert!(marks.contains(&(0..1, Mark::Cursor)), "{marks:?}");

        // Edited since: kept, but stale.
        editor.current_mut().expect("doc").buffer.insert("x");
        editor.note_edit();
        assert_eq!(editor.current().expect("doc").diag_state, DiagState::Stale);

        // A late answer for old text is stale; an answer nobody asked for is dropped.
        let revision = editor.begin_diagnose(&path).expect("begin");
        editor.current_mut().expect("doc").buffer.insert("y");
        editor.finish_diagnose(&path, revision, Ok(vec![diag(2, 1, Severity::Warning)]));
        assert_eq!(editor.current().expect("doc").diag_state, DiagState::Stale);
        editor.finish_diagnose(&path, revision, Ok(Vec::new()));
        assert_eq!(editor.current().expect("doc").diagnostics.len(), 1);

        // Failure clears the list and says why.
        let revision = editor.begin_diagnose(&path).expect("begin");
        editor.finish_diagnose(&path, revision, Err("lsp isn't running".into()));
        let doc = editor.current().expect("doc");
        assert!(doc.diagnostics.is_empty());
        assert_eq!(
            doc.diag_state,
            DiagState::Unavailable("lsp isn't running".into())
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn marks_cover_selections_across_lines() {
        let dir = dir("marks");
        let path = file(&dir, "a.txt", "abc\ndef\nghi\n");
        let mut editor = Editor::default();
        editor.open(&path, None).expect("open");
        let doc = editor.current_mut().expect("doc");
        doc.buffer.set_cursor(Pos::new(0, 1), false);
        doc.buffer.set_cursor(Pos::new(2, 1), true);
        assert!(doc.marks(0).contains(&(1..4, Mark::Selected)));
        assert!(doc.marks(1).contains(&(0..4, Mark::Selected)));
        assert!(doc.marks(2).contains(&(0..1, Mark::Selected)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn segments_cut_overlaps() {
        let marks = vec![
            (0..4, Mark::Selected),
            (2..6, Mark::Error),
            (5..6, Mark::Cursor),
        ];
        let got = segments(8, &marks);
        assert_eq!(
            got,
            [
                (0..2, vec![Mark::Selected]),
                (2..4, vec![Mark::Selected, Mark::Error]),
                (4..5, vec![Mark::Error]),
                (5..6, vec![Mark::Error, Mark::Cursor]),
            ]
        );
        assert!(segments(0, &[]).is_empty());
    }

    #[test]
    fn paths_resolve_against_a_base() {
        let base = Path::new("/home/me/site");
        assert_eq!(
            resolve_path("src/../a.tsx", base),
            PathBuf::from("/home/me/site/a.tsx")
        );
        assert_eq!(resolve_path("/etc/./x", base), PathBuf::from("/etc/x"));
        assert_eq!(
            resolve_path("  ./b  ", base),
            PathBuf::from("/home/me/site/b")
        );
    }

    #[test]
    fn languages_come_from_the_extension() {
        assert_eq!(language_of(Path::new("/a/Header.tsx")), "typescriptreact");
        assert_eq!(language_of(Path::new("/a/main.go")), "go");
        assert_eq!(language_of(Path::new("/a/Makefile")), "makefile");
        assert_eq!(language_of(Path::new("/a/noext")), "plaintext");
    }
}
