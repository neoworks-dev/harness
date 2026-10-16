//! The GitHub view: a full-screen overlay (`/github`, Ctrl-R) to read a repository's issues and
//! pull requests with their comments, and to hand one to the model.
//!
//! Everything comes from the `gh` CLI, read-only, on background threads: the UI thread only
//! starts jobs ([`Github::key`]) and picks up their answers ([`Github::tick`]), so it never
//! blocks. The repository is the one `gh` finds for the session's directory. Lists (per tab,
//! state filter and search) and details are cached until the directory changes; `r` reloads.
//!
//! - **List**: tabs Issues / Pull requests (Tab, 1, 2), Up/Down/j/k, PgUp/PgDn, g/G, Enter opens,
//!   `/` searches (GitHub search syntax), `o` toggles open / all, `r` reloads, Esc or q close.
//! - **Detail**: scrolls like the diff overlay; Esc goes back, q closes. `d` shows a pull
//!   request's diff in the diff overlay's view.
//! - **To the model**: `a` (Enter in a detail) puts the item as a `<github-issue>` /
//!   `<github-pr>` block into the prompt and closes the view; `s` sends it right away with a
//!   short instruction. Both are [`Outgoing`] values for the shell to carry out.

mod doc;
mod format;
pub mod gh;
mod md;
pub mod model;
mod view;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::Line;
use suite_renderer_ratatui::Palette;

use crate::widgets::DiffView;
use gh::{Gh, GhCli, GhError};
use model::{Detail, Kind, Summary};

/// Rows one wheel notch moves.
const WHEEL_ROWS: usize = 3;

/// What the shell does with a key or a finished job.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    /// Put this text into the prompt editor.
    Attach(String),
    /// Send this text as a user message now.
    Send(String),
}

