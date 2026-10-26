//! The chat pane's state: the session's view items, their rendered lines (cached), and the
//! scroll position. The terminal's own scrollback is not used; the pane scrolls itself.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use neo_protocol::{ItemKind, ItemState, ViewItem, ViewPatch, ViewSnapshot};
use neo_renderer_ratatui::{
    ButtonRef, ImageOptions, ImageSlot, ImageStatus, Options, Palette, UiDocInfo, UiState, buttons,
    image_sources, openui_docs, render_item_images,
};
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
}

#[derive(Debug, Clone, PartialEq)]
struct CacheKey {
    rev: u64,
    width: u16,
    expanded: bool,
    thoughts: bool,
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
}

struct Entry {
    r#ref: String,
    /// The agent is thinking (thoughts hidden): drawn as the animated line instead.
    thinking: bool,
    lines: Rc<Vec<Line<'static>>>,
    slots: Rc<Vec<ImageSlot>>,
    /// The line of an OpenUI target that has focus.
    focus: Option<usize>,
    offset: usize,
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
        let key = LayoutKey {
            width,
            opts: opts.clone(),
            generation: self.generation,
            images: self.images.clone(),
        };
        if self.layout_key.as_ref() == Some(&key) {
            self.apply_reveal();
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
        let mut offset = 0;
        // Tool rows follow each other without a blank line; prose and questions get one.
        let mut after_tool = false;
        let mut k = 0;
        while k < shown.len() {
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
                    if !entries.is_empty() && !after_tool {
                        offset += 1;
                    }
                    entries.push(Entry {
                        r#ref: r.clone(),
                        thinking: false,
                        lines: Rc::new(vec![folded_line(&run, usize::from(width), &opts.palette)]),
                        slots: Rc::new(Vec::new()),
                        focus: None,
                        offset,
                    });
                    offset += 1;
                    after_tool = true;
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
            let (lines, slots, focus) = match self.cache.get(r) {
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
                    )
                }
                Some(c) if c.key == ckey => (c.lines.clone(), c.slots.clone(), c.focus),
                _ => {
                    let (lines, slots, focus) = render(item, width, &ckey, self.app.as_deref());
                    let (lines, slots) = (Rc::new(lines), Rc::new(slots));
                    self.cache.insert(
                        r.clone(),
                        Cached {
                            key: ckey,
                            lines: lines.clone(),
                            slots: slots.clone(),
                            focus,
                        },
                    );
                    (lines, slots, focus)
                }
            };
            if lines.is_empty() {
                continue;
            }
            if !entries.is_empty() && !(after_tool && tool_row) {
                offset += 1;
            }
            after_tool = tool_row;
            let len = lines.len();
            entries.push(Entry {
                r#ref: r.clone(),
                thinking,
                lines,
                slots,
                focus,
                offset,
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

    /// The lines in view, the thinking line animated for the time `now_ms`.
    pub fn visible_at(&self, now_ms: Option<i64>) -> Vec<Line<'static>> {
        let top = self.top();
        let bottom = top + self.height;
        let mut out: Vec<Line<'static>> = Vec::with_capacity(self.height);
        let first = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= top);
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
}

/// A tool call that can fold into a run: finished, not a question (`p-`/`q-`, which say what was
/// answered).
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
}

/// A folded run: `15 tool calls · 7 fetch, 5 bash, 3 search · 1 failed`, or `12 bash` when all
/// are one tool; all muted but the failures.
fn folded_line(run: &[&ViewItem], width: usize, p: &Palette) -> Line<'static> {
    let mut verbs: Vec<(String, usize)> = Vec::new();
    for item in run {
        // A command's call (its label gave way to the command) counts as bash, like neo's.
        let label = match (&item.label, &item.command) {
            (Some(label), _) => label.as_str(),
            (None, Some(_)) => "bash",
            (None, None) => "tool",
        };
        let verb = label
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

/// `b` mixed into `a` by `t` (0 to 1); None unless both are true colors.
fn mix(
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
) -> (Vec<Line<'static>>, Vec<ImageSlot>, Option<usize>) {
    let opts = Options {
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
        }),
        openui: key.ui.clone().unwrap_or_default(),
        app: app.map(str::to_string),
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
        .map(|line| barred(line, key.marker))
        .collect();
    let mut slots = out.images;
    for slot in &mut slots {
        slot.x += 2;
    }
    (lines, slots, out.focus_line)
}

