//! Reviewing a pull request from its detail: Enter on the files or the checks picks one (j/k),
//! Enter again opens it.
//!
//! - **A file**: its part of the diff in the diff view, the review comments on their lines;
//!   `[` / `]` go to the previous or next file, `f` fetches the file at the head commit and
//!   shows all of it around the changes (and back).
//! - **A check**: a GitHub Actions job's log in a reader, only the failed steps first, `f` for
//!   all of it; a check from elsewhere has no log here, Enter copies its link.
//!
//! `d` opens the files from the first one (or the picked one). `y` copies in both views.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::{Detail, Kind};
use super::{Github, GhError, Log, Mode, Msg, Outgoing, Part, pr};
use crate::reader::{Reader, ReaderAction};
use crate::widgets::DiffView;
use neo_github::fetch;

const LOG_HINT: &str = "/ search · f all/failed · v V select · y copy · q back";

impl Github {
    /// The pull request the detail shows, loaded.
    fn pull(&self) -> Option<&Detail> {
        let Mode::Detail(v) = &self.mode else {
            return None;
        };
        if v.kind != Kind::Pr {
            return None;
        }
        self.details.get(&(v.kind, v.number))?.detail.as_ref()
    }

    /// A key while a row of the checks or the files is picked; None when it is not one of
    /// theirs (it goes on to the detail's).
    pub(super) fn row_key(&mut self, key: KeyEvent) -> Option<Option<Outgoing>> {
        let Mode::Detail(v) = &mut self.mode else {
            return None;
        };
        let row = v.row?;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => v.pick_row(row.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => v.pick_row(row + 1),
            KeyCode::Home | KeyCode::Char('g') => v.pick_row(0),
            KeyCode::End | KeyCode::Char('G') => v.pick_row(usize::MAX),
            KeyCode::Esc => v.row = None,
            KeyCode::Enter | KeyCode::Char('d') if v.focus == Part::Files => {
                self.open_file(row);
            }
            KeyCode::Enter if v.focus == Part::Checks => return Some(self.open_check(row)),
            _ => return None,
        }
        Some(None)
    }

    // ---- files ----

    /// Shows file `i` of the pull request (the last when past the end), fetching the diff
    /// first if it is not here yet.
    pub(super) fn open_file(&mut self, i: usize) {
        let Some(d) = self.pull() else {
            return;
        };
        if d.files.is_empty() {
            self.note = Some("no files changed".into());
            return;
        }
        let i = i.min(d.files.len() - 1);
        let number = d.number;
        let view = match self.diffs.get(&number) {
            Some(text) => self.file_view(i, text),
            None => {
                self.load_diff(number);
                DiffView::loading(&self.file_title(i))
            }
        };
        if let Mode::Detail(v) = &mut self.mode {
            v.diff = Some(view);
            v.file = Some(i);
        }
    }

    fn file_title(&self, i: usize) -> String {
        let Some(d) = self.pull() else {
            return String::new();
        };
        let path = d.files.get(i).map_or("", |f| f.path.as_str());
        format!("#{} {path}  {}/{}  [ ] files", d.number, i + 1, d.files.len())
    }

    /// File `i`'s part of `diff`, with its review comments.
    fn file_view(&self, i: usize, diff: &str) -> DiffView {
        let title = self.file_title(i);
        let Some(d) = self.pull() else {
            return DiffView::loading(&title);
        };
        let path = &d.files[i].path;
        let Some((_, section)) = pr::split_diff(diff).into_iter().find(|(p, _)| p == path) else {
            let mut view = DiffView::loading(&title);
            view.finish(Err("no diff for this file: binary, or too big for GitHub to show".into()));
            return view;
        };
        let mut view = DiffView::from_file_text(&title, path, &section)
            .with_remote_full(&section)
            .with_sides(&d.base, &d.head);
        view.set_remarks(pr::remarks(&d.review_comments, path, self.now_ms));
        view
    }

