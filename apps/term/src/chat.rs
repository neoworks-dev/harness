//! The chat pane's state: the session's view items, their rendered lines (cached), and the
//! scroll position. The terminal's own scrollback is not used; the pane scrolls itself.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use neo_protocol::{
    Component, DiffComponent, DiffLineKind, ItemKind, ItemState, ViewItem, ViewPage, ViewPatch,
    ViewSnapshot,
};
use neo_renderer_ratatui::{
    ButtonRef, DevRef, ImageOptions, ImageSlot, ImageStatus, Options, Palette, UiDocInfo, UiState,
    buttons, file_icon, image_sources, neoui_docs, neoui_refs, relative_path, render_item_images,
};

use crate::util::width;
use crate::widgets::truncate_start;
use neo_sdk::ViewState;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

// The layout of `impl Chat`, and the lines it is made of.
mod layout;
mod lines;
pub use lines::*;

/// Lines of diff or code a prompt shows before the buttons (Ctrl-O shows all of it).
const PREVIEW_LINES: usize = 12;

/// Prose wraps at this many columns however wide the pane; tables and code use all of it.
const PROSE_WIDTH: usize = 100;

/// Finished tool calls in a row fold into one line from this many on (Ctrl-O shows them).
const FOLD_MIN: usize = 3;

/// How tall a picture is at most, in pixels, and with tool results expanded (Ctrl-O): it
/// takes the pane's width unless that would make it taller.
const IMAGE_PX: u16 = 500;
const IMAGE_PX_EXPANDED: u16 = 1000;
/// The rows those are in a terminal whose cell height is not known.
const CELL_PX: u16 = 20;

/// What the pane knows about images: the terminal's cell size and the state of each source.
/// Without it images stay the one-line placeholder.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatImages {
    pub cell: (u16, u16),
    pub status: Arc<HashMap<String, ImageStatus>>,
    /// Charts are pictures (`/settings` charts as images, and a terminal that draws them).
    pub charts: bool,
}

/// An image to draw: its source and the cells it covers, relative to the pane's top left. `y`
/// is negative while the image's top has scrolled out of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub src: String,
    pub x: u16,
    pub y: i32,
    pub cols: u16,
    pub rows: u16,
}

/// What the pane draws besides the items.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatOpts {
    pub expanded: bool,
    pub show_thoughts: bool,
    pub palette: Palette,
    /// The prompt being answered: item ref and the focused button.
    pub active: Option<(String, usize)>,
    /// The active prompt is drawn in place of the editor: the chat shows one line for it
    /// (the whole card when expanded).
    pub in_panel: bool,
    pub marker: Style,
    /// The visual state of neoui docs (focus, tabs, edit buffers), per item and doc.
    pub ui: HashMap<String, HashMap<String, UiState>>,
    /// File paths start with their file type icon.
    pub icons: bool,
    /// The last turn is still going: its changed files aren't summed up yet.
    pub running: bool,
    /// Diffs side by side where they fit (`/diff split`).
    pub split_diffs: bool,
    /// Only the newest edit shows its changes, the others their one line (`/settings
    /// hide-edits`); off, every edit shows them.
    pub hide_edits: bool,
    /// The choices the user dismissed (Esc), by ref: not drawn while they are.
    pub dismissed: Vec<String>,
    /// The item the terminal panel is at (a command's ref): drawn with the bar in the margin
    /// that the panel puts on that command's rows, so the two can be told apart from the rest.
    pub linked: Option<String>,
}

/// A file a turn changed, as the tools' diffs name it, and the lines added and removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

/// What Tab picks in the chat: a line that stands for a file (an edit's row, a line of a turn's
/// changes), or a reply of the agent's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    /// The entry it is in.
    r#ref: String,
    what: Pick,
    line: usize,
    /// Lines it takes.
    len: usize,
}