/// A line of the prompt being answered: the bar in the margin first.
pub fn barred(line: Line<'static>, marker: Style) -> Line<'static> {
    let mut spans = vec![Span::styled("▎ ", marker)];
    spans.extend(line.spans);
    Line::from(spans)
}

/// The prompt as the input area shows it: the item with the n-th button focused and at most
/// `preview` lines of any diff or code, each line with the bar in the margin. `hide_keys` draws
/// the buttons without their keys.
pub fn panel_lines(
    item: &ViewItem,
    width: u16,
    focused: usize,
    preview: usize,
    hide_keys: bool,
    palette: Palette,
    marker: Style,
) -> Vec<Line<'static>> {
    let opts = Options {
        expanded: false,
        show_thoughts: false,
        focused_button: Some(focused),
        hide_keys,
        prose_width: Some(PROSE_WIDTH),
        preview_lines: Some(preview),
        theme: palette,
        images: None,
        openui: HashMap::new(),
        app: None,
    };
    neo_renderer_ratatui::render_item(item, width.saturating_sub(2).max(1), &opts)
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
    fn tool_rows_follow_each_other_without_a_blank_line() {
        let mut c = Chat::new();
        c.apply_snapshot(snapshot(vec![
            call("t1", "read a", "done"),
            call("t2", "read b", "running"),
            prompt("p1", "pending", &["Allow"]),
        ]));
        c.layout(40, 20, &opts());
        let t = texts(&c);
        assert_eq!(t[..4], ["read a", "  ⎿ out", "read b", "  ⎿ out"]);
        assert_eq!(t[4], "", "a question gets room");
    }

    #[test]
    fn a_folded_command_counts_as_bash_and_only_failures_are_red() {
        let mut command: ViewItem = serde_json::from_value(json!({
            "ref": "t3", "kind": "tool", "command": "ls -la", "state": "done",
            "component": {"type":"code","code":"out"}
        }))
        .unwrap();
        let mut c = Chat::new();
        let mut items = vec![
            call("t1", "read a", "done"),
            call("t2", "read b", "done"),
            command.clone(),
            item("a1", "agent", "Done."),
        ];
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "3 tool calls · 2 read, 1 bash");
        let last = |c: &Chat| c.visible()[0].spans.last().unwrap().style.fg;
        assert_eq!(last(&c), Some(Palette::default().muted));

        command.state = Some(ItemState::Failed);
        items[2] = command;
        c.apply_snapshot(snapshot(items.clone()));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "3 tool calls · 2 read, 1 bash · 1 failed");
        assert_eq!(last(&c), Some(Palette::default().danger));

        // Calls of one tool only say how many.
        items[0] = call("t1", "bash: pwd", "done");
        items[1] = call("t2", "bash: ls", "done");
        c.apply_snapshot(snapshot(items));
        c.layout(80, 20, &opts());
        assert_eq!(texts(&c)[0], "3 bash · 1 failed");
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
            8,
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
        let first_again = Rc::as_ptr(&c.entries[0].lines);
        let pic = Rc::as_ptr(&c.entries[1].lines);
        c.set_images(known(&[("/tmp/p.png", ready(400, 200))]));
        c.layout(60, 30, &opts());
        assert_eq!(Rc::as_ptr(&c.entries[0].lines), first_again);
        assert_ne!(Rc::as_ptr(&c.entries[1].lines), pic);
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