    /// The pull request's diff came: the file waiting for it is shown.
    pub(super) fn diff_arrived(&mut self, number: u64, result: Result<String, GhError>) {
        let result = result.map_err(|e| e.message());
        if let Ok(text) = &result {
            self.diffs.insert(number, text.clone());
        }
        let Mode::Detail(v) = &self.mode else {
            return;
        };
        if v.number != number || v.diff.is_none() {
            return;
        }
        let file = v.file;
        let view = match (&result, file) {
            (Ok(text), Some(i)) => Some(self.file_view(i, text)),
            _ => None,
        };
        if let Mode::Detail(v) = &mut self.mode
            && let Some(diff) = &mut v.diff
        {
            match view {
                Some(view) => *diff = view,
                None => diff.finish(result),
            }
        }
    }

    /// A key while a file's diff is shown: `[` and `]` go between the files, the diff view has
    /// the rest.
    pub(super) fn diff_key(&mut self, key: KeyEvent) -> Option<Outgoing> {
        let Mode::Detail(v) = &mut self.mode else {
            return None;
        };
        let file = v.file;
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        match (key.code, file) {
            (KeyCode::Char(']'), Some(i)) if plain => {
                self.open_file(i + 1);
                return None;
            }
            (KeyCode::Char('['), Some(i)) if plain => {
                self.open_file(i.saturating_sub(1));
                return None;
            }
            _ => {}
        }
        let diff = v.diff.as_mut()?;
        if diff.key(key) {
            v.diff = None;
            v.file = None;
            return None;
        }
        let asked = diff.take_full_ask();
        let copy = diff.take_copy();
        if asked {
            self.fetch_whole();
        }
        copy.map(Outgoing::Copy)
    }

    /// `f` in a file's diff: the whole file, from the cache, from the diff itself (a file
    /// added or deleted is all there), or from GitHub at the head commit.
    fn fetch_whole(&mut self) {
        let Mode::Detail(v) = &self.mode else {
            return;
        };
        let (Some(i), number) = (v.file, v.number) else {
            return;
        };
        let Some(d) = self.pull() else {
            return;
        };
        let path = d.files[i].path.clone();
        let sha = d.head_sha.clone();
        let section = self
            .diffs
            .get(&number)
            .and_then(|t| pr::split_diff(t).into_iter().find(|(p, _)| *p == path))
            .map(|(_, s)| s);
        let answer = match &section {
            None => Some(Err("no diff for this file".to_string())),
            Some(s) if pr::is_whole(s) => Some(Ok(s.clone())),
            Some(s) => match self.whole_files.get(&(sha.clone(), path.clone())) {
                Some(file) => Some(Ok(pr::expand(s, file))),
                None if sha.is_empty() => Some(Err("the head commit is not known".to_string())),
                None => None,
            },
        };
        if let Some(answer) = answer {
            self.give_whole(answer);
            return;
        }
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.spawn(move |gh, cwd| {
            let result = fetch::file_at(gh, cwd, &repo, &path, &sha);
            Msg::File(number, format!("{sha}\n{path}"), result)
        });
    }

    fn give_whole(&mut self, answer: Result<String, String>) {
        if let Mode::Detail(v) = &mut self.mode
            && let Some(diff) = &mut v.diff
        {
            diff.give_full(answer);
        }
    }

    /// A file at a head commit came (`key` is `sha\npath`): kept, and shown whole when its
    /// diff is still open.
    pub(super) fn file_arrived(&mut self, number: u64, key: &str, result: Result<String, GhError>) {
        let Some((sha, path)) = key.split_once('\n') else {
            return;
        };
        if let Ok(text) = &result {
            self.whole_files.insert((sha.to_string(), path.to_string()), text.clone());
        }
        let Mode::Detail(v) = &self.mode else {
            return;
        };
        let shown = v.number == number
            && v.file.is_some_and(|i| self.pull().is_some_and(|d| d.files[i].path == path));
        if !shown {
            return;
        }
        let section = self
            .diffs
            .get(&number)
            .and_then(|t| pr::split_diff(t).into_iter().find(|(p, _)| p == path))
            .map(|(_, s)| s)
            .unwrap_or_default();
        self.give_whole(
            result
                .map(|file| pr::expand(&section, &file))
                .map_err(|e| format!("could not fetch the file: {}", e.message())),
        );
    }

