//! The GitHub view: a full-screen overlay (`/github`, Ctrl-R) to read a repository's issues and
//! pull requests with their comments, to hand one to the model, and to comment, react and close
//! (see `write.rs`).
//!
//! Everything comes from the `gh` CLI, reads checked against an allowlist and the few writes
//! named there, on background threads: the UI thread only
//! starts jobs ([`Github::key`]) and picks up their answers ([`Github::tick`]), so it never
//! blocks. The repository is the one `gh` finds for the session's directory. Lists (per tab,
//! state filter and search) and details are cached until the directory changes; `r` reloads.
//!
//! - **List**: tabs Issues / Pull requests (Tab, 1, 2), Up/Down/j/k, PgUp/PgDn, g/G, Enter opens,
//!   `/` searches (GitHub search syntax), `o` toggles open / all, `r` reloads, Esc or q close.
//! - **Detail**: blocks (the header, the description, each comment, review and comment on the
//!   diff, the checks, the files) stepped through with j/k, g/G; the keys act on the one in
//!   focus (see `write.rs`). Enter goes into it to select text (`visual.rs`), Esc comes out,
//!   then goes back to the list; q closes. Under it the input for a comment, Tab to type.
//!   A pull request's files open one at a time in the diff view and its checks' logs in a
//!   reader (see `review.rs`).
//! - **To the model**: `a` puts the item as a `<github-issue>` /
//!   `<github-pr>` block into the prompt and closes the view; `s` sends it right away with a
//!   short instruction. Both are [`Outgoing`] values for the shell to carry out.

mod complete;
mod doc;
mod edit;
mod md;
mod pr;
mod review;
mod view;
mod visual;
mod write;

// The read-only `gh` boundary and its data are shared with the desktop app.
pub use neo_github::{format, gh, model};

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use neo_renderer_ratatui::Palette;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::widgets::DiffView;
use doc::Part;
use gh::{Gh, GhCli, GhError};
use model::{Detail, Kind, Label, Person, Summary};
use neo_github::fetch;

/// Rows one wheel notch moves.
const WHEEL_ROWS: usize = 3;

/// What the shell does with a key or a finished job.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    /// Put this text into the prompt editor.
    Attach(String),
    /// Send this text as a user message now.
    Send(String),
    /// Put this text on the clipboard; the view stays open.
    Copy(String),
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
    /// What the organization's issue fields are set to (issues only, once asked).
    fields: Vec<(String, String)>,
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
    /// How many label colors were known: the lines are built again when more come.
    colors: usize,
    /// Room was left for avatars (the terminal draws pictures).
    pictures: bool,
    lines: Vec<Line<'static>>,
    avatars: Vec<doc::Avatar>,
    blocks: Vec<doc::Block>,
}

struct DetailView {
    kind: Kind,
    number: u64,
    scroll: usize,
    /// Body rows of the last draw.
    rows: usize,
    doc: Option<Doc>,
    diff: Option<DiffView>,
    /// The block in focus, which the keys act on. Kept by what it is, so it stays when the
    /// lines are built again.
    focus: Part,
    /// Inside the block in focus (Enter): the vim cursor over its lines (see `visual.rs`).
    cur: Option<visual::Cursor>,
    /// Inside the checks or the files (Enter): the one picked, by its row in the block.
    row: Option<usize>,
    /// The file of the pull request `diff` shows, by its place in the detail's files.
    file: Option<usize>,
    /// A check's log, over the detail.
    log: Option<Log>,
}

