//! The chat pane's state: the session's view items, their rendered lines (cached), and the
//! scroll position. The terminal's own scrollback is not used; the pane scrolls itself.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use neo_protocol::{
    Component, DiffLineKind, ItemKind, ItemState, ViewItem, ViewPatch, ViewSnapshot,
};
use neo_renderer_ratatui::{
    ButtonRef, ImageOptions, ImageSlot, ImageStatus, Options, Palette, UiDocInfo, UiState, buttons,
    file_icon, image_sources, openui_docs, relative_path, render_item_images,
};

use crate::util::width;
use crate::widgets::truncate_start;
use neo_sdk::ViewState;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Lines of diff or code a prompt shows before the buttons (Ctrl-O shows all of it).
const PREVIEW_LINES: usize = 12;

/// Prose wraps at this many columns however wide the pane; tables and code use all of it.
const PROSE_WIDTH: usize = 100;

/// Finished tool calls in a row fold into one line from this many on (Ctrl-O shows them).
const FOLD_MIN: usize = 3;

/// Rows an image thumbnail takes at most, and with tool results expanded (Ctrl-O).
const IMAGE_ROWS: u16 = 12;
const IMAGE_ROWS_EXPANDED: u16 = 30;

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
    /// The visual state of OpenUI docs (focus, tabs, edit buffers), per item and doc.
    pub ui: HashMap<String, HashMap<String, UiState>>,
    /// File paths start with their file type icon.
    pub icons: bool,
    /// The last turn is still going: its changed files aren't summed up yet.
    pub running: bool,
    /// Diffs side by side where they fit (`/diff split`).
    pub split_diffs: bool,
}

/// A file a turn changed, as the tools' diffs name it, and the lines added and removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

/// A line in the chat that stands for a file: an edit's row or a line of a turn's changes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileTarget {
    /// The entry the line is in.
    r#ref: String,
    path: String,
    line: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct CacheKey {
    rev: u64,
    width: u16,
    expanded: bool,
    thoughts: bool,
    icons: bool,
    split_diffs: bool,
    /// The newest edit: it shows what it changed, the others are one line.
    open_edit: bool,
    focused: Option<usize>,
    palette: Palette,
    marker: Style,
    /// Only for items that hold images: they redraw when an image's state changes.
    images: Option<ChatImages>,
    /// Only for items that hold OpenUI docs.
    ui: Option<HashMap<String, UiState>>,
}

struct Cached {
    key: CacheKey,
    lines: Rc<Vec<Line<'static>>>,
    slots: Rc<Vec<ImageSlot>>,
    focus: Option<usize>,
    /// Its code has its syntax colors (else it is plain, from the layout).
    lit: bool,
}