/// The result of [`Github::tick`].
#[derive(Debug, Default)]
pub struct Tick {
    /// Something on screen changed.
    pub changed: bool,
    /// A fetch the user waited for (attach / send from the list) finished.
    pub out: Option<Outgoing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ListKey {
    kind: Kind,
    all: bool,
    query: String,
}

enum Repo {
    Unknown,
    Loading,
    Ready(String),
    Failed(GhError),
}

#[derive(Default)]
struct ListData {
    rows: Vec<Summary>,
    loading: bool,
    error: Option<GhError>,
    selected: usize,
    top: usize,
}

#[derive(Default)]
struct DetailData {
    detail: Option<Detail>,
    loading: bool,
    error: Option<GhError>,
    /// Bumped when `detail` changes, so the shown lines are rebuilt.
    version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Attach,
    Send,
}

/// The lines of the detail at the width they were built for.
struct Doc {
    width: usize,
    version: u64,
    minute: i64,
    lines: Vec<Line<'static>>,
}

struct DetailView {
    kind: Kind,
    number: u64,
    scroll: usize,
    /// Body rows of the last draw.
    rows: usize,
    doc: Option<Doc>,
    diff: Option<DiffView>,
}

/// One value lives in the state, so the size difference costs nothing.
#[allow(clippy::large_enum_variant)]
enum Mode {
    List,
    Detail(DetailView),
}

/// Answers from the job threads.
enum Msg {
    Repo(Result<String, GhError>),
    List(ListKey, Result<Vec<Summary>, GhError>),
    Detail(Kind, u64, Result<Box<Detail>, GhError>),
    Diff(u64, Result<String, GhError>),
}

pub struct Github {
    gh: Arc<dyn Gh>,
    cwd: PathBuf,
    palette: Palette,
    open: bool,
    repo: Repo,
    tab: Kind,
    all: bool,
    query: String,
    /// The search being typed (`/`).
    typing: Option<String>,
    lists: HashMap<ListKey, ListData>,
    details: HashMap<(Kind, u64), DetailData>,
    diffs: HashMap<u64, String>,
    mode: Mode,
    /// A fetch an attach / send waits for.
    pending: Option<(Kind, u64, Intent)>,
    /// An error to show in the footer until the next key.
    note: Option<String>,
    /// Body rows of the last list draw.
    list_rows: usize,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Github {
    /// For a session working in `cwd`; nothing runs until the view is opened.
    pub fn new(cwd: PathBuf, palette: Palette) -> Github {
        Github::with_runner(cwd, palette, Arc::new(GhCli))
    }

    /// Like [`Github::new`] with another way to run `gh` (tests).
    pub fn with_runner(cwd: PathBuf, palette: Palette, gh: Arc<dyn Gh>) -> Github {
        let (tx, rx) = mpsc::channel();
        Github {
            gh,
            cwd,
            palette,
            open: false,
            repo: Repo::Unknown,
            tab: Kind::Issue,
            all: false,
            query: String::new(),
            typing: None,
            lists: HashMap::new(),
            details: HashMap::new(),
            diffs: HashMap::new(),
            mode: Mode::List,
            pending: None,
            note: None,
            list_rows: 10,
            tx,
            rx,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Another session (or directory) is shown: closed, and the cache dropped if the directory
    /// is not the same. Jobs still running find their channel gone and end quietly.
    pub fn reset(&mut self, cwd: PathBuf) {
        if cwd != self.cwd {
            *self = Github::with_runner(cwd, self.palette, self.gh.clone());
        }
        self.open = false;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Shows the view (where it was left) and starts loading what is missing.
    pub fn open(&mut self) {
        self.open = true;
        self.note = None;
        self.ensure_list();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.typing = None;
    }

    // ---- jobs ----

    fn spawn(&self, job: impl FnOnce(&dyn Gh, &std::path::Path) -> Msg + Send + 'static) {
        let (gh, cwd, tx) = (self.gh.clone(), self.cwd.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(job(&*gh, &cwd));
        });
    }

    fn repo_name(&self) -> Option<&str> {
        match &self.repo {
            Repo::Ready(name) => Some(name),
            _ => None,
        }
    }

    fn current(&self) -> ListKey {
        ListKey {
            kind: self.tab,
            all: self.all,
            query: self.query.clone(),
        }
    }

    /// Starts what the shown list needs: the repository first, then the list itself.
    fn ensure_list(&mut self) {
        match &self.repo {
            Repo::Unknown => {
                self.repo = Repo::Loading;
                self.spawn(|gh, cwd| {
                    Msg::Repo(
                        gh.run(cwd, &gh::repo_args())
                            .and_then(|out| model::parse_repo(&out).map_err(GhError::Other)),
                    )
                });
            }
            Repo::Ready(_) => {
                let key = self.current();
                if !self.lists.contains_key(&key) {
                    self.load_list(key);
                }
            }
            Repo::Loading | Repo::Failed(_) => {}
        }
    }

    fn load_list(&mut self, key: ListKey) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        let data = self.lists.entry(key.clone()).or_default();
        if data.loading {
            return;
        }
        data.loading = true;
        data.error = None;
        self.spawn(move |gh, cwd| {
            let args = gh::list_args(key.kind, &repo, key.all, &key.query);
            let result = gh
                .run(cwd, &args)
                .and_then(|out| model::parse_list(key.kind, &out).map_err(GhError::Other));
            Msg::List(key, result)
        });
    }

    fn load_detail(&mut self, kind: Kind, number: u64) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        let data = self.details.entry((kind, number)).or_default();
        if data.loading {
            return;
        }
        data.loading = true;
        data.error = None;
        self.spawn(move |gh, cwd| {
            let result = gh
                .run(cwd, &gh::view_args(kind, &repo, number))
                .and_then(|out| model::parse_detail(kind, &out).map_err(GhError::Other))
                .map(|detail| {
                    let mut detail = Box::new(detail);
                    if kind == Kind::Pr {
                        // Best effort: the pull request is useful without them.
                        if let Ok(out) = gh.run(cwd, &gh::review_comments_args(&repo, number)) {
                            detail.review_comments = model::parse_review_comments(&out);
                        }
                    }
                    detail
                });
            Msg::Detail(kind, number, result)
        });
    }

    fn load_diff(&mut self, number: u64) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.spawn(move |gh, cwd| Msg::Diff(number, gh.run(cwd, &gh::diff_args(&repo, number))));
    }