/// What a line that can be picked stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// A changed file, by its path.
    File(String),
    /// A reply, all of it.
    Reply,
    /// A commit, issue or pull request a card shows.
    Ref(DevRef),
}

#[derive(Debug, Clone, PartialEq)]
struct CacheKey {
    rev: u64,
    width: u16,
    expanded: bool,
    thoughts: bool,
    icons: bool,
    split_diffs: bool,
    /// An edit that shows what it changed: every one, or with hide-edits the newest.
    open_edit: bool,
    focused: Option<usize>,
    /// The item is the one the terminal panel is at.
    linked: bool,
    palette: Palette,
    marker: Style,
    /// Only for items that hold images: they redraw when an image's state changes.
    images: Option<ChatImages>,
    /// Only for items that hold neoui docs.
    ui: Option<HashMap<String, UiState>>,
}

struct Cached {
    key: CacheKey,
    lines: Rc<Vec<Line<'static>>>,
    slots: Rc<Vec<ImageSlot>>,
    focus: Option<usize>,
    /// Its code has its syntax colors (else it is plain, from the layout).
    lit: bool,
    /// The commits, issues and pull requests its cards show, by the line each starts on.
    refs: Rc<Vec<(usize, DevRef)>>,
}

struct Entry {
    r#ref: String,
    /// The agent is thinking (thoughts hidden): drawn as the animated line instead.
    thinking: bool,
    /// A tool call that is running: a light sweeps across its first line.
    running: bool,
    lines: Rc<Vec<Line<'static>>>,
    slots: Rc<Vec<ImageSlot>>,
    /// The line of a neoui target that has focus.
    focus: Option<usize>,
    offset: usize,
    /// Done: its code has its colors, or it has none to get (a folded run, a turn's changes).
    lit: bool,
}

/// The inputs of the flat layout; when none changed it is reused.
#[derive(Debug, Clone, PartialEq)]
struct LayoutKey {
    width: u16,
    opts: ChatOpts,
    generation: u64,
    images: Option<ChatImages>,
}

#[derive(Default)]
pub struct Chat {
    state: ViewState,
    revs: HashMap<String, u64>,
    next_rev: u64,
    /// Bumped on every change to the items.
    generation: u64,
    cache: HashMap<String, Cached>,
    buttons: HashMap<String, Vec<ButtonRef>>,
    /// The neoui docs of the items that have any with something to focus.
    docs: HashMap<String, Vec<UiDocInfo>>,
    /// Scroll to the focused neoui target of this item at the next layout.
    reveal: Option<String>,
    /// Sources of the images of the items that have any.
    sources: HashMap<String, Vec<String>>,
    images: Option<ChatImages>,
    entries: Vec<Entry>,
    layout_key: Option<LayoutKey>,
    total: usize,
    height: usize,
    /// Cells across, as laid out last.
    width: usize,
    /// First visible line while scrolled up; None follows the tail.
    top: Option<usize>,
    /// Output arrived while scrolled up.
    unseen: bool,
    /// Whether a snapshot has arrived for the shown session.
    loaded: bool,
    /// The colors of the last layout, for the thinking animation.
    palette: Palette,
    /// This app's name: the user's messages from it are drawn without their tag.
    app: Option<String>,
    /// The session's directory: paths in it are shown relative to it.
    cwd: Option<String>,
    /// The files, replies and refs that can be picked, top to bottom, as laid out last.
    targets: Vec<Target>,
    /// The target picked with the keys: its entry and what it stands for.
    selected: Option<(String, Pick)>,
    /// The picked target is drawn highlighted (the chat has the keys).
    highlight: bool,
    /// Text selected with the mouse: from where the drag started to where it is, as (line,
    /// cell) of the whole chat.
    mouse: Option<((usize, usize), (usize, usize))>,
    /// The item whose first line says Ctrl-B sends its command to the background.
    background_hint: Option<String>,
    /// When each tool call started (its `tool_call` record), by item ref: a running one says
    /// how long it has run.
    started: HashMap<String, i64>,
    /// Tool calls that read a file, by item ref: Ctrl-O leaves them their summary, else every
    /// file the agent looked at would show in full. An image read is still shown larger.
    reads: std::collections::HashSet<String>,
    /// The entry at the top of the view and the line of it there, to keep in place through the
    /// next layout (Ctrl-O while scrolled up).
    anchor: Option<(String, usize)>,
    /// Older items were asked for (`view.page`) and haven't come yet.
    paging: bool,
}