struct Entry {
    r#ref: String,
    /// The agent is thinking (thoughts hidden): drawn as the animated line instead.
    thinking: bool,
    /// A tool call that is running: a light sweeps across its first line.
    running: bool,
    lines: Rc<Vec<Line<'static>>>,
    slots: Rc<Vec<ImageSlot>>,
    /// The line of an OpenUI target that has focus.
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
    /// The OpenUI docs of the items that have any with something to focus.
    docs: HashMap<String, Vec<UiDocInfo>>,
    /// Scroll to the focused OpenUI target of this item at the next layout.
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
    /// The lines that stand for files, top to bottom, as laid out last.
    files: Vec<FileTarget>,
    /// The file picked with the keys: its entry and path.
    selected: Option<(String, String)>,
    /// The picked file's line is drawn highlighted (the chat has the keys).
    highlight: bool,
}

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
        self.revs.clear();
        self.buttons.clear();
        self.docs.clear();
        self.sources.clear();
        for item in &self.state.items {
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

    pub fn buttons_of(&self, r: &str) -> &[ButtonRef] {
        self.buttons.get(r).map_or(&[], Vec::as_slice)
    }

    /// Whether the item has an OpenUI doc with fields or buttons that can be used now.
    pub fn is_form(&self, r: &str) -> bool {
        self.docs
            .get(r)
            .is_some_and(|d| d.iter().any(|d| d.interactive && !d.streaming))
    }

    /// Whether the item has OpenUI targets that can take focus now (fields, buttons, tabs,
    /// accordion items).
    pub fn has_targets(&self, r: &str) -> bool {
        self.docs
            .get(r)
            .is_some_and(|d| d.iter().any(|d| d.focusable && !d.streaming))
    }

    /// Whether keys can answer the item: it has buttons or OpenUI targets.
    pub fn is_prompt(&self, r: &str) -> bool {
        self.buttons.contains_key(r) || self.has_targets(r)
    }

    /// Refs of items that have buttons or OpenUI targets, oldest first.
    pub fn prompt_refs(&self) -> Vec<&str> {
        self.state
            .items
            .iter()
            .filter(|i| self.is_prompt(&i.r#ref))
            .map(|i| i.r#ref.as_str())
            .collect()
    }

    /// Whether an item since the user last spoke has OpenUI targets: the latest reply has
    /// something to use.
    pub fn latest_has_targets(&self) -> bool {
        self.state
            .items
            .iter()
            .rev()
            .take_while(|i| i.kind != ItemKind::User)
            .any(|i| self.has_targets(&i.r#ref))
    }

    /// The items with buttons still waiting for an answer, and the OpenUI forms the user has
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

    /// Scroll to the item's focused OpenUI target once the next layout knows where it is.
    pub fn reveal_focus(&mut self, r: &str) {
        self.reveal = Some(r.to_string());
    }

    /// Brings the line of the focused OpenUI target (and the one under it) into view.
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

    // ---- layout ----

    /// Lays the items out for `width` x `height`; reuses rendered lines of untouched items.
    pub fn layout(&mut self, width: u16, height: u16, opts: &ChatOpts) {
        self.height = usize::from(height);
        self.width = usize::from(width);
        let key = LayoutKey {
            width,
            opts: opts.clone(),
            generation: self.generation,
            images: self.images.clone(),
        };
        if self.layout_key.as_ref() == Some(&key) {
            self.apply_reveal();
            self.light_visible();
            return;
        }
        let was_total = self.total;
        let items = &self.state.items;
        let last = items.len().saturating_sub(1);
        // The items drawn, and whether each is the thinking line.
        let shown: Vec<(usize, bool)> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                // A question the user allowed has done its job: the tool's own row says the rest.
                if is_allowed_question(item) {
                    return None;
                }
                // Hidden thoughts show while the agent thinks, as one animated line, and then go.
                let thinking = item.kind == ItemKind::Thought && !opts.show_thoughts;
                (!thinking || index == last).then_some((index, thinking))
            })
            .collect();
        let mut entries: Vec<Entry> = Vec::with_capacity(shown.len());
        let mut files: Vec<FileTarget> = Vec::new();
        let mut summaries = turn_changes(items, opts.running).into_iter().peekable();
        // Only the newest edit shows what it changed (Ctrl-O opens them all).
        let last_edit = items.iter().rposition(is_edit);
        // A blank row between entries, tool calls too.
        let mut offset = 0;
        let mut k = 0;
        loop {
            // A finished turn's changed files go after its last item.
            let next = shown.get(k).map(|&(index, _)| index);
            while let Some((end, changed)) =
                summaries.next_if(|(end, _)| next.is_none_or(|n| *end < n))
            {
                if !entries.is_empty() {
                    offset += 1;
                }
                let r = format!("{}#changes", items[end].r#ref);
                let lines = changes_lines(&changed, usize::from(width), opts, self.cwd.as_deref());
                for (j, c) in changed.iter().enumerate() {
                    files.push(FileTarget {
                        r#ref: r.clone(),
                        path: c.path.clone(),
                        line: offset + 1 + j,
                    });
                }
                let len = lines.len();
                entries.push(Entry {
                    r#ref: r,
                    thinking: false,
                    running: false,
                    lines: Rc::new(lines),
                    slots: Rc::new(Vec::new()),
                    focus: None,
                    offset,
                    lit: true,
                });
                offset += len;
            }
            if k >= shown.len() {
                break;
            }
            let (index, thinking) = shown[k];
            let item = &items[index];
            let r = &item.r#ref;
            let tool_row =
                item.kind == ItemKind::Tool && !self.buttons.contains_key(r) && !self.is_form(r);
            // A run of finished calls that something came after is folded to one line.
            if !opts.expanded && foldable(item, &self.buttons, &self.docs) {
                let end = (k..shown.len())
                    .find(|&j| !foldable(&items[shown[j].0], &self.buttons, &self.docs))
                    .unwrap_or(shown.len());
                if end - k >= FOLD_MIN && end < shown.len() {
                    let run: Vec<&ViewItem> =
                        shown[k..end].iter().map(|&(i, _)| &items[i]).collect();
                    if !entries.is_empty() {
                        offset += 1;
                    }
                    entries.push(Entry {
                        r#ref: r.clone(),
                        thinking: false,
                        running: false,
                        lines: Rc::new(vec![folded_line(&run, usize::from(width), &opts.palette)]),
                        slots: Rc::new(Vec::new()),
                        focus: None,
                        offset,
                        lit: true,
                    });
                    offset += 1;
                    k = end;
                    continue;
                }
            }
            k += 1;
            let rev = self.revs.get(r).copied().unwrap_or(0);
            let active = opts
                .active
                .as_ref()
                .filter(|(a, _)| a == r)
                .map(|(_, b)| *b);
            let ckey = CacheKey {
                rev,
                width,
                expanded: opts.expanded,
                thoughts: opts.show_thoughts,
                icons: opts.icons,
                split_diffs: opts.split_diffs,
                open_edit: Some(index) == last_edit,
                focused: active,
                palette: opts.palette,
                marker: opts.marker,
                images: self
                    .sources
                    .contains_key(r)
                    .then(|| self.images.clone())
                    .flatten(),
                ui: self
                    .docs
                    .contains_key(r)
                    .then(|| opts.ui.get(r).cloned().unwrap_or_default()),
            };
            let (lines, slots, focus, lit) = match self.cache.get(r) {
                // Ctrl-O (everything in full) keeps the whole card, e.g. a long diff.
                _ if opts.in_panel
                    && active.is_some()
                    && !opts.expanded
                    && !self.docs.contains_key(r) =>
                {
                    (
                        Rc::new(vec![panel_stub(item, opts)]),
                        Rc::new(Vec::new()),
                        None,
                        true,
                    )
                }
                Some(c) if c.key == ckey => (c.lines.clone(), c.slots.clone(), c.focus, c.lit),
                // Without syntax colors: they are worked out for what comes into view.
                _ => {
                    let (app, cwd) = (self.app.as_deref(), self.cwd.as_deref());
                    let (lines, slots, focus) = render(item, width, &ckey, app, cwd, false);
                    let (lines, slots) = (Rc::new(lines), Rc::new(slots));
                    self.cache.insert(
                        r.clone(),
                        Cached {
                            key: ckey,
                            lines: lines.clone(),
                            slots: slots.clone(),
                            focus,
                            lit: false,
                        },
                    );
                    (lines, slots, focus, false)
                }
            };
            if lines.is_empty() {
                continue;
            }
            if !entries.is_empty() {
                offset += 1;
            }
            if tool_row && let Some(first) = changed_files(item).into_iter().next() {
                files.push(FileTarget {
                    r#ref: r.clone(),
                    path: first.path,
                    line: offset,
                });
            }
            let len = lines.len();
            entries.push(Entry {
                r#ref: r.clone(),
                thinking,
                running: tool_row && item.state == Some(ItemState::Running),
                lines,
                slots,
                focus,
                offset,
                lit,
            });
            offset += len;
        }
        let live = items.len();
        if self.cache.len() > live {
            let items = &self.state.items;
            self.cache
                .retain(|r, _| items.iter().any(|i| &i.r#ref == r));
        }
        self.entries = entries;
        self.files = files;
        if self.selected_index().is_none() {
            self.selected = None;
        }
        self.palette = opts.palette;
        self.total = offset;
        self.layout_key = Some(key);
        if self.top.is_some() && self.total > was_total {
            self.unseen = true;
        }
        if let Some(top) = self.top {
            self.top = Some(top.min(self.max_top()));
        }
        self.apply_reveal();
        self.light_visible();
    }

    /// Gives the code of the entries in view (and a screen above and below) its syntax colors.
    /// The lines stay where they are: colors don't change where a line wraps. Highlighting a
    /// whole long conversation at once (Ctrl-O) took a noticeable moment.
    fn light_visible(&mut self) {
        let (top, height) = (self.top(), self.height.max(1));
        let (from, to) = (top.saturating_sub(height), top + 2 * height);
        let first = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= from);
        let (app, cwd) = (self.app.as_deref(), self.cwd.as_deref());
        for e in &mut self.entries[first..] {
            if e.offset >= to {
                break;
            }
            if e.lit {
                continue;
            }
            e.lit = true;
            let item = self.state.items.iter().rev().find(|i| i.r#ref == e.r#ref);
            let (Some(item), Some(c)) = (item, self.cache.get_mut(&e.r#ref)) else {
                continue;
            };
            if c.lit {
                (e.lines, e.slots, e.focus) = (c.lines.clone(), c.slots.clone(), c.focus);
                continue;
            }
            let (lines, slots, focus) = render(item, c.key.width, &c.key, app, cwd, true);
            // Should colors ever move a line, the plain lines stay: the layout counted them.
            if lines.len() != e.lines.len() {
                continue;
            }
            (c.lines, c.slots, c.focus, c.lit) = (Rc::new(lines), Rc::new(slots), focus, true);
            (e.lines, e.slots, e.focus) = (c.lines.clone(), c.slots.clone(), c.focus);
        }
    }

    pub fn total_lines(&self) -> usize {
        self.total
    }

    fn max_top(&self) -> usize {
        self.total.saturating_sub(self.height)
    }

    /// The first visible line.
    pub fn top(&self) -> usize {
        self.top.unwrap_or_else(|| self.max_top())
    }

    pub fn following(&self) -> bool {
        self.top.is_none()
    }

    pub fn has_unseen(&self) -> bool {
        self.unseen
    }

    /// The lines in view (at most the pane's height), the thinking line still.
    pub fn visible(&self) -> Vec<Line<'static>> {
        self.visible_at(None)
    }

    /// The lines in view, the thinking line and running calls animated for the time `now_ms`.
    pub fn visible_at(&self, now_ms: Option<i64>) -> Vec<Line<'static>> {
        let top = self.top();
        let bottom = top + self.height;
        let mut out: Vec<Line<'static>> = Vec::with_capacity(self.height);
        let first = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= top);
        let picked = self
            .selected_index()
            .filter(|_| self.highlight)
            .map(|i| self.files[i].line);
        let mut at = top;
        for e in &self.entries[first..] {
            if e.offset >= bottom {
                break;
            }
            while at < e.offset {
                out.push(Line::default());
                at += 1;
            }
            let skip = at - e.offset;
            for line in e.lines.iter().skip(skip) {
                if at >= bottom {
                    break;
                }
                match now_ms {
                    Some(now) if e.thinking => out.push(thinking_line(line, now, &self.palette)),
                    Some(now) if e.running && at == e.offset => {
                        out.push(sweep(line, now, &self.palette));
                    }
                    _ if picked == Some(at) => out.push(picked_line(line, self.width)),
                    _ => out.push(line.clone()),
                }
                at += 1;
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

    // ---- files ----

    /// Whether any line stands for a file (an edit, a turn's changes).
    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    fn selected_index(&self) -> Option<usize> {
        let (r, path) = self.selected.as_ref()?;
        self.files
            .iter()
            .position(|f| &f.r#ref == r && &f.path == path)
    }

    /// Whether the picked file's line is drawn highlighted.
    pub fn set_highlight(&mut self, on: bool) {
        self.highlight = on;
    }

    /// Picks the n-th file line (counted from the end when negative) and scrolls to it.
    pub fn pick_file(&mut self, n: isize) {
        let len = self.files.len() as isize;
        if len == 0 {
            return;
        }
        let i = if n < 0 { len + n } else { n }.clamp(0, len - 1) as usize;
        let f = &self.files[i];
        self.selected = Some((f.r#ref.clone(), f.path.clone()));
        self.reveal_line(f.line);
    }

    /// Moves the pick by `by` file lines; with none picked, starts from the last.
    pub fn move_file(&mut self, by: isize) {
        match self.selected_index() {
            Some(i) => self.pick_file((i as isize + by).max(0)),
            None => self.pick_file(-1),
        }
    }

    /// The path of the picked file.
    pub fn picked_file(&self) -> Option<&str> {
        self.selected_index().map(|i| self.files[i].path.as_str())
    }
}

/// The diffs in an item's result, in order (a path twice is summed).
fn changed_files(item: &ViewItem) -> Vec<Changed> {
    fn walk(c: &Component, out: &mut Vec<Changed>) {
        match c {
            Component::CardComponent(v) => v.children.iter().for_each(|c| walk(c, out)),
            Component::RowComponent(v) => v.children.iter().for_each(|c| walk(c, out)),
            Component::StackComponent(v) => v.children.iter().for_each(|c| walk(c, out)),
            Component::DiffComponent(v) => {
                let Some(path) = v.path.as_deref().filter(|p| !p.is_empty()) else {
                    return;
                };
                let count = |kind| v.lines.iter().filter(|l| l.kind == kind).count();
                add_changed(
                    out,
                    Changed {
                        path: path.to_string(),
                        added: count(DiffLineKind::Add),
                        removed: count(DiffLineKind::Remove),
                    },
                );
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&item.component, &mut out);
    out
}

fn add_changed(all: &mut Vec<Changed>, c: Changed) {
    match all.iter_mut().find(|x| x.path == c.path) {
        Some(x) => {
            x.added += c.added;
            x.removed += c.removed;
        }
        None => all.push(c),
    }
}

/// An edit the agent made: a tool call that ran, with a diff, that isn't a question.
fn is_edit(item: &ViewItem) -> bool {
    item.kind == ItemKind::Tool
        && item.state == Some(ItemState::Done)
        && !item.r#ref.starts_with("p-")
        && !item.r#ref.starts_with("q-")
        && !changed_files(item).is_empty()
}

/// Each finished turn that changed files: the index of its last item and the files, summed per
/// path. A turn ends where the next user message starts; the last one once it isn't running.
fn turn_changes(items: &[ViewItem], running: bool) -> Vec<(usize, Vec<Changed>)> {
    let mut out = Vec::new();
    let mut turn: Vec<Changed> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if item.kind == ItemKind::User && i > 0 && !turn.is_empty() {
            out.push((i - 1, std::mem::take(&mut turn)));
        }
        if is_edit(item) {
            for c in changed_files(item) {
                add_changed(&mut turn, c);
            }
        }
    }
    if !running && !turn.is_empty() {
        out.push((items.len() - 1, turn));
    }
    out
}

/// A turn's changes: `3 files changed  +40 −12`, then each file with its icon and counts, its
/// path relative to `cwd` when it is in there.
fn changes_lines(
    changed: &[Changed],
    width_cells: usize,
    opts: &ChatOpts,
    cwd: Option<&str>,
) -> Vec<Line<'static>> {
    let p = &opts.palette;
    let muted = Style::new().fg(p.muted);
    let counts = |added: usize, removed: usize| {
        vec![
            Span::styled(format!("+{added}"), Style::new().fg(p.success)),
            Span::raw(" "),
            Span::styled(format!("−{removed}"), Style::new().fg(p.danger)),
        ]
    };
    let (added, removed) = changed
        .iter()
        .fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
    let n = changed.len();
    let mut head = vec![Span::styled(
        format!("{n} file{} changed  ", if n == 1 { "" } else { "s" }),
        muted,
    )];
    head.extend(counts(added, removed));
    let mut out = vec![Line::from(head)];
    for c in changed {
        let mut spans = vec![Span::raw("  ")];
        if opts.icons {
            spans.push(file_icon(&c.path, p));
            spans.push(Span::raw(" "));
        }
        let tail = counts(c.added, c.removed);
        let used: usize = spans.iter().chain(&tail).map(|s| width(&s.content)).sum();
        let room = width_cells.saturating_sub(used + 2).max(1);
        spans.push(Span::raw(truncate_start(relative_path(&c.path, cwd), room)));
        spans.push(Span::raw("  "));
        spans.extend(tail);
        out.push(Line::from(spans));
    }
    out
}

/// The picked file's line: reversed and bold across the whole pane, so it can't be missed
/// whatever the colors (`NO_COLOR` too).
fn picked_line(line: &Line<'static>, width_cells: usize) -> Line<'static> {
    let mut line = line.clone();
    let used: usize = line.spans.iter().map(|s| width(&s.content)).sum();
    line.spans
        .push(Span::raw(" ".repeat(width_cells.saturating_sub(used))));
    for s in &mut line.spans {
        s.style = s.style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
    }
    line
}

/// A tool call that can fold into a run: finished, not a question (`p-`/`q-`, which say what was
/// answered), not an edit (what was written stays in sight) and not a command (each says what it
/// was for, on a line of its own).
fn foldable(
    item: &ViewItem,
    buttons: &HashMap<String, Vec<ButtonRef>>,
    docs: &HashMap<String, Vec<UiDocInfo>>,
) -> bool {
    item.kind == ItemKind::Tool
        && matches!(item.state, Some(ItemState::Done | ItemState::Failed))
        && !buttons.contains_key(&item.r#ref)
        && !docs
            .get(&item.r#ref)
            .is_some_and(|d| d.iter().any(|d| d.interactive))
        && !item.r#ref.starts_with("p-")
        && !item.r#ref.starts_with("q-")
        && item.command.is_none()
        && changed_files(item).is_empty()
}

/// A folded run: `15 tool calls · 7 fetch, 5 read, 3 search · 1 failed`, or `12 read` when all
/// are one tool; all muted but the failures.
fn folded_line(run: &[&ViewItem], width: usize, p: &Palette) -> Line<'static> {
    let mut verbs: Vec<(String, usize)> = Vec::new();
    for item in run {
        let verb = item
            .label
            .as_deref()
            .unwrap_or("tool")
            .split_whitespace()
            .next()
            .unwrap_or("tool")
            .trim_end_matches(':')
            .to_string();
        match verbs.iter_mut().find(|(v, _)| *v == verb) {
            Some((_, n)) => *n += 1,
            None => verbs.push((verb, 1)),
        }
    }
    // Most first; ties in the order they came.
    verbs.sort_by_key(|v| std::cmp::Reverse(v.1));
    let counts: Vec<String> = verbs.iter().map(|(v, n)| format!("{n} {v}")).collect();
    let failed = run
        .iter()
        .filter(|i| i.state == Some(ItemState::Failed))
        .count();
    let muted = Style::new().fg(p.muted);
    let mut text = match counts.as_slice() {
        [one] => one.clone(),
        _ => format!("{} tool calls · {}", run.len(), counts.join(", ")),
    };
    let tail = (failed > 0).then(|| format!(" · {failed} failed"));
    let room = width.saturating_sub(tail.as_ref().map_or(0, |t| t.chars().count()));
    if text.chars().count() > room {
        text = text
            .chars()
            .take(room.saturating_sub(1))
            .collect::<String>()
            + "…";
    }
    let mut spans = vec![Span::styled(text, muted)];
    if let Some(tail) = tail {
        spans.push(Span::styled(tail, Style::new().fg(p.danger)));
    }
    Line::from(spans)
}

/// A permission question that was answered with an allow: the session view marks it done.
fn is_allowed_question(item: &ViewItem) -> bool {
    item.kind == ItemKind::Tool
        && item.r#ref.starts_with("p-")
        && item.state == Some(ItemState::Done)
}

/// How long the light takes to sweep across the thinking line.
const SWEEP_MS: i64 = 1600;

/// The thinking line with a light sweeping over it: muted text, the accent color where the light
/// is. Without true colors the lit characters are the accent color and bold.
fn thinking_line(line: &Line<'static>, now_ms: i64, p: &Palette) -> Line<'static> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as f32;
    // From before the start to past the end, so the light comes in and leaves.
    let phase = now_ms.rem_euclid(SWEEP_MS) as f32 / SWEEP_MS as f32;
    let center = -4.0 + phase * (n + 8.0);
    let spans = chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let light = (1.0 - (i as f32 - center).abs() / 4.0).max(0.0);
            let style = match mix(p.muted, p.accent, light) {
                Some(color) => Style::new().fg(color),
                None if light > 0.5 => Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
                None => Style::new().fg(p.muted),
            };
            Span::styled(c.to_string(), style)
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

/// How far the light of a sweep reaches either side of its center, in columns.
const SWEEP_REACH: f32 = 6.0;

/// The line with a soft light breathing across it, left to right: each character keeps its own
/// style, its color drawn towards the accent where the light is. Without true colors the lit
/// characters are bold.
pub fn sweep(line: &Line<'static>, now_ms: i64, p: &Palette) -> Line<'static> {
    let n = line.width() as f32;
    let phase = now_ms.rem_euclid(SWEEP_MS) as f32 / SWEEP_MS as f32;
    let center = -SWEEP_REACH + phase * (n + 2.0 * SWEEP_REACH);
    let mut col = 0.0;
    let mut spans = Vec::new();
    for span in &line.spans {
        let base = span
            .style
            .fg
            .filter(|c| *c != ratatui::style::Color::Reset)
            .unwrap_or(p.fg);
        for c in span.content.chars() {
            let x = (col - center) / SWEEP_REACH;
            // A bell, not a cone: the light swells and fades instead of edging along.
            let light = if x.abs() < 1.0 {
                (1.0 + (x * std::f32::consts::PI).cos()) / 2.0
            } else {
                0.0
            };
            let style = match mix(base, p.accent, light) {
                Some(color) => span.style.fg(color),
                None if light > 0.5 => span.style.add_modifier(Modifier::BOLD),
                None => span.style,
            };
            spans.push(Span::styled(c.to_string(), style));
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as f32;
        }
    }
    Line::from(spans).style(line.style)
}

/// `b` mixed into `a` by `t` (0 to 1); None unless both are true colors.
pub fn mix(
    a: ratatui::style::Color,
    b: ratatui::style::Color,
    t: f32,
) -> Option<ratatui::style::Color> {
    use ratatui::style::Color;
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (a, b) else {
        return None;
    };
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Some(Color::Rgb(m(r0, r1), m(g0, g1), m(b0, b1)))
}

/// The docs of an item that have anything to focus.
fn focusable_docs(item: &ViewItem) -> Vec<UiDocInfo> {
    openui_docs(item)
        .into_iter()
        .filter(|d| d.focusable)
        .collect()
}

fn render(
    item: &ViewItem,
    width: u16,
    key: &CacheKey,
    app: Option<&str>,
    cwd: Option<&str>,
    lit: bool,
) -> (Vec<Line<'static>>, Vec<ImageSlot>, Option<usize>) {
    let opts = Options {
        plain_code: !lit,
        split_diffs: key.split_diffs,
        open_edits: key.open_edit,
        expanded: key.expanded,
        show_thoughts: key.thoughts,
        focused_button: key.focused,
        hide_keys: false,
        prose_width: Some(PROSE_WIDTH),
        preview_lines: Some(PREVIEW_LINES),
        theme: key.palette,
        images: key.images.as_ref().map(|i| ImageOptions {
            max_rows: if key.expanded {
                IMAGE_ROWS_EXPANDED
            } else {
                IMAGE_ROWS
            },
            cell: i.cell,
            status: i.status.clone(),
            charts: i.charts,
        }),
        openui: key.ui.clone().unwrap_or_default(),
        app: app.map(str::to_string),
        file_icons: key.icons,
        cwd: cwd.map(str::to_string),
    };
    if key.focused.is_none() {
        let out = render_item_images(item, width, &opts);
        return (out.lines, out.images, out.focus_line);
    }
    // The prompt being answered gets a bar in the margin.
    let inner = width.saturating_sub(2).max(1);
    let out = render_item_images(item, inner, &opts);
    let lines = out
        .lines
        .into_iter()
        .map(|line| barred(line, Some(key.marker)))
        .collect();
    let mut slots = out.images;
    for slot in &mut slots {
        slot.x += 2;
    }
    (lines, slots, out.focus_line)
}

/// A line of the prompt being answered: the bar in the margin first. None where something
/// else draws the bar (the input box's).
pub fn barred(line: Line<'static>, marker: Option<Style>) -> Line<'static> {
    let Some(marker) = marker else {
        return line;
    };
    let mut spans = vec![Span::styled("▎ ", marker)];
    spans.extend(line.spans);
    Line::from(spans)
}

/// How the chat draws, for a prompt drawn in the input area to look the same: an edit asked
/// about reads like the edit once made.
#[derive(Debug, Clone, Copy)]
pub struct PanelLook<'a> {
    pub palette: Palette,
    pub icons: bool,
    pub split_diffs: bool,
    /// Paths in it are shown relative to it.
    pub cwd: Option<&'a str>,
}

/// The prompt as the input area shows it: the item with the n-th button focused and at most
/// `preview` lines of any diff or code, each line with the bar in the margin (none without a
/// marker). `hide_keys` draws the buttons without their keys.
pub fn panel_lines(
    item: &ViewItem,
    width: u16,
    focused: usize,
    preview: usize,
    hide_keys: bool,
    look: &PanelLook<'_>,
    marker: Option<Style>,
) -> Vec<Line<'static>> {
    let opts = Options {
        expanded: false,
        show_thoughts: false,
        focused_button: Some(focused),
        hide_keys,
        prose_width: Some(PROSE_WIDTH),
        preview_lines: Some(preview),
        theme: look.palette,
        images: None,
        openui: HashMap::new(),
        app: None,
        file_icons: look.icons,
        cwd: look.cwd.map(str::to_string),
        plain_code: false,
        split_diffs: look.split_diffs,
        open_edits: false,
    };
    let bar = if marker.is_some() { 2 } else { 0 };
    neo_renderer_ratatui::render_item(item, width.saturating_sub(bar).max(1), &opts)
        .into_iter()
        .map(|line| barred(line, marker))
        .collect()
}

/// The one-line input for the answer's note: `note › text` with a cursor, or a hint while empty.
/// A long note shows its end.
pub fn note_line(text: &str, width: usize, accent: Style) -> Line<'static> {
    input_line(
        "note › ",
        "for the agent · enter sends · esc cancels",
        text,
        width,
        accent,
    )
}

/// The one-line filter of a long choice: `filter › text` with a cursor, or a hint while empty.
pub fn filter_line(text: &str, width: usize, accent: Style) -> Line<'static> {
    input_line("filter › ", "type to find an option", text, width, accent)
}

fn input_line(lead: &str, hint: &str, text: &str, width: usize, accent: Style) -> Line<'static> {
    let mut spans = vec![Span::styled(lead.to_string(), accent)];
    if text.is_empty() {
        spans.push(Span::styled(
            " ",
            Style::new().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::styled(
            hint.to_string(),
            Style::new().add_modifier(Modifier::DIM),
        ));
    } else {
        let room = width.saturating_sub(lead.chars().count() + 1).max(1);
        let chars: Vec<char> = text.chars().collect();
        let shown: String = if chars.len() > room {
            std::iter::once('…')
                .chain(chars[chars.len() - (room - 1)..].iter().copied())
                .collect()
        } else {
            text.to_string()
        };
        spans.push(Span::raw(shown));
        spans.push(Span::styled(
            " ",
            Style::new().add_modifier(Modifier::REVERSED),
        ));
    }
    Line::from(spans)
}

/// The line a prompt leaves in the chat while it is answered below the chat.
fn panel_stub(item: &ViewItem, opts: &ChatOpts) -> Line<'static> {
    let label = item.label.as_deref().unwrap_or("a question");
    Line::from(vec![
        Span::styled(label.to_string(), Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" · answer below", Style::new().fg(opts.palette.muted)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_renderer_ratatui::plain_text;
    use serde_json::json;

    pub fn item(r: &str, kind: &str, text: &str) -> ViewItem {
        serde_json::from_value(json!({
            "ref": r, "kind": kind,
            "component": {"type":"text","blocks":[{"type":"paragraph","spans":[{"text": text}]}]}
        }))
        .unwrap()
    }

    pub fn prompt(r: &str, state: &str, labels: &[&str]) -> ViewItem {
        let children: Vec<_> = labels
            .iter()
            .map(|l| json!({"type":"button","label":l,"action":"neo.permit","args":{"id":r,"option":l}}))
            .collect();
        serde_json::from_value(json!({
            "ref": r, "kind": "tool", "label": "asks", "state": state,
            "component": {"type":"row","children": children}
        }))
        .unwrap()
    }

    fn snapshot(items: Vec<ViewItem>) -> ViewSnapshot {
        ViewSnapshot {
            agent: None,
            session: "s".into(),
            seq: 1,
            items,
        }
    }

    fn opts() -> ChatOpts {
        ChatOpts {
            expanded: false,
            show_thoughts: false,
            palette: Palette::default(),
            active: None,
            in_panel: false,
            marker: Style::new(),
            ui: Default::default(),
            icons: false,
            running: false,
            split_diffs: false,
        }
    }

    fn many(n: usize) -> Chat {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(
            (0..n)
                .map(|i| item(&format!("a{i}"), "agent", &format!("line {i}")))
                .collect(),
        ));
        c.layout(40, 10, &opts());
        c
    }

    fn texts(c: &Chat) -> Vec<String> {
        plain_text(&c.visible())
    }

    #[test]
    fn allowed_questions_and_finished_thoughts_are_left_out() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            item("u", "user", "go"),
            item("t1", "thought", "hmm"),
            prompt("p-1", "done", &[]),
            prompt("p-2", "failed", &[]),
            item("a", "agent", "ok"),
            item("t2", "thought", "next"),
        ]));
        c.layout(40, 20, &opts());
        let shown = texts(&c);
        // The refused question stays; the allowed one and the earlier thought go; the agent is
        // thinking again at the end.
        assert_eq!(shown.iter().filter(|l| l.contains("asks")).count(), 1);
        assert_eq!(shown.iter().filter(|l| l.contains("thinking")).count(), 1);
        assert_eq!(shown.last().map(String::as_str), Some("∴ thinking"));
    }

    #[test]
    fn the_thinking_line_has_a_light_that_moves() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![item("t", "thought", "x")]));
        c.layout(40, 5, &opts());
        let colors = |now: i64| -> Vec<Option<ratatui::style::Color>> {
            let lines = c.visible_at(Some(now));
            lines[0].spans.iter().map(|s| s.style.fg).collect()
        };
        let p = Palette::default();
        let early = colors(SWEEP_MS / 3);
        let late = colors(SWEEP_MS * 2 / 3);
        assert_ne!(early, late);
        // Somewhere lit (towards the accent), somewhere not.
        assert!(early.contains(&Some(p.muted)));
        assert!(early.iter().any(|c| *c != Some(p.muted)));
        // The text itself is the same.
        assert_eq!(plain_text(&c.visible_at(Some(5))), ["∴ thinking"]);
    }

    #[test]
    fn items_are_separated_by_a_blank_line_and_follow_the_tail() {
        let c = many(3);
        assert_eq!(c.total_lines(), 5);
        assert_eq!(texts(&c), ["line 0", "", "line 1", "", "line 2"]);
        let c = many(10);
        assert_eq!(c.total_lines(), 19);
        assert!(c.following());
        assert_eq!(texts(&c).last().unwrap(), "line 9");
        assert_eq!(texts(&c).len(), 10);
    }

    fn call(r: &str, label: &str, state: &str) -> ViewItem {
        serde_json::from_value(json!({
            "ref": r, "kind": "tool", "label": label, "state": state,
            "component": {"type":"code","code":"out"}
        }))
        .unwrap()
    }

    #[test]
    fn a_running_call_has_a_light_sweeping_across_its_first_line() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            call("t1", "read a long file name here", "done"),
            call("t2", "read another long file name", "running"),
        ]));
        c.layout(60, 20, &opts());
        let lines = |now: i64| c.visible_at(Some(now));
        let colors =
            |line: &Line<'static>| -> Vec<_> { line.spans.iter().map(|s| s.style.fg).collect() };
        let (early, late) = (lines(SWEEP_MS / 3), lines(SWEEP_MS * 2 / 3));
        let first = texts(&c)
            .iter()
            .position(|l| l.contains("another"))
            .unwrap();
        assert_ne!(colors(&early[first]), colors(&late[first]));
        // Only the running call's first line; the text stays the same.
        assert_eq!(colors(&early[0]), colors(&late[0]));
        assert_eq!(plain_text(&early), texts(&c));
    }

    #[test]
    fn the_sweep_keeps_each_characters_style_where_the_light_is_not() {
        let p = Palette::default();
        let bold = Style::new().fg(p.danger).add_modifier(Modifier::BOLD);
        let line = Line::from(vec![Span::styled("abc", bold), Span::raw(" ".repeat(40))]);
        // The light is far right: the start is as it was.
        let lit = sweep(&line, SWEEP_MS - 1, &p);
        assert_eq!(lit.spans[0].style, bold);
        assert_eq!(lit.width(), line.width());
    }

    #[test]
    fn tool_rows_have_a_blank_line_between_them() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            call("t1", "read a", "done"),
            call("t2", "read b", "running"),
            prompt("p1", "pending", &["Allow"]),
        ]));
        c.layout(40, 20, &opts());
        let t = texts(&c);
        assert_eq!(t[..5], ["read a", "  ⎿ out", "", "read b", "  ⎿ out"]);
        assert_eq!(t[5], "", "a question gets room");
    }

    #[test]
    fn commands_never_fold_and_say_what_they_were_for() {
        let command = |r: &str, description: &str| -> ViewItem {
            serde_json::from_value(json!({
                "ref": r, "kind": "tool", "command": "cargo test", "description": description,
                "state": "done", "component": {"type":"code","code":"out"}
            }))
            .unwrap()
        };
        let mut c = Chat::new();
        let mut items = vec![
            command("c1", "Run the tests"),
            command("c2", "Lint"),
            command("c3", "Format"),
            item("a1", "agent", "Done."),
        ];
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[..5], ["Run the tests", "", "Lint", "", "Format"]);

        // Other calls still fold, and only the failures are red.
        items[..3].clone_from_slice(&[
            call("t1", "read a", "done"),
            call("t2", "read b", "done"),
            call("t3", "search x", "done"),
        ]);
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "3 tool calls · 2 read, 1 search");
        let last = |c: &Chat| c.visible()[0].spans.last().unwrap().style.fg;
        assert_eq!(last(&c), Some(Palette::default().muted));
        items[2] = call("t3", "search x", "failed");
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "3 tool calls · 2 read, 1 search · 1 failed");
        assert_eq!(last(&c), Some(Palette::default().danger));

        // Calls of one tool only say how many.
        items[2] = call("t3", "read c", "failed");
        c.apply_snapshot(snapshot(items));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "3 read · 1 failed");
    }

    #[test]
    fn a_finished_run_of_calls_folds_into_one_line() {
        let mut c = Chat::new();
        let mut items = vec![
            call("t1", "fetch https://a", "done"),
            call("t2", "bash: ls", "done"),
            call("t3", "fetch https://b", "failed"),
            call("t4", "search the web: x", "done"),
        ];
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(
            c.total_lines(),
            11,
            "the tail run is still going: shown in full"
        );

        items.push(item("a1", "agent", "Done."));
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(
            texts(&c),
            [
                "4 tool calls · 2 fetch, 1 bash, 1 search · 1 failed",
                "",
                "Done."
            ]
        );
        // Ctrl-O shows every call.
        c.layout(
            80,
            20,
            &ChatOpts {
                expanded: true,
                ..opts()
            },
        );
        assert!(texts(&c).contains(&"bash: ls".to_string()));

        // Two are not worth folding.
        c.apply_snapshot(snapshot(vec![
            items[0].clone(),
            items[1].clone(),
            items[4].clone(),
        ]));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "fetch https://a");
    }

    fn edit(r: &str, path: &str, added: usize, removed: usize) -> ViewItem {
        let mut lines: Vec<serde_json::Value> = Vec::new();
        lines.extend((0..removed).map(|_| json!({"kind":"remove","text":"old"})));
        lines.extend((0..added).map(|_| json!({"kind":"add","text":"new"})));
        serde_json::from_value(json!({
            "ref": r, "kind": "tool", "label": format!("edit {path}"), "state": "done",
            "component": {"type":"diff","path": path,"lines": lines}
        }))
        .unwrap()
    }

    #[test]
    fn edits_stay_in_sight_and_a_finished_turn_sums_up_its_files() {
        let mut c = Chat::new();
        let items = vec![
            item("u1", "user", "go"),
            call("t1", "read a", "done"),
            edit("e1", "src/a.rs", 3, 1),
            call("t2", "read b", "done"),
            call("t3", "read c", "done"),
            call("t4", "read d", "done"),
            edit("e2", "src/b.rs", 2, 0),
            edit("e3", "src/a.rs", 1, 1),
            item("a1", "agent", "Done."),
        ];
        c.apply_snapshot(snapshot(items.clone()));
        let running = ChatOpts {
            running: true,
            ..opts()
        };
        c.layout(80, 40, &running);
        let shown = texts(&c);
        // The reads around the edits fold; the edits don't.
        assert!(shown.contains(&"3 read".to_string()), "{shown:#?}");
        assert!(shown.contains(&"edit src/a.rs  +3 −1".to_string()));
        assert!(shown.contains(&"edit src/b.rs  +2 −0".to_string()));
        assert!(
            !shown.iter().any(|l| l.contains("changed")),
            "not while it runs"
        );

        c.layout(80, 40, &opts());
        let shown = texts(&c);
        let at = shown
            .iter()
            .position(|l| l == "2 files changed  +6 −2")
            .unwrap();
        assert_eq!(shown[at - 2], "Done.");
        assert_eq!(shown[at + 1], "  src/a.rs  +4 −2");
        assert_eq!(shown[at + 2], "  src/b.rs  +2 −0");

        // A turn the next message ended is summed up even while a later one runs.
        let mut more = items.clone();
        more.push(item("u2", "user", "again"));
        c.apply_snapshot(snapshot(more));
        c.layout(80, 40, &running);
        let shown = texts(&c);
        let at = shown
            .iter()
            .position(|l| l == "2 files changed  +6 −2")
            .unwrap();
        assert_eq!(shown[at + 4], "› again");
    }

    /// A long session: prose with code, commands with output, edits with diffs.
    fn long_session(turns: usize) -> Vec<ViewItem> {
        let code: String = (0..40)
            .map(|i| format!("fn f{i}(x: u32) -> u32 {{ x * {i} }} // note {i}\n"))
            .collect();
        let mut items = Vec::new();
        for t in 0..turns {
            items.push(item(&format!("u{t}"), "user", "do the next thing"));
            items.push(
                serde_json::from_value(json!({
                    "ref": format!("a{t}"), "kind": "agent",
                    "component": {"type":"text","blocks":[
                        {"type":"paragraph","spans":[{"text":"Here is the plan, with some code."}]},
                        {"type":"code","language":"rust","code": code}]}
                }))
                .unwrap(),
            );
            items.push(
                serde_json::from_value(json!({
                    "ref": format!("c{t}"), "kind": "tool", "label": "bash: cargo test",
                    "command": "cargo test -p neo-term 2>&1 | tail -40", "state": "done",
                    "component": {"type":"code","code": code}
                }))
                .unwrap(),
            );
            let lines: Vec<serde_json::Value> = (0..80)
                .map(|i| {
                    json!({"kind": if i % 3 == 0 {"add"} else {"context"},
                                "text": format!("    let x{i} = \"s\".len() + {i};")})
                })
                .collect();
            items.push(
                serde_json::from_value(json!({
                    "ref": format!("e{t}"), "kind": "tool", "label": "edit src/a.rs", "state": "done",
                    "component": {"type":"diff","path":"src/a.rs","lines": lines}
                }))
                .unwrap(),
            );
        }
        items
    }

    #[test]
    fn code_gets_its_colors_as_it_comes_into_view() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(long_session(30)));
        let mut o = opts();
        o.palette.fg = ratatui::style::Color::Rgb(220, 220, 220);
        o.palette.bg = ratatui::style::Color::Rgb(20, 20, 20);
        o.expanded = true;
        c.layout(120, 40, &o);
        // The tail is in view and lit; the top of the conversation isn't yet.
        let lit = |c: &Chat, r: &str| c.entries.iter().find(|e| e.r#ref == r).unwrap().lit;
        assert!(lit(&c, "e29"));
        assert!(!lit(&c, "e0"));
        let plain_lines = c
            .entries
            .iter()
            .find(|e| e.r#ref == "e0")
            .unwrap()
            .lines
            .len();
        c.to_top();
        c.layout(120, 40, &o);
        assert!(lit(&c, "e0"));
        assert_eq!(
            c.entries
                .iter()
                .find(|e| e.r#ref == "e0")
                .unwrap()
                .lines
                .len(),
            plain_lines,
            "the same lines, colored"
        );
        // And colored indeed: some code span has a true color.
        let colored = c.visible().iter().flat_map(|l| l.spans.iter()).any(|s| {
            matches!(s.style.fg, Some(ratatui::style::Color::Rgb(..)))
                && s.style.fg != Some(o.palette.fg)
        });
        assert!(colored);
    }

    #[test]
    #[ignore = "a timing: cargo test --release -p neo-term ctrl_o_timing -- --ignored --nocapture"]
    fn ctrl_o_timing() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(long_session(100)));
        let mut true_color = opts();
        true_color.palette.fg = ratatui::style::Color::Rgb(220, 220, 220);
        true_color.palette.bg = ratatui::style::Color::Rgb(20, 20, 20);
        let t = std::time::Instant::now();
        c.layout(120, 40, &true_color);
        eprintln!("first layout: {:?}", t.elapsed());
        let expanded = ChatOpts {
            expanded: true,
            ..true_color.clone()
        };
        let t = std::time::Instant::now();
        c.layout(120, 40, &expanded);
        eprintln!("ctrl-o: {:?} ({} lines)", t.elapsed(), c.total_lines());
        let t = std::time::Instant::now();
        c.layout(120, 40, &true_color);
        eprintln!("ctrl-o back: {:?}", t.elapsed());
        // Each kind of item on its own, expanded.
        let key = CacheKey {
            rev: 0,
            width: 120,
            expanded: true,
            thoughts: false,
            split_diffs: false,
            open_edit: false,
            icons: false,
            focused: None,
            palette: expanded.palette,
            marker: Style::new(),
            images: None,
            ui: None,
        };
        for kind in ["u", "a", "c", "e"] {
            let t = std::time::Instant::now();
            for it in long_session(100)
                .iter()
                .filter(|i| i.r#ref.starts_with(kind))
            {
                render(it, 120, &key, None, None, true);
            }
            eprintln!("  {kind}: {:?}", t.elapsed());
        }
        let no_path: Vec<ViewItem> = long_session(100)
            .into_iter()
            .filter(|i| i.r#ref.starts_with('e'))
            .map(|mut i| {
                if let neo_protocol::Component::DiffComponent(d) = &mut i.component {
                    d.path = None;
                }
                i
            })
            .collect();
        let t = std::time::Instant::now();
        for it in &no_path {
            render(it, 120, &key, None, None, true);
        }
        eprintln!("  e without a grammar: {:?}", t.elapsed());
        let rust = neo_renderer_ratatui::syntax::syntax_for("a.rs").unwrap();
        let t = std::time::Instant::now();
        for _ in 0..100 {
            let mut h = neo_renderer_ratatui::syntax::Highlighter::new(rust);
            for i in 0..80 {
                h.line(&format!("    let x{i} = \"s\".len() + {i};"));
            }
        }
        eprintln!("  syntect alone, 8000 lines: {:?}", t.elapsed());
    }

    #[test]
    fn paths_in_the_session_directory_lose_its_prefix() {
        let mut c = Chat::new().in_dir("/w/site");
        c.apply_snapshot(snapshot(vec![
            item("u1", "user", "go"),
            edit("e1", "/w/site/src/a.rs", 1, 0),
            edit("e2", "/elsewhere/b.rs", 1, 0),
            item("a1", "agent", "Done."),
        ]));
        c.layout(80, 40, &opts());
        let shown = texts(&c);
        for line in ["edit src/a.rs  +1 −0", "  src/a.rs  +1 −0"] {
            assert!(shown.contains(&line.to_string()), "{line}: {shown:#?}");
        }
        assert!(
            shown.contains(&"  /elsewhere/b.rs  +1 −0".to_string()),
            "{shown:#?}"
        );
        // What is opened is still the whole path.
        c.pick_file(1);
        assert_eq!(c.picked_file(), Some("/elsewhere/b.rs"));
    }

    #[test]
    fn file_lines_are_picked_with_the_keys() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            item("u1", "user", "go"),
            edit("e1", "src/a.rs", 1, 0),
            edit("e2", "src/b.rs", 1, 0),
            item("a1", "agent", "Done."),
        ]));
        c.layout(80, 40, &opts());
        assert!(c.has_files());
        // The two edits, then the two lines of the turn's changes.
        c.pick_file(-1);
        assert_eq!(c.picked_file(), Some("src/b.rs"));
        c.move_file(-1);
        assert_eq!(c.picked_file(), Some("src/a.rs"));
        c.move_file(-1);
        assert_eq!(c.picked_file(), Some("src/b.rs"));
        c.pick_file(0);
        assert_eq!(c.picked_file(), Some("src/a.rs"));
        c.move_file(-1);
        assert_eq!(c.picked_file(), Some("src/a.rs"), "stays at the first");

        // Only drawn picked while the chat has the keys.
        let plain = c.visible();
        c.set_highlight(true);
        let lit = c.visible();
        let differ: Vec<usize> = (0..plain.len()).filter(|&i| plain[i] != lit[i]).collect();
        assert_eq!(differ.len(), 1);
        assert_eq!(
            plain_text(&lit[differ[0]..=differ[0]]),
            ["edit src/a.rs  +1 −0"]
        );
    }

    #[test]
    fn edits_show_what_they_changed_inline_or_side_by_side() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            item("u1", "user", "go"),
            edit("e1", "src/b.rs", 2, 0),
            edit("e2", "src/a.rs", 1, 1),
            item("a1", "agent", "Done."),
        ]));
        c.layout(100, 40, &opts());
        // Only the newest edit is open.
        assert_eq!(
            texts(&c)[2..7],
            [
                "edit src/b.rs  +2 −0",
                "",
                "edit src/a.rs  +1 −1",
                "-old",
                "+new"
            ]
        );
        let split = ChatOpts {
            split_diffs: true,
            ..opts()
        };
        c.layout(100, 40, &split);
        let row = &texts(&c)[5];
        assert!(row.starts_with("-old") && row.contains("│+new"), "{row}");
    }

    #[test]
    fn scrolling_up_pins_the_view_and_new_output_does_not_move_it() {
        let mut c = many(10);
        c.page_up();
        assert!(!c.following());
        let before = texts(&c);
        c.apply_patch(ViewPatch {
            agent: None,
            session: "s".into(),
            seq: 2,
            ops: vec![set(item("new", "agent", "fresh"))],
        });
        c.layout(40, 10, &opts());
        assert_eq!(texts(&c), before, "the view stays put");
        assert!(c.has_unseen());
        c.scroll_down(1000);
        assert!(c.following());
        assert!(!c.has_unseen());
        assert_eq!(texts(&c).last().unwrap(), "fresh");
    }

    #[test]
    fn page_keys_clamp_at_the_ends() {
        let mut c = many(10);
        for _ in 0..10 {
            c.page_up();
        }
        assert_eq!(c.top(), 0);
        assert_eq!(texts(&c)[0], "line 0");
        c.half_page_down();
        assert_eq!(c.top(), 5);
        c.to_top();
        assert_eq!(c.top(), 0);
        for _ in 0..10 {
            c.page_down();
        }
        assert!(c.following());
    }

    #[test]
    fn short_content_does_not_scroll() {
        let mut c = many(2);
        c.page_up();
        assert!(c.following());
        c.to_top();
        assert!(c.following());
    }

    #[test]
    fn rendering_is_cached_until_an_item_changes() {
        let mut c = many(3);
        let first = Rc::as_ptr(&c.entries[0].lines);
        c.layout(
            40,
            10,
            &ChatOpts {
                expanded: false,
                ..opts()
            },
        );
        assert_eq!(Rc::as_ptr(&c.entries[0].lines), first);
        // A patch to the last item re-renders only that one.
        c.apply_patch(ViewPatch {
            agent: None,
            session: "s".into(),
            seq: 2,
            ops: vec![set(item("a2", "agent", "changed"))],
        });
        c.layout(40, 10, &opts());
        assert_eq!(Rc::as_ptr(&c.entries[0].lines), first);
        assert_eq!(texts(&c).last().unwrap(), "changed");
        // A new width re-renders everything.
        c.layout(30, 10, &opts());
        assert_ne!(Rc::as_ptr(&c.entries[0].lines), first);
    }

    #[test]
    fn prompts_are_found_by_their_buttons() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            item("a", "agent", "hi"),
            prompt("p1", "done", &["Allow"]),
            prompt("p2", "pending", &["Allow", "Reject"]),
            prompt("p3", "pending", &["Allow"]),
        ]));
        assert_eq!(c.prompt_refs(), ["p1", "p2", "p3"]);
        assert_eq!(c.pending_prompts(), ["p2", "p3"], "oldest first");
        assert_eq!(c.buttons_of("p2").len(), 2);
        c.apply_patch(ViewPatch {
            agent: None,
            session: "s".into(),
            seq: 2,
            ops: vec![set(item("p2", "tool", "allowed"))],
        });
        assert_eq!(c.pending_prompts(), ["p3"]);
        c.apply_patch(ViewPatch {
            agent: None,
            session: "s".into(),
            seq: 3,
            ops: vec![set(item("p3", "tool", "allowed"))],
        });
        assert!(c.pending_prompts().is_empty());
        assert_eq!(c.prompt_refs(), ["p1"]);
    }

    #[test]
    fn the_active_prompt_gets_a_margin_bar_and_a_focused_button() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![prompt("p", "pending", &["Allow", "Reject"])]));
        let mut o = opts();
        o.active = Some(("p".into(), 1));
        c.layout(40, 10, &o);
        let lines = c.visible();
        assert!(plain_text(&lines).iter().any(|l| l.starts_with("▎ ")));
        let reversed = lines.iter().flat_map(|l| &l.spans).any(|s| {
            s.style
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED)
                && s.content.contains("Reject")
        });
        assert!(reversed, "{lines:?}");
    }

    fn picture(r: &str, src: &str) -> ViewItem {
        serde_json::from_value(json!({
            "ref": r, "kind": "agent",
            "component": {"type":"image","src":src,"alt":"pic"}
        }))
        .unwrap()
    }

    fn known(sizes: &[(&str, ImageStatus)]) -> Option<ChatImages> {
        Some(ChatImages {
            cell: (10, 20),
            status: Arc::new(
                sizes
                    .iter()
                    .map(|(s, st)| (s.to_string(), st.clone()))
                    .collect(),
            ),
            charts: false,
        })
    }

    fn ready(width: u32, height: u32) -> ImageStatus {
        ImageStatus::Ready { width, height }
    }

    /// Three lines of text, a 400x400 picture (24x12 cells), another line.
    fn gallery(height: u16) -> Chat {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            item("a", "agent", "before"),
            picture("p", "/tmp/p.png"),
            item("b", "agent", "after"),
        ]));
        c.set_images(known(&[("/tmp/p.png", ready(400, 400))]));
        c.layout(60, height, &opts());
        c
    }

    #[test]
    fn images_are_placeholders_until_they_are_turned_on() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![picture("p", "/tmp/p.png")]));
        c.layout(60, 20, &opts());
        assert_eq!(texts(&c), ["[image pic]"]);
        assert!(c.placements().is_empty());
        assert_eq!(c.image_sources().collect::<Vec<_>>(), ["/tmp/p.png"]);
        // On, but the size not known yet.
        c.set_images(known(&[]));
        c.layout(60, 20, &opts());
        assert_eq!(texts(&c), ["[image pic] loading image…"]);
        assert!(c.placements().is_empty());
        // Known: 12 blank rows and the caption.
        c.set_images(known(&[("/tmp/p.png", ready(400, 400))]));
        c.layout(60, 20, &opts());
        assert_eq!(c.total_lines(), 13);
        assert_eq!(texts(&c)[12], "pic · 400×400");
        // And off again.
        c.set_images(None);
        c.layout(60, 20, &opts());
        assert_eq!(texts(&c), ["[image pic]"]);
    }

    #[test]
    fn a_placement_is_the_slot_relative_to_the_top_of_the_pane() {
        let c = gallery(30);
        // "before", a blank line, then the picture.
        assert_eq!(
            c.placements(),
            [Placement {
                src: "/tmp/p.png".into(),
                x: 0,
                y: 2,
                cols: 24,
                rows: 12
            }]
        );
    }

    #[test]
    fn a_scrolled_image_is_clipped_and_leaves_when_out_of_view() {
        // 2 + 12 + 1 + 1 + 1 lines in a pane of 8: following the tail shows the last 8.
        let mut c = gallery(8);
        let at = |c: &Chat| c.placements().first().map(|p| p.y);
        assert_eq!(c.total_lines(), 17);
        assert_eq!(c.top(), 9);
        assert_eq!(at(&c), Some(2 - 9), "the top 7 rows are above the pane");
        c.scroll_up(5);
        assert_eq!(at(&c), Some(2 - 4));
        c.scroll_up(2);
        assert_eq!(at(&c), Some(0));
        c.scroll_up(1);
        assert_eq!(at(&c), Some(1));
        c.to_top();
        assert_eq!(
            at(&c),
            Some(2),
            "a row of the picture at the pane's bottom edge"
        );
        // Rows 2..14 are the picture; a pane of 2 lines at the top does not reach it.
        let mut c = gallery(2);
        c.to_top();
        assert_eq!(at(&c), None);
        // Just the picture's last row in view (top = 13, nothing below it but the caption).
        let mut c = gallery(2);
        c.scroll_up(3);
        assert_eq!(c.top(), 12);
        assert_eq!(at(&c), Some(2 - 12));
        c.scroll_down(1);
        assert_eq!(c.top(), 13);
        assert_eq!(at(&c), Some(2 - 13), "only its last row");
        c.scroll_down(1);
        assert_eq!(c.top(), 14);
        assert_eq!(at(&c), None, "the caption and the next item only");
    }

    #[test]
    fn a_wider_pane_or_ctrl_o_changes_the_size_and_a_new_status_redraws_only_pictures() {
        let mut c = gallery(30);
        let size = |c: &Chat| c.placements()[0].rows;
        assert_eq!(size(&c), 12);
        // Expanded: up to 30 rows (400x400 is 40x20 cells at most).
        let open = ChatOpts {
            expanded: true,
            ..opts()
        };
        c.layout(60, 30, &open);
        assert_eq!((c.placements()[0].cols, size(&c)), (40, 20));
        // A narrower pane: capped to the width, the aspect kept.
        c.layout(16, 30, &opts());
        assert_eq!((c.placements()[0].cols, size(&c)), (16, 8));
        // The picture's status changing leaves the text items' lines alone.
        c.layout(60, 30, &opts());
        // (Held on to, so a new allocation can't take the same address.)
        let first_again = c.entries[0].lines.clone();
        let pic = c.entries[1].lines.clone();
        c.set_images(known(&[("/tmp/p.png", ready(400, 200))]));
        c.layout(60, 30, &opts());
        assert!(Rc::ptr_eq(&c.entries[0].lines, &first_again));
        assert!(!Rc::ptr_eq(&c.entries[1].lines, &pic));
        assert_eq!((c.placements()[0].cols, size(&c)), (40, 10));
    }

    #[test]
    fn the_prompt_marker_moves_the_slot_over() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![picture("p", "/tmp/p.png")]));
        c.set_images(known(&[("/tmp/p.png", ready(400, 400))]));
        let mut o = opts();
        o.active = Some(("p".into(), 0));
        c.layout(60, 30, &o);
        assert_eq!(c.placements()[0].x, 2);
    }

    #[test]
    fn items_are_found_again_after_patches() {
        let mut c = gallery(30);
        assert_eq!(c.image_sources().count(), 1);
        c.apply_patch(ViewPatch {
            agent: None,
            session: "s".into(),
            seq: 2,
            ops: vec![set(item("p", "agent", "gone"))],
        });
        assert_eq!(c.image_sources().count(), 0);
        c.apply_patch(ViewPatch {
            agent: None,
            session: "s".into(),
            seq: 3,
            ops: vec![set(picture("q", "/tmp/q.png"))],
        });
        assert_eq!(c.image_sources().collect::<Vec<_>>(), ["/tmp/q.png"]);
    }

    fn set(item: ViewItem) -> neo_protocol::ViewOp {
        neo_protocol::ViewOp::SetViewOp(neo_protocol::SetViewOp { item })
    }
}