    /// Picks up finished jobs. Called on every loop iteration.
    pub fn tick(&mut self) -> Tick {
        let mut tick = Tick::default();
        while let Ok(msg) = self.rx.try_recv() {
            tick.changed = true;
            match msg {
                Msg::Repo(Ok(name)) => {
                    self.repo = Repo::Ready(name);
                    self.ensure_list();
                }
                Msg::Repo(Err(e)) => self.repo = Repo::Failed(e),
                Msg::List(key, result) => {
                    let data = self.lists.entry(key).or_default();
                    data.loading = false;
                    match result {
                        Ok(rows) => {
                            data.error = None;
                            data.selected = data.selected.min(rows.len().saturating_sub(1));
                            data.rows = rows;
                        }
                        Err(e) => {
                            data.error = Some(e);
                            data.rows.clear();
                        }
                    }
                }
                Msg::Detail(kind, number, result) => {
                    let data = self.details.entry((kind, number)).or_default();
                    data.loading = false;
                    let waiting = self.pending.filter(|p| (p.0, p.1) == (kind, number));
                    match result {
                        Ok(detail) => {
                            data.error = None;
                            data.detail = Some(*detail);
                            data.version += 1;
                            if let Some((_, _, intent)) = waiting {
                                self.pending = None;
                                tick.out = self.outgoing(kind, number, intent);
                            }
                        }
                        Err(e) => {
                            if waiting.is_some() {
                                self.pending = None;
                                self.note =
                                    Some(format!("could not load #{number}: {}", e.message()));
                            } else if data.detail.is_some() {
                                self.note =
                                    Some(format!("could not reload #{number}: {}", e.message()));
                            }
                            data.error = Some(e);
                        }
                    }
                }
                Msg::Diff(number, result) => {
                    let result = result.map_err(|e| e.message());
                    if let Ok(text) = &result {
                        self.diffs.insert(number, text.clone());
                    }
                    if let Mode::Detail(v) = &mut self.mode
                        && v.number == number
                        && let Some(diff) = &mut v.diff
                    {
                        diff.finish(result);
                    }
                }
            }
        }
        if let Mode::Detail(DetailView { diff: Some(d), .. }) = &mut self.mode {
            tick.changed |= d.poll();
        }
        tick
    }

    // ---- to the model ----

    /// The text for this item if it is loaded, else None.
    fn outgoing(&mut self, kind: Kind, number: u64, intent: Intent) -> Option<Outgoing> {
        let repo = self.repo_name()?.to_string();
        let detail = self.details.get(&(kind, number))?.detail.as_ref()?;
        let out = match intent {
            Intent::Attach => Outgoing::Attach(format::block(&repo, detail)),
            Intent::Send => Outgoing::Send(format::send_text(&repo, detail)),
        };
        self.close();
        Some(out)
    }

    /// Attach or send an item: now when its detail is loaded and not older than `seen` (the
    /// list's `updatedAt`), else after fetching it.
    fn request(&mut self, kind: Kind, number: u64, seen: i64, intent: Intent) -> Option<Outgoing> {
        let fresh = self
            .details
            .get(&(kind, number))
            .and_then(|d| d.detail.as_ref())
            .is_some_and(|d| d.updated_ms >= seen);
        if fresh {
            return self.outgoing(kind, number, intent);
        }
        self.pending = Some((kind, number, intent));
        self.load_detail(kind, number);
        None
    }

    // ---- keys ----