    // ---- checks ----

    /// Check `row` (of the detail's order) opened: its log, or its link copied when it has none
    /// here.
    fn open_check(&mut self, row: usize) -> Option<Outgoing> {
        let d = self.pull()?;
        let i = *pr::check_order(&d.checks).get(row)?;
        let check = d.checks[i].clone();
        if check.job == 0 {
            if check.url.is_empty() {
                self.note = Some(format!("{} has no log or link", check.name));
                return None;
            }
            self.info = Some(format!("{} has no log here: its link is copied", check.name));
            return Some(Outgoing::Copy(check.url));
        }
        self.show_log(i, true);
        None
    }

    /// The log of check `i`, from the cache or fetched.
    fn show_log(&mut self, i: usize, failed: bool) {
        let Some(d) = self.pull() else {
            return;
        };
        let check = &d.checks[i];
        let (job, name) = (check.job, check.name.clone());
        let title = log_title(&name, failed);
        let reader = match self.logs.get(&(job, failed)) {
            Some(text) => log_reader(&title, text, failed),
            None => {
                if let Some(repo) = self.repo_name().map(str::to_string) {
                    self.spawn(move |gh, cwd| {
                        Msg::Log(job, failed, fetch::job_log(gh, cwd, &repo, job, failed))
                    });
                }
                Reader::plain(&title, "loading…", LOG_HINT)
            }
        };
        if let Mode::Detail(v) = &mut self.mode {
            v.log = Some(Log {
                check: i,
                failed,
                reader,
            });
        }
    }

    /// A job's log came: kept, and shown when it is the one open.
    pub(super) fn log_arrived(&mut self, job: u64, failed: bool, result: Result<String, GhError>) {
        let text = match result {
            Ok(raw) => {
                let text = pr::tidy_log(&raw);
                self.logs.insert((job, failed), text.clone());
                text
            }
            Err(e) => format!("could not read the log: {}", e.message()),
        };
        let name = {
            let Mode::Detail(v) = &self.mode else {
                return;
            };
            let Some(log) = &v.log else {
                return;
            };
            match self.pull().and_then(|d| d.checks.get(log.check)) {
                Some(c) if c.job == job && log.failed == failed => c.name.clone(),
                _ => return,
            }
        };
        if let Mode::Detail(v) = &mut self.mode
            && let Some(log) = &mut v.log
        {
            log.reader = log_reader(&log_title(&name, failed), &text, failed);
        }
    }

    /// A key while a log is shown: `f` switches between the failed steps and all of it, the
    /// reader has the rest; leaving it is back to the detail.
    pub(super) fn log_key(&mut self, key: KeyEvent) -> Option<Outgoing> {
        let Mode::Detail(v) = &mut self.mode else {
            return None;
        };
        let log = v.log.as_mut()?;
        // `f` switches, unless it is typed into a search.
        if key.code == KeyCode::Char('f') && key.modifiers.is_empty() && !log.reader.is_typing() {
            let (check, failed) = (log.check, log.failed);
            self.show_log(check, !failed);
            return None;
        }
        if let ReaderAction::Close = log.reader.key(key) {
            v.log = None;
            return None;
        }
        log.reader.take_copy().map(Outgoing::Copy)
    }
}

fn log_title(name: &str, failed: bool) -> String {
    format!("{name} · {}", if failed { "failed steps" } else { "all steps" })
}

fn log_reader(title: &str, text: &str, failed: bool) -> Reader {
    let text = if text.trim().is_empty() {
        if failed {
            "no failed steps in this log: f shows all of it"
        } else {
            "the log is empty"
        }
    } else {
        text
    };
    let mut reader = Reader::plain(title, text, LOG_HINT);
    // Where it went wrong is usually at the end.
    reader.to_end();
    reader
}