/// A running call shows how long it has run once it has taken this long.
const ELAPSED_AFTER_MS: i64 = 1000;

/// What the line of a command Ctrl-B can send to the background ends with.
const BACKGROUND_HINT: &str = "ctrl-b background";

impl Chat {
    pub fn new() -> Chat {
        Chat::default()
    }

    /// The pane of the app `app`: the user's messages from it leave out their `· app` tag.
    pub fn for_app(app: &str) -> Chat {
        Chat {
            app: Some(app.to_string()),
            ..Chat::default()
        }
    }

    /// Paths in `cwd` (the session's directory) are shown relative to it.
    pub fn in_dir(self, cwd: &str) -> Chat {
        Chat {
            cwd: Some(cwd.to_string()),
            ..self
        }
    }

    /// The session moved (/cd): paths are shown relative to `cwd` now, so every item draws
    /// again.
    pub fn set_dir(&mut self, cwd: &str) {
        self.cwd = Some(cwd.to_string());
        for rev in self.revs.values_mut() {
            self.next_rev += 1;
            *rev = self.next_rev;
        }
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn items(&self) -> &[ViewItem] {
        &self.state.items
    }

    pub fn apply_snapshot(&mut self, snapshot: ViewSnapshot) {
        self.state.apply_snapshot(snapshot);
        self.loaded = true;
        self.paging = false;
        self.revs.clear();
        self.buttons.clear();
        self.docs.clear();
        self.sources.clear();
        for i in 0..self.state.items.len() {
            self.register(i);
        }
        self.generation += 1;
    }

    /// What the pane keeps about item `i` besides the item: its revision, buttons, docs and
    /// images.
    fn register(&mut self, i: usize) {
        let item = &self.state.items[i];
        self.next_rev += 1;
        self.revs.insert(item.r#ref.clone(), self.next_rev);
        let b = buttons(item);
        if !b.is_empty() {
            self.buttons.insert(item.r#ref.clone(), b);
        }
        let d = focusable_docs(item);
        if !d.is_empty() {
            self.docs.insert(item.r#ref.clone(), d);
        }
        let srcs = image_sources(item);
        if !srcs.is_empty() {
            self.sources.insert(item.r#ref.clone(), srcs);
        }
    }

    /// The item to page back from when older ones should come (the snapshot was a tail): the
    /// chat doesn't fill two screens yet, or the view is within a screen of its top. None
    /// while a page is on its way, or when there is nothing older.
    pub fn wants_page(&self) -> Option<&str> {
        if self.paging || self.state.before == 0 || self.height == 0 {
            return None;
        }
        let near_top = self.total < 2 * self.height || self.top() < self.height;
        near_top.then(|| self.state.items.first().map(|i| i.r#ref.as_str()))?
    }

    /// A page was asked for: no other until it comes (or fails).
    pub fn paging(&mut self, on: bool) {
        self.paging = on;
    }

    /// Older items in front of the ones held; scrolled up, the view stays on what it showed.
    pub fn prepend(&mut self, page: ViewPage) {
        self.paging = false;
        let held = self.state.items.len();
        self.keep_place();
        self.state.prepend(page);
        for i in 0..self.state.items.len() - held {
            self.register(i);
        }
        self.generation += 1;
    }

    pub fn apply_patch(&mut self, patch: ViewPatch) {
        for r in self.state.apply_patch(patch) {
            self.touch(&r);
        }
        self.generation += 1;
    }

    fn touch(&mut self, r: &str) {
        self.next_rev += 1;
        self.revs.insert(r.to_string(), self.next_rev);
        match self.state.items.iter().rev().find(|i| i.r#ref == r) {
            Some(item) => {
                let b = buttons(item);
                if b.is_empty() {
                    self.buttons.remove(r);
                } else {
                    self.buttons.insert(r.to_string(), b);
                }
                let d = focusable_docs(item);
                if d.is_empty() {
                    self.docs.remove(r);
                } else {
                    self.docs.insert(r.to_string(), d);
                }
                let srcs = image_sources(item);
                if srcs.is_empty() {
                    self.sources.remove(r);
                } else {
                    self.sources.insert(r.to_string(), srcs);
                }
            }
            None => {
                self.revs.remove(r);
                self.buttons.remove(r);
                self.docs.remove(r);
                self.sources.remove(r);
                self.cache.remove(r);
            }
        }
    }

    pub fn item(&self, r: &str) -> Option<&ViewItem> {
        self.state.items.iter().rev().find(|i| i.r#ref == r)
    }

    /// The session's open surfaces, in the order they opened (`crate::surfaces`).
    pub fn surfaces(&self) -> &[ViewItem] {
        &self.state.surfaces
    }

    pub fn surface(&self, r: &str) -> Option<&ViewItem> {
        self.state.surfaces.iter().find(|s| s.r#ref == r)
    }

    /// The edit a prompt asks about: its first diff that names a file.
    pub fn diff_of(&self, r: &str) -> Option<&DiffComponent> {
        lines::first_diff(&self.item(r)?.component)
    }

    pub fn buttons_of(&self, r: &str) -> &[ButtonRef] {
        self.buttons.get(r).map_or(&[], Vec::as_slice)
    }

    /// Whether the item has a neoui doc with fields or buttons that can be used now.
    pub fn is_form(&self, r: &str) -> bool {
        self.docs
            .get(r)
            .is_some_and(|d| d.iter().any(|d| d.interactive && !d.streaming))
    }

    /// Whether the item has neoui targets that can take focus now (fields, buttons, tabs,
    /// accordion items).
    pub fn has_targets(&self, r: &str) -> bool {
        self.docs
            .get(r)
            .is_some_and(|d| d.iter().any(|d| d.focusable && !d.streaming))
    }

    /// Whether keys can answer the item: it has buttons or neoui targets.
    pub fn is_prompt(&self, r: &str) -> bool {
        self.buttons.contains_key(r) || self.has_targets(r)
    }

    /// Refs of items that have buttons or neoui targets, oldest first.
    pub fn prompt_refs(&self) -> Vec<&str> {
        self.state
            .items
            .iter()
            .filter(|i| self.is_prompt(&i.r#ref))
            .map(|i| i.r#ref.as_str())
            .collect()
    }

    /// Whether an item since the user last spoke has neoui targets: the latest reply has
    /// something to use.
    pub fn latest_has_targets(&self) -> bool {
        self.state
            .items
            .iter()
            .rev()
            .take_while(|i| i.kind != ItemKind::User)
            .any(|i| self.has_targets(&i.r#ref))
    }

    /// The items with buttons still waiting for an answer, and the neoui forms the user has
    /// not spoken after, oldest first.
    pub fn pending_prompts(&self) -> Vec<&str> {
        let last_user = self
            .state
            .items
            .iter()
            .rposition(|i| i.kind == ItemKind::User);
        self.state
            .items
            .iter()
            .enumerate()
            .filter(|(at, i)| {
                let asking =
                    self.buttons.contains_key(&i.r#ref) && i.state == Some(ItemState::Pending);
                asking || (self.is_form(&i.r#ref) && last_user.is_none_or(|u| *at > u))
            })
            .map(|(_, i)| i.r#ref.as_str())
            .collect()
    }

    /// Scroll to the item's focused neoui target once the next layout knows where it is.
    pub fn reveal_focus(&mut self, r: &str) {
        self.reveal = Some(r.to_string());
    }

    /// Brings the line of the focused neoui target (and the one under it) into view.
    fn apply_reveal(&mut self) {
        let Some(r) = self.reveal.take() else {
            return;
        };
        let Some(line) = self
            .entries
            .iter()
            .find(|e| e.r#ref == r)
            .and_then(|e| e.focus.map(|l| e.offset + l))
        else {
            return;
        };
        let (top, height) = (self.top(), self.height.max(1));
        if line < top {
            self.top = Some(line.min(self.max_top()));
        } else if line + 2 > top + height {
            self.top = Some((line + 2 - height).min(self.max_top()));
        } else {
            return;
        }
        if self.top == Some(self.max_top()) {
            self.follow();
        }
    }

    // ---- images ----

    /// Turns real images on (with what is known about them) or off. Changes take effect at the
    /// next layout.
    pub fn set_images(&mut self, images: Option<ChatImages>) {
        self.images = images;
    }

    /// The sources of every image in the conversation.
    pub fn image_sources(&self) -> impl Iterator<Item = &str> {
        self.sources.values().flatten().map(String::as_str)
    }

    /// The images in view, as laid out last: where to draw them over the blank lines the
    /// renderer reserved. A slot is listed while any row of it shows.
    pub fn placements(&self) -> Vec<Placement> {
        let top = self.top();
        let bottom = top + self.height;
        let first = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= top);
        let mut out = Vec::new();
        for e in &self.entries[first..] {
            if e.offset >= bottom {
                break;
            }
            for s in e.slots.iter() {
                let at = e.offset + s.line;
                if at + usize::from(s.rows) > top && at < bottom {
                    out.push(Placement {
                        src: s.src.clone(),
                        x: s.x,
                        y: at as i32 - top as i32,
                        cols: s.cols,
                        rows: s.rows,
                    });
                }
            }
        }
        out
    }

    // ---- scrolling ----

    pub fn follow(&mut self) {
        self.top = None;
        self.unseen = false;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let cur = self.top();
        self.top = Some(cur.saturating_sub(lines));
        if self.max_top() == 0 {
            self.follow();
        }
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let Some(cur) = self.top else {
            return;
        };
        let to = cur + lines;
        if to >= self.max_top() {
            self.follow();
        } else {
            self.top = Some(to);
        }
    }

    pub fn page_up(&mut self) {
        self.scroll_up(self.height.saturating_sub(1).max(1));
    }

    pub fn page_down(&mut self) {
        self.scroll_down(self.height.saturating_sub(1).max(1));
    }

    pub fn half_page_up(&mut self) {
        self.scroll_up((self.height / 2).max(1));
    }

    pub fn half_page_down(&mut self) {
        self.scroll_down((self.height / 2).max(1));
    }

    /// Scrolls just enough to bring the item into view (its top, if it is taller than the pane).
    pub fn reveal(&mut self, r: &str) {
        let Some(e) = self.entries.iter().find(|e| e.r#ref == r) else {
            return;
        };
        let (start, end) = (e.offset, e.offset + e.lines.len());
        let top = self.top();
        if start >= top && end <= top + self.height {
            return;
        }
        if end > top + self.height && end - start <= self.height {
            self.top = Some((end - self.height).min(self.max_top()));
        } else {
            self.top = Some(start.min(self.max_top()));
        }
        if self.top == Some(self.max_top()) {
            self.follow();
        }
    }

    /// Scrolls so the item starts at the top of the view (as far as the end of the chat lets
    /// it): the terminal panel moving through the commands takes the chat along with it.
    pub fn scroll_to(&mut self, r: &str) {
        let Some(e) = self.entries.iter().find(|e| e.r#ref == r) else {
            return;
        };
        let top = e.offset.min(self.max_top());
        if self.top() == top {
            return;
        }
        self.top = Some(top);
        if top == self.max_top() {
            self.follow();
        }
    }

    pub fn to_top(&mut self) {
        if self.max_top() > 0 {
            self.top = Some(0);
        }
    }

    /// Brings a line into view (with one line of room under it when there is).
    fn reveal_line(&mut self, line: usize) {
        let (top, height) = (self.top(), self.height.max(1));
        if line < top {
            self.top = Some(line.min(self.max_top()));
        } else if line + 1 >= top + height {
            self.top = Some((line + 2).saturating_sub(height).min(self.max_top()));
        } else {
            return;
        }
        if self.top == Some(self.max_top()) {
            self.follow();
        }
    }

    // ---- files and replies ----

    /// Whether anything can be picked: a line that stands for a file (an edit, a turn's
    /// changes) or a reply.
    pub fn has_picks(&self) -> bool {
        !self.targets.is_empty()
    }

    fn selected_index(&self) -> Option<usize> {
        let (r, what) = self.selected.as_ref()?;
        self.targets
            .iter()
            .position(|f| &f.r#ref == r && &f.what == what)
    }

    /// Whether the picked target is drawn highlighted.
    pub fn set_highlight(&mut self, on: bool) {
        self.highlight = on;
    }

    /// The item (`c-<call id>`) whose command Ctrl-B would send to the background, if any.
    pub fn set_background_hint(&mut self, r: Option<String>) {
        self.background_hint = r;
    }

    /// A tool call started at `ts` (its first `tool_call` record): item `c-<call id>`.
    pub fn tool_started(&mut self, r: &str, ts: i64) {
        self.started.entry(r.to_string()).or_insert(ts);
    }

    /// Scrolled up, the line at the top of the view stays there through the next layout, however
    /// much the items above it grow or shrink (Ctrl-O). Following the tail, the tail stays.
    pub fn keep_place(&mut self) {
        if self.top.is_none() {
            return;
        }
        let top = self.top();
        let at = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= top);
        self.anchor = self
            .entries
            .get(at)
            .map(|e| (e.r#ref.clone(), top.saturating_sub(e.offset)));
    }

    /// Where the kept place is now: the entry's new offset and the same line of it (its last
    /// when it got shorter). An item folded away is found by the entry it went into.
    fn anchored_top(&self, r: &str, within: usize) -> Option<usize> {
        let items = &self.state.items;
        let index = |r: &str| items.iter().position(|i| i.r#ref == r);
        let entry = self.entries.iter().find(|e| e.r#ref == r).or_else(|| {
            let want = index(r)?;
            self.entries
                .iter()
                .rev()
                .find(|e| index(&e.r#ref).is_some_and(|i| i <= want))
        })?;
        Some(entry.offset + within.min(entry.lines.len().saturating_sub(1)))
    }

    /// The tool call of item `r` reads a file: Ctrl-O doesn't show what it read.
    pub fn tool_reads(&mut self, r: &str) {
        if self.reads.insert(r.to_string()) {
            self.generation += 1;
        }
    }

    /// While scrolled up, the items at or above the view's last line, oldest first; None while
    /// following the tail (everything is).
    pub fn items_through_view(&self) -> Option<&[ViewItem]> {
        self.top?;
        let bottom = self.top() + self.height;
        let items = &self.state.items;
        // The first item that starts below the view; a turn's changes aren't an item.
        let below = self.entries.partition_point(|e| e.offset < bottom);
        let end = self.entries[below..]
            .iter()
            .find_map(|e| items.iter().position(|i| i.r#ref == e.r#ref))
            .unwrap_or(items.len());
        Some(&items[..end])
    }

    /// Picks the n-th target (counted from the end when negative) and scrolls to it: a reply
    /// as much of it as fits, from its top.
    pub fn pick(&mut self, n: isize) {
        let len = self.targets.len() as isize;
        if len == 0 {
            return;
        }
        let i = if n < 0 { len + n } else { n }.clamp(0, len - 1) as usize;
        let t = &self.targets[i];
        self.selected = Some((t.r#ref.clone(), t.what.clone()));
        let (line, end) = (t.line, t.line + t.len - 1);
        self.reveal_line(end.min(line + self.height.saturating_sub(2)));
        self.reveal_line(line);
    }

    /// Moves the pick by `by` targets; with none picked, starts from the last.
    pub fn move_pick(&mut self, by: isize) {
        match self.selected_index() {
            Some(i) => self.pick((i as isize + by).max(0)),
            None => self.pick(-1),
        }
    }

    /// The path of the picked file.
    pub fn picked_file(&self) -> Option<&str> {
        match &self.targets[self.selected_index()?].what {
            Pick::File(path) => Some(path),
            _ => None,
        }
    }

    /// The ref of the picked reply.
    pub fn picked_reply(&self) -> Option<&str> {
        self.selected_index()
            .filter(|&i| self.targets[i].what == Pick::Reply)
            .map(|i| self.targets[i].r#ref.as_str())
    }

    /// The commit, issue or pull request picked.
    pub fn picked_ref(&self) -> Option<&DevRef> {
        match &self.targets[self.selected_index()?].what {
            Pick::Ref(dev) => Some(dev),
            _ => None,
        }
    }

    /// What a click on the chat's line `at` opens: the file or the ref whose line it is (a
    /// reply is all of its lines, so a click in one doesn't open it).
    pub fn pick_at(&self, at: usize) -> Option<Pick> {
        self.targets
            .iter()
            .find(|t| t.line == at && t.what != Pick::Reply)
            .map(|t| t.what.clone())
    }

    // ---- the mouse ----

    /// Where the pane's cell (`col`, `row`) is in the whole chat: its line and cell.
    pub fn at_cell(&self, col: u16, row: u16) -> (usize, usize) {
        (self.top() + usize::from(row), usize::from(col))
    }

    /// A drag starts (`extend` false) or goes on to a line and cell of the chat.
    pub fn select_to(&mut self, at: (usize, usize), extend: bool) {
        let at = (at.0.min(self.total.saturating_sub(1)), at.1);
        self.mouse = match self.mouse {
            Some((from, _)) if extend => Some((from, at)),
            _ => Some((at, at)),
        };
    }

    /// A drag is under way.
    pub fn selecting(&self) -> bool {
        self.mouse.is_some()
    }

    pub fn clear_selection(&mut self) {
        self.mouse = None;
    }

    /// The mouse's selection in order, from its first cell to its last (both in it); None
    /// while it is a single cell (a click).
    pub fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let (a, b) = self.mouse?;
        (a != b).then(|| if a <= b { (a, b) } else { (b, a) })
    }

    /// The line of a press let go where it started (a click, not a selection).
    pub fn clicked(&self) -> Option<usize> {
        let (a, b) = self.mouse?;
        (a == b).then_some(a.0)
    }

    /// The text of the mouse's selection, each line's trailing blanks cut.
    pub fn selected_text(&self) -> Option<String> {
        let ((l0, c0), (l1, c1)) = self.selection()?;
        let mut out = Vec::new();
        for line in l0..=l1 {
            let text = self.line_text(line);
            let from = if line == l0 { c0 } else { 0 };
            let to = if line == l1 { c1 + 1 } else { usize::MAX };
            out.push(cells(&text, from, to).trim_end().to_string());
        }
        Some(out.join("\n"))
    }

    /// A line of the chat as plain text (blank between entries).
    fn line_text(&self, at: usize) -> String {
        let i = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= at);
        self.entries
            .get(i)
            .filter(|e| e.offset <= at)
            .and_then(|e| e.lines.get(at - e.offset))
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