/// A check's log being read: which check, whether only its failed steps, and the text.
struct Log {
    check: usize,
    failed: bool,
    reader: crate::reader::Reader,
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
    /// A job's log (by job id; only the failed steps or all).
    Log(u64, bool, Result<String, GhError>),
    /// A file of a pull request at its head commit (by number and path).
    File(u64, String, Result<String, GhError>),
    /// Which reactions on an item (of the version updated at the time) are the user's, as
    /// `fetch::my_reactions` finds them.
    Mine(Kind, u64, i64, Result<fetch::MyReactions, GhError>),
    /// A write finished: what the footer says when it worked, or after `failed` when it did not.
    Wrote {
        failed: String,
        result: Result<String, GhError>,
    },
    /// What a list needed from GitHub (milestones, types, fields, projects).
    Loaded(edit::Which, Result<edit::Loaded, GhError>),
    /// What the issue fields of an item are set to.
    Fields(Kind, u64, Result<Vec<(String, String)>, GhError>),
    Labels(Result<Vec<Label>, GhError>),
    People(Result<Vec<Person>, GhError>),
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
    /// The suggestion chosen with Up / Down while typing; Tab or Enter takes it.
    pick: Option<usize>,
    /// What the search suggests from: asked for once per repository, when typing starts.
    labels: Vec<Label>,
    people: Vec<Person>,
    facets_asked: bool,
    lists: HashMap<ListKey, ListData>,
    details: HashMap<(Kind, u64), DetailData>,
    diffs: HashMap<u64, String>,
    /// Jobs' logs, by job and whether only the failed steps.
    logs: HashMap<(u64, bool), String>,
    /// Files of pull requests as they are at a head commit (`f`), by commit and path.
    whole_files: HashMap<(String, String), String>,
    /// The time of the last draw, for how old the review comments in a diff are.
    now_ms: i64,
    mode: Mode,
    /// A fetch an attach / send waits for.
    pending: Option<(Kind, u64, Intent)>,
    /// An error to show in the footer until the next key.
    note: Option<String>,
    /// What the footer says about a write that worked, until the next key.
    info: Option<String>,
    /// Typing a comment, or choosing among what a key offers (see `write.rs`).
    writing: Option<write::Writing>,
    /// The comment being written. It stays while the box is left with Esc, to be added to
    /// (selections are quoted into it) and to be come back to with `c`.
    draft: crate::editor::Editor,
    /// The description or comment being edited.
    body_edit: crate::editor::Editor,
    /// A comment on its way, to put back in the draft if it does not arrive.
    unsent: Option<String>,
    /// A review comment on its way from a file's diff (by the file's path), to put back on its
    /// lines if it does not arrive.
    unsent_line: Option<(String, crate::widgets::DiffComment)>,
    /// What milestones, types, fields and projects there are, once a list asked for them.
    cache: edit::Cache,
    /// The repository's owner has no issue fields (asking said so): not asked again.
    no_fields: bool,
    /// A write is on its way.
    sending: bool,
    /// Asks for which reactions are the user's that have not been answered yet.
    mine_asked: usize,
    /// Body rows of the last list draw.
    list_rows: usize,
    /// Jobs started whose answer has not come yet (each sends one).
    in_flight: std::cell::Cell<usize>,
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
            pick: None,
            labels: Vec::new(),
            people: Vec::new(),
            facets_asked: false,
            lists: HashMap::new(),
            details: HashMap::new(),
            diffs: HashMap::new(),
            logs: HashMap::new(),
            whole_files: HashMap::new(),
            now_ms: 0,
            mode: Mode::List,
            pending: None,
            note: None,
            info: None,
            writing: None,
            cache: edit::Cache::default(),
            no_fields: false,
            draft: crate::editor::Editor::default(),
            body_edit: crate::editor::Editor::default(),
            unsent: None,
            unsent_line: None,
            sending: false,
            mine_asked: 0,
            list_rows: 10,
            in_flight: std::cell::Cell::new(0),
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

    /// Shows the view at issue or pull request `number` of the session's repository (Esc goes
    /// on to its list).
    pub fn open_number(&mut self, kind: Kind, number: u64) {
        self.open();
        self.tab = kind;
        self.ensure_list();
        if !self.details.contains_key(&(kind, number)) {
            self.load_detail(kind, number);
        }
        self.mode = Mode::Detail(DetailView::new(kind, number));
    }

    /// The repository `gh` found for the session's directory, once it has.
    pub fn repo(&self) -> Option<&str> {
        self.repo_name()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.typing = None;
        // A comment half written goes with it; one on its way finishes.
        if !self.sending {
            self.writing = None;
        }
    }

    // ---- jobs ----