    /// A key press while the view is open.
    pub fn key(&mut self, key: KeyEvent) -> Option<Outgoing> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.note = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let diff_open = matches!(&self.mode, Mode::Detail(v) if v.diff.is_some());
        if self.typing.is_some() {
            self.typing_key(key);
            return None;
        }
        if ctrl && matches!(key.code, KeyCode::Char('c' | 'r')) && !diff_open {
            self.close();
            return None;
        }
        match &mut self.mode {
            Mode::List => self.list_key(key),
            Mode::Detail(_) => self.detail_key(key),
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(t) = &mut self.typing {
            t.extend(text.chars().filter(|c| !c.is_control()));
        }
    }

    /// The mouse wheel: moves the selection, scrolls the detail or the diff.
    pub fn wheel(&mut self, down: bool) {
        let delta = if down { 1 } else { -1 };
        match &mut self.mode {
            Mode::List => self.move_selection(delta * WHEEL_ROWS as isize),
            Mode::Detail(v) => match &mut v.diff {
                Some(diff) => {
                    let code = if down { KeyCode::Down } else { KeyCode::Up };
                    for _ in 0..WHEEL_ROWS {
                        diff.key(KeyEvent::from(code));
                    }
                }
                None => v.scroll_by(delta * WHEEL_ROWS as isize),
            },
        }
    }

    fn typing_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(text) = &mut self.typing else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.typing = None,
            KeyCode::Char('c') if ctrl => self.typing = None,
            KeyCode::Char('u') if ctrl => text.clear(),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Enter => {
                self.query = text.trim().to_string();
                self.typing = None;
                self.ensure_list();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                text.push(c);
            }
            _ => {}
        }
    }

    fn set_tab(&mut self, tab: Kind) {
        self.tab = tab;
        self.ensure_list();
    }

    fn list_key(&mut self, key: KeyEvent) -> Option<Outgoing> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.list_rows.saturating_sub(1).max(1) as isize;
        match (key.code, ctrl) {
            (KeyCode::Esc | KeyCode::Char('q'), _) => self.close(),
            (KeyCode::Tab | KeyCode::BackTab, _) => self.set_tab(self.tab.other()),
            (KeyCode::Char('1'), false) => self.set_tab(Kind::Issue),
            (KeyCode::Char('2'), false) => self.set_tab(Kind::Pr),
            (KeyCode::Up | KeyCode::Char('k'), _) => self.move_selection(-1),
            (KeyCode::Down | KeyCode::Char('j'), _) => self.move_selection(1),
            (KeyCode::PageUp | KeyCode::Char('b'), _) => self.move_selection(-page),
            (KeyCode::PageDown | KeyCode::Char(' '), _) => self.move_selection(page),
            (KeyCode::Char('u'), true) => self.move_selection(-(page / 2).max(1)),
            (KeyCode::Char('d'), true) => self.move_selection((page / 2).max(1)),
            (KeyCode::Home | KeyCode::Char('g'), _) => self.move_selection(isize::MIN / 2),
            (KeyCode::End | KeyCode::Char('G'), _) => self.move_selection(isize::MAX / 2),
            (KeyCode::Char('/'), _) => self.typing = Some(self.query.clone()),
            (KeyCode::Char('o'), false) => {
                self.all = !self.all;
                self.ensure_list();
            }
            (KeyCode::Char('r'), false) => self.reload_list(),
            (KeyCode::Enter, _) => self.open_selected(),
            (KeyCode::Char('a'), false) => return self.send_selected(Intent::Attach),
            (KeyCode::Char('s'), false) => return self.send_selected(Intent::Send),
            _ => {}
        }
        None
    }

    fn reload_list(&mut self) {
        if matches!(self.repo, Repo::Failed(_)) {
            self.repo = Repo::Unknown;
        }
        let key = self.current();
        if matches!(self.repo, Repo::Ready(_)) {
            self.load_list(key);
        } else {
            self.ensure_list();
        }
    }

    fn selected(&self) -> Option<&Summary> {
        let data = self.lists.get(&self.current())?;
        data.rows.get(data.selected)
    }

    fn move_selection(&mut self, delta: isize) {
        let key = self.current();
        if let Some(data) = self.lists.get_mut(&key) {
            let last = data.rows.len().saturating_sub(1);
            data.selected = data.selected.saturating_add_signed(delta).min(last);
        }
    }

    fn open_selected(&mut self) {
        let Some((number, seen)) = self.selected().map(|s| (s.number, s.updated_ms)) else {
            return;
        };
        let kind = self.tab;
        let stale = self
            .details
            .get(&(kind, number))
            .and_then(|d| d.detail.as_ref())
            .is_none_or(|d| d.updated_ms < seen);
        if stale {
            self.load_detail(kind, number);
        }
        self.mode = Mode::Detail(DetailView::new(kind, number));
    }

    fn send_selected(&mut self, intent: Intent) -> Option<Outgoing> {
        let (number, seen) = self.selected().map(|s| (s.number, s.updated_ms))?;
        self.request(self.tab, number, seen, intent)
    }

    fn detail_key(&mut self, key: KeyEvent) -> Option<Outgoing> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Mode::Detail(v) = &mut self.mode else {
            return None;
        };
        if let Some(diff) = &mut v.diff {
            if diff.key(key) {
                v.diff = None;
            }
            return None;
        }
        let (kind, number) = (v.kind, v.number);
        let page = v.rows.saturating_sub(1).max(1) as isize;
        match (key.code, ctrl) {
            (KeyCode::Esc, _) => self.mode = Mode::List,
            (KeyCode::Char('q'), false) => self.close(),
            (KeyCode::Up | KeyCode::Char('k'), _) => v.scroll_by(-1),
            (KeyCode::Down | KeyCode::Char('j'), _) => v.scroll_by(1),
            (KeyCode::PageUp | KeyCode::Char('b'), _) => v.scroll_by(-page),
            (KeyCode::PageDown | KeyCode::Char(' '), _) => v.scroll_by(page),
            (KeyCode::Char('u'), true) => v.scroll_by(-(page / 2).max(1)),
            (KeyCode::Char('d'), true) => v.scroll_by((page / 2).max(1)),
            (KeyCode::Home | KeyCode::Char('g'), _) => v.scroll_by(isize::MIN / 2),
            (KeyCode::End | KeyCode::Char('G'), _) => v.scroll_by(isize::MAX / 2),
            (KeyCode::Char('r'), false) => self.load_detail(kind, number),
            (KeyCode::Enter | KeyCode::Char('a'), _) => {
                return self.request(kind, number, 0, Intent::Attach);
            }
            (KeyCode::Char('s'), false) => return self.request(kind, number, 0, Intent::Send),
            (KeyCode::Char('d'), false) if kind == Kind::Pr => self.open_diff(number),
            _ => {}
        }
        None
    }

    fn open_diff(&mut self, number: u64) {
        let title = format!("#{number} {}", self.repo_name().unwrap_or(""));
        let cached = self.diffs.get(&number).cloned();
        let diff = match &cached {
            Some(text) => DiffView::from_text(&title, text),
            None => DiffView::loading(&title),
        };
        if cached.is_none() {
            self.load_diff(number);
        }
        if let Mode::Detail(v) = &mut self.mode {
            v.diff = Some(diff);
        }
    }

    // ---- drawing ----

    /// Fills `area` (the chat's).
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, now_ms: i64, st: &crate::style::Styles) {
        view::draw(self, frame, area, now_ms, st);
    }
}

impl DetailView {
    fn new(kind: Kind, number: u64) -> DetailView {
        DetailView {
            kind,
            number,
            scroll: 0,
            rows: 10,
            doc: None,
            diff: None,
        }
    }

    fn max_scroll(&self) -> usize {
        self.doc
            .as_ref()
            .map_or(0, |d| d.lines.len().saturating_sub(self.rows))
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll());
    }
}