    fn spawn(&self, job: impl FnOnce(&dyn Gh, &std::path::Path) -> Msg + Send + 'static) {
        let (gh, cwd, tx) = (self.gh.clone(), self.cwd.clone(), self.tx.clone());
        self.in_flight.set(self.in_flight.get() + 1);
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
                self.spawn(|gh, cwd| Msg::Repo(fetch::repo(gh, cwd)));
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
            let result = fetch::list(gh, cwd, key.kind, &repo, key.all, &key.query);
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
            let result = fetch::detail(gh, cwd, kind, &repo, number).map(Box::new);
            Msg::Detail(kind, number, result)
        });
    }

    /// Asks for the labels and people the search suggests from, once. Failing is fine: the
    /// search then suggests only qualifiers.
    fn ask_facets(&mut self) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        if std::mem::replace(&mut self.facets_asked, true) {
            return;
        }
        let r = repo.clone();
        self.spawn(move |gh, cwd| Msg::Labels(fetch::labels(gh, cwd, &r)));
        self.spawn(move |gh, cwd| Msg::People(fetch::people(gh, cwd, &repo)));
    }

    /// The colors of the repository's labels, by name.
    fn label_colors(&self) -> view::Colors {
        self.labels
            .iter()
            .map(|l| (l.name.clone(), l.rgb))
            .collect()
    }

    /// What the search being typed suggests.
    fn suggestions(&self) -> Vec<complete::Candidate> {
        match &self.typing {
            Some(text) => complete::suggest(text, self.tab, &self.labels, &self.people),
            None => Vec::new(),
        }
    }

    /// Asks what the organization's issue fields of a loaded issue are set to.
    fn ask_fields(&mut self, kind: Kind, number: u64) {
        if kind != Kind::Issue || self.no_fields {
            return;
        }
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.mine_asked += 1;
        self.spawn(move |gh, cwd| {
            Msg::Fields(kind, number, fetch::field_values(gh, cwd, &repo, number))
        });
    }

    /// Asks which of the reactions on a loaded item are the user's, if it has any.
    fn ask_mine(&mut self, kind: Kind, number: u64) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        let Some(detail) = self
            .details
            .get(&(kind, number))
            .and_then(|d| d.detail.as_ref())
            .filter(|d| {
                !d.reactions.is_empty() || d.comments.iter().any(|c| !c.reactions.is_empty())
            })
            .cloned()
        else {
            return;
        };
        let updated = detail.updated_ms;
        self.mine_asked += 1;
        self.spawn(move |gh, cwd| {
            Msg::Mine(
                kind,
                number,
                updated,
                fetch::my_reactions(gh, cwd, &repo, &detail),
            )
        });
    }

    fn load_diff(&mut self, number: u64) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.spawn(move |gh, cwd| Msg::Diff(number, fetch::diff(gh, cwd, &repo, number)));
    }

    /// Picks up finished jobs. Called on every loop iteration.
    pub fn tick(&mut self) -> Tick {
        let mut tick = Tick::default();
        while let Ok(msg) = self.rx.try_recv() {
            tick.changed = true;
            self.in_flight.set(self.in_flight.get().saturating_sub(1));
            match msg {
                Msg::Repo(Ok(name)) => {
                    self.repo = Repo::Ready(name);
                    self.ensure_list();
                    // The labels give the chips their colors, and the search its choices.
                    self.ask_facets();
                    // Opened at an issue or pull request before the repository was known.
                    if let Mode::Detail(v) = &self.mode
                        && !self.details.contains_key(&(v.kind, v.number))
                    {
                        self.load_detail(v.kind, v.number);
                    }
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
                    let mut refreshed = None;
                    match result {
                        Ok(detail) => {
                            data.error = None;
                            let before = data.detail.as_ref().map(|d| d.head_sha.clone());
                            refreshed = Some(before.is_some_and(|b| b != detail.head_sha));
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
                    if let Some(pushed) = refreshed {
                        self.pull_refreshed(number, pushed);
                    }
                    self.ask_mine(kind, number);
                    self.ask_fields(kind, number);
                }
                Msg::Loaded(which, result) => self.loaded(which, result),
                Msg::Fields(kind, number, result) => match result {
                    Ok(values) => {
                        self.mine_asked = self.mine_asked.saturating_sub(1);
                        if let Some(data) = self.details.get_mut(&(kind, number)) {
                            data.fields = values;
                            data.version += 1;
                        }
                    }
                    // Not an organization with issue fields: not asked again.
                    Err(_) => {
                        self.mine_asked = self.mine_asked.saturating_sub(1);
                        self.no_fields = true;
                    }
                },
                Msg::Mine(kind, number, updated_ms, found) => {
                    // gh does not say which reactions are the user's: they come separately,
                    // and only count for the version of the item they were asked about.
                    self.mine_asked = self.mine_asked.saturating_sub(1);
                    if let Some(data) = self.details.get_mut(&(kind, number))
                        && let Some(detail) = &mut data.detail
                        && detail.updated_ms == updated_ms
                        && let Ok(found) = found
                    {
                        for (who, words) in found {
                            let reactions = match who {
                                None => Some(&mut detail.reactions),
                                Some(i) => detail.comments.get_mut(i).map(|c| &mut c.reactions),
                            };
                            for r in reactions.into_iter().flatten() {
                                r.mine = words.contains(&r.content);
                            }
                        }
                        data.version += 1;
                    }
                }
                Msg::Wrote { failed, result } => self.wrote(failed, result),
                Msg::Labels(result) => self.labels = result.unwrap_or_default(),
                Msg::People(result) => self.people = result.unwrap_or_default(),
                Msg::Diff(number, result) => self.diff_arrived(number, result),
                Msg::Log(job, failed, result) => self.log_arrived(job, failed, result),
                Msg::File(number, path, result) => self.file_arrived(number, &path, result),
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
        if !self.sending {
            self.info = None;
        }
        if self.writing.is_some() {
            self.writing_key(key);
            return None;
        }
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
        if self.paste_into_comment(text) {
            return;
        }
        if let Some(t) = &mut self.typing {
            t.extend(text.chars().filter(|c| !c.is_control()));
            self.pick = None;
        }
    }

    /// The mouse wheel: moves the selection, scrolls the detail or the diff.
    pub fn wheel(&mut self, down: bool) {
        let delta = if down { 1 } else { -1 };
        match &mut self.mode {
            Mode::List => self.move_selection(delta * WHEEL_ROWS as isize),
            Mode::Detail(v) if v.log.is_some() => {
                if let Some(log) = &mut v.log {
                    log.reader.wheel(down);
                }
            }
            Mode::Detail(v) => match &mut v.diff {
                Some(diff) => {
                    let code = if down { KeyCode::Down } else { KeyCode::Up };
                    for _ in 0..WHEEL_ROWS {
                        diff.key(KeyEvent::from(code));
                    }
                }
                None => v.scroll_freely(delta * WHEEL_ROWS as isize),
            },
        }
    }

    fn typing_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let suggestions = self.suggestions();
        let n = suggestions.len();
        let Some(text) = &mut self.typing else {
            return;
        };
        // The suggestion Tab takes, and Enter once one was chosen with the arrows.
        let chosen = self.pick.filter(|i| *i < n);
        match key.code {
            KeyCode::Esc => self.typing = None,
            KeyCode::Char('c') if ctrl => self.typing = None,
            KeyCode::Up if n > 0 => {
                self.pick = Some(chosen.map_or(n - 1, |i| i.checked_sub(1).unwrap_or(n - 1)));
            }
            KeyCode::Down if n > 0 => self.pick = Some(chosen.map_or(0, |i| (i + 1) % n)),
            KeyCode::Tab if n > 0 => {
                *text = complete::accept(text, &suggestions[chosen.unwrap_or(0)]);
                self.pick = None;
            }
            KeyCode::Enter if chosen.is_some() => {
                *text = complete::accept(text, &suggestions[chosen.unwrap_or(0)]);
                self.pick = None;
            }
            KeyCode::Enter => {
                self.query = text.trim().to_string();
                self.typing = None;
                self.ensure_list();
            }
            KeyCode::Char('u') if ctrl => {
                text.clear();
                self.pick = None;
            }
            KeyCode::Backspace => {
                text.pop();
                self.pick = None;
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                text.push(c);
                self.pick = None;
            }
            _ => {}
        }
        if self.typing.is_none() {
            self.pick = None;
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
            (KeyCode::Char('/'), _) => {
                self.typing = Some(self.query.clone());
                self.pick = None;
                self.ask_facets();
            }
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
        // Over the detail: a check's log, a file's diff.
        if v.log.is_some() {
            return self.log_key(key);
        }
        if v.diff.is_some() {
            return self.diff_key(key);
        }
        if v.row.is_some()
            && let Some(out) = self.row_key(key)
        {
            return out;
        }
        if !self.in_scope(key) || self.write_hotkey(key) {
            return None;
        }
        match self.cursor_key(key) {
            Some(visual::Keyed::Add(text)) => {
                self.add_to_message(&text);
                return None;
            }
            Some(visual::Keyed::Handled) => return None,
            _ => {}
        }
        let Mode::Detail(v) = &mut self.mode else {
            return None;
        };
        let (kind, number) = (v.kind, v.number);
        let inside = v.cur.is_some();
        let page = v.rows.saturating_sub(1).max(1) as isize;
        match (key.code, ctrl) {
            // Out of the block, then back to the list.
            (KeyCode::Esc, _) if inside => v.cur = None,
            (KeyCode::Esc, _) => self.mode = Mode::List,
            (KeyCode::Char('q'), false) => self.close(),
            (KeyCode::Up | KeyCode::Char('k'), _) => v.step(false),
            (KeyCode::Down | KeyCode::Char('j'), _) => v.step(true),
            (KeyCode::PageUp | KeyCode::Char('b'), _) => v.scroll_freely(-page),
            (KeyCode::PageDown | KeyCode::Char(' '), _) => v.scroll_freely(page),
            (KeyCode::Char('u'), true) => v.scroll_freely(-(page / 2).max(1)),
            (KeyCode::Char('d'), true) => v.scroll_freely((page / 2).max(1)),
            (KeyCode::Home | KeyCode::Char('g'), _) => v.focus_at(0, true),
            (KeyCode::End | KeyCode::Char('G'), _) => v.focus_at(usize::MAX, false),
            (KeyCode::Enter, _) if !inside => v.enter(),
            (KeyCode::Tab | KeyCode::Char('c'), _) => {
                if self
                    .details
                    .get(&(kind, number))
                    .is_some_and(|d| d.detail.is_some())
                {
                    self.writing = Some(write::Writing::Comment);
                }
            }
            (KeyCode::Char('r'), false) => self.load_detail(kind, number),
            (KeyCode::Char('a'), false) => return self.request(kind, number, 0, Intent::Attach),
            (KeyCode::Char('s'), false) => return self.request(kind, number, 0, Intent::Send),
            (KeyCode::Char('d'), false) if kind == Kind::Pr => self.open_file(0),
            _ => {}
        }
        None
    }

    /// Inside a block, a key for the vim cursor: what it did. The view follows the cursor.
    fn cursor_key(&mut self, key: KeyEvent) -> Option<visual::Keyed> {
        let Mode::Detail(v) = &mut self.mode else {
            return None;
        };
        let page = v.rows.saturating_sub(1).max(1);
        let start = v.block().map_or(0, |b| b.start);
        let cur = v.cur.as_mut()?;
        let keyed = cur.key(key, page);
        if keyed != visual::Keyed::Pass {
            let row = start + cur.row();
            if row < v.scroll {
                v.scroll = row;
            } else if row >= v.scroll + v.rows {
                v.scroll = row + 1 - v.rows;
            }
        }
        Some(keyed)
    }

    // ---- drawing ----

    /// Fills `area` (the chat's).
    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        now_ms: i64,
        st: &crate::style::Styles,
        images: Option<&mut crate::images::Images>,
    ) {
        view::draw(self, frame, area, now_ms, st, images);
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
            focus: Part::Head,
            cur: None,
            row: None,
            file: None,
            log: None,
        }
    }

    fn max_scroll(&self) -> usize {
        self.doc
            .as_ref()
            .map_or(0, |d| d.lines.len().saturating_sub(self.rows))
    }

    fn blocks(&self) -> &[doc::Block] {
        self.doc.as_ref().map_or(&[], |d| &d.blocks)
    }

    /// Where the block in focus is among the blocks.
    fn index(&self) -> Option<usize> {
        self.blocks().iter().position(|b| b.part == self.focus)
    }

    /// The block in focus, once the lines are built.
    fn block(&self) -> Option<doc::Block> {
        self.index().map(|i| self.blocks()[i])
    }

    /// The first line that goes with block `i`: the gap and heading above it too.
    fn top_of(&self, i: usize) -> usize {
        match i.checked_sub(1) {
            Some(prev) => self.blocks()[prev].end,
            None => 0,
        }
    }

    /// Focuses block `i` (the last when past the end) and scrolls it into view: its top when
    /// coming from above, its bottom from below.
    fn focus_at(&mut self, i: usize, from_above: bool) {
        let n = self.blocks().len();
        if n == 0 {
            return;
        }
        let i = i.min(n - 1);
        let b = self.blocks()[i];
        let top = self.top_of(i);
        self.focus = b.part;
        self.cur = None;
        self.row = None;
        let rows = self.rows.max(1);
        if b.end - top <= rows {
            if top < self.scroll {
                self.scroll = top;
            } else if b.end > self.scroll + rows {
                self.scroll = b.end - rows;
            }
        } else if from_above {
            self.scroll = top;
        } else {
            self.scroll = b.end - rows;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// `j` / `k`: through the rest of a block taller than the view, half a view at a time,
    /// then on to the next one.
    fn step(&mut self, down: bool) {
        let Some(i) = self.index() else {
            if !self.blocks().is_empty() {
                self.focus_at(0, true);
            }
            return;
        };
        let b = self.blocks()[i];
        let half = (self.rows / 2).max(1);
        if down {
            if b.end > self.scroll + self.rows {
                self.scroll = (self.scroll + half)
                    .min(b.end - self.rows)
                    .min(self.max_scroll());
            } else if i + 1 < self.blocks().len() {
                self.focus_at(i + 1, true);
            }
        } else if b.start < self.scroll {
            self.scroll = self.scroll.saturating_sub(half).max(self.top_of(i));
        } else if i > 0 {
            self.focus_at(i - 1, false);
        } else {
            self.scroll = 0;
        }
    }

    /// Scrolls without stepping (wheel, pages): the focus moves to a block in view when its
    /// own leaves it, and out of the block when inside it.
    fn scroll_freely(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll());
        let (top, bottom) = (self.scroll, self.scroll + self.rows);
        let seen = |b: &doc::Block| b.end > top && b.start < bottom;
        if self.block().is_some_and(|b| seen(&b)) {
            return;
        }
        let pick = if delta > 0 {
            self.blocks().iter().find(|b| seen(b))
        } else {
            self.blocks().iter().rev().find(|b| seen(b))
        };
        if let Some(b) = pick {
            self.focus = b.part;
            self.cur = None;
            self.row = None;
        }
    }

    /// Enter: into the block in focus. The checks and the files pick one of their rows; the
    /// rest has the cursor at its start.
    fn enter(&mut self) {
        let Some(b) = self.block() else {
            return;
        };
        if matches!(b.part, Part::Checks | Part::Files) {
            self.pick_row(0);
            return;
        }
        self.cur = Some(visual::Cursor::new(self.block_text(&b)));
        if b.start < self.scroll {
            self.scroll = b.start;
        }
    }

    /// Picks row `i` of the block in focus (the last when past the end) and keeps it in view.
    fn pick_row(&mut self, i: usize) {
        let Some(b) = self.block() else {
            return;
        };
        let i = i.min((b.end - b.start).saturating_sub(1));
        self.row = Some(i);
        let line = b.start + i;
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + self.rows {
            self.scroll = line + 1 - self.rows;
        }
    }

    /// The lines of a block as text, for the cursor to walk.
    fn block_text(&self, b: &doc::Block) -> Vec<Vec<char>> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        doc.lines[b.start..b.end]
            .iter()
            .map(|l| l.spans.iter().flat_map(|s| s.content.chars()).collect())
            .collect()
    }
}
