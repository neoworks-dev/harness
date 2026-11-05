//! neoui docs (`docs/neoui.md`) as styled lines.
//!
//! The daemon evaluates the program; this draws the tree it sends. What is purely visual (the
//! selected tab, open accordion items, which target has focus, a text field's edit buffer and
//! an open Select list) comes in through [`Options::neoui`], per doc id, so the renderer stays
//! pure. Tabs and accordion items have no key: they are named by their node path in the doc.
//!
//! The pieces: `layout` (stacks, rows, cards, tabs, accordions, steps), `content` (text, code,
//! tables, metrics…), `charts`, `dev` (diffs, files, commits, output, tests, GitHub), `forms`
//! and `generic` (Box and Shape).

mod charts;
mod content;
mod dev;
mod forms;
mod generic;
mod layout;
pub(crate) use layout::columns;

pub use forms::is_date;

use std::collections::HashMap;

use neo_protocol as pb;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::Options;
use crate::render::{Cx, Lines};
use crate::text::{one_line, sline, truncate};

/// Marks the first line of the focused target until the item has found its place. Zero width.
const FOCUS_MARK: &str = "\u{2063}";

/// Purely visual state of one doc.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiState {
    /// The focused target: a key for fields and buttons, a node path for tabs and accordion
    /// items (see [`UiTarget::id`]).
    pub focus: Option<String>,
    /// The selected tab of each `Tabs`, by the target's id. Absent: the first.
    pub tabs: HashMap<String, usize>,
    /// Accordion items opened (true) or closed (false) by hand, by the target's id. Absent: the item's own
    /// `open`.
    pub open: HashMap<String, bool>,
    /// Text fields (`Input`, `TextArea`, `DatePicker`) being edited, by key: drawn instead of the
    /// node's `value`.
    pub edits: HashMap<String, UiEdit>,
    /// Values chosen here that the daemon has not echoed yet (checkbox, switch, radio, select,
    /// slider), by key: drawn instead of the node's `value`.
    pub values: HashMap<String, Value>,
    /// The `Select` whose list is open (its key), and the highlighted option.
    pub select: Option<(String, usize)>,
    /// The first slide shown of each `Carousel`, by the target's id. Absent: the first.
    pub slides: HashMap<String, usize>,
    /// The item a focused `CheckBoxGroup` or `SwitchGroup` has its cursor on, by key. Absent: the
    /// first.
    pub cursors: HashMap<String, usize>,
}

/// A text field's edit buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiEdit {
    pub text: String,
    /// The cursor, as a count of characters before it.
    pub cursor: usize,
}

/// What a target is, with what the app needs to change it.
#[derive(Debug, Clone, PartialEq)]
pub enum UiKind {
    /// `Input`; `secret` for a password.
    Input {
        secret: bool,
    },
    TextArea,
    /// A `DatePicker`: a text field that holds `YYYY-MM-DD`.
    Date,
    Select {
        options: Vec<UiOption>,
    },
    Radio {
        options: Vec<UiOption>,
    },
    CheckBox,
    Switch,
    /// Checkboxes under one name; the value is the list of the checked items' values. Up/Down
    /// move between the items, Space toggles one.
    CheckBoxGroup {
        items: Vec<UiOption>,
    },
    /// Switches under one name; the value is an object of item value to on/off.
    SwitchGroup {
        items: Vec<UiOption>,
    },
    Slider {
        min: f64,
        max: f64,
        step: f64,
    },
    Button {
        url: Option<String>,
        copy: Option<String>,
        disabled: bool,
    },
    /// A tab strip: Left/Right switch tabs.
    Tabs {
        count: usize,
    },
    /// One accordion item.
    Accordion,
    /// A carousel of `count` slides: Left/Right scroll.
    Carousel {
        count: usize,
    },
}

impl UiKind {
    /// Typing edits it.
    pub fn is_text(&self) -> bool {
        matches!(self, UiKind::Input { .. } | UiKind::TextArea | UiKind::Date)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiOption {
    pub value: String,
    pub label: String,
}

/// Something in a doc that can take focus.
#[derive(Debug, Clone, PartialEq)]
pub struct UiTarget {
    pub doc: String,
    /// The node's `key` (fields, buttons) or its path in the doc, which starts with `/` (tabs,
    /// accordion items).
    pub id: String,
    pub kind: UiKind,
    /// The text a person knows it by: the label of a button, a field's label or name.
    pub label: String,
    /// The value the daemon sent with the node (absent: null).
    pub value: Value,
    /// The path of the `Form` it sits in.
    pub form: Option<String>,
}

/// What an app has to know about a doc to decide how to treat the item that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiDocInfo {
    pub id: String,
    pub streaming: bool,
    /// Has fields or buttons (nodes with a `key`).
    pub interactive: bool,
    /// Has anything that can take focus: the above, tabs, accordions.
    pub focusable: bool,
}

/// What one render has noted about the docs it drew.
#[derive(Debug, Default)]
pub(crate) struct Run {
    doc: String,
    streaming: bool,
    form: Option<String>,
    /// Inside a `FormControl`, which draws the label.
    in_control: bool,
    /// The label of the `FormControl` the field sits in.
    pub(crate) control_label: Option<String>,
    /// Drawing only to measure: nothing is recorded.
    quiet: u32,
    pub(crate) targets: Vec<UiTarget>,
    marked: bool,
    pub(crate) focus_line: Option<usize>,
    /// Filled cards around what is being drawn: each one a shade lighter than the one it is in.
    pub(crate) cards: u32,
}

impl Run {
    /// Drawing only to measure: the lines will be thrown away.
    pub(crate) fn drawing_to_measure(&self) -> bool {
        self.quiet > 0
    }
}

/// A node of the doc: an object with `component`, optionally `key` and `partial`, and `props`.
#[derive(Clone, Copy)]
pub(crate) struct N<'a>(&'a Value);

impl<'a> N<'a> {
    pub(crate) fn new(v: &'a Value) -> Option<N<'a>> {
        v.get("component")?.as_str()?;
        Some(N(v))
    }

    pub(crate) fn name(self) -> &'a str {
        self.0
            .get("component")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(crate) fn key(self) -> Option<&'a str> {
        self.0
            .get("key")
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
    }

    pub(crate) fn prop(self, name: &str) -> Option<&'a Value> {
        self.0.get("props")?.get(name)
    }

    pub(crate) fn str(self, name: &str) -> Option<&'a str> {
        self.prop(name)?.as_str()
    }

    /// A string prop, cleaned to one line; None when absent or empty.
    pub(crate) fn line(self, name: &str) -> Option<String> {
        self.str(name).map(one_line).filter(|s| !s.is_empty())
    }

    pub(crate) fn bool(self, name: &str) -> Option<bool> {
        self.prop(name)?.as_bool()
    }

    pub(crate) fn num(self, name: &str) -> Option<f64> {
        self.prop(name)?.as_f64()
    }

    pub(crate) fn node(self, name: &str) -> Option<N<'a>> {
        N::new(self.prop(name)?)
    }

    /// The nodes of an array prop (what is not a node is left out).
    pub(crate) fn nodes(self, name: &str) -> Vec<N<'a>> {
        self.prop(name)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(N::new).collect())
            .unwrap_or_default()
    }

    pub(crate) fn tone(self) -> Option<pb::Tone> {
        tone_of(self.str("tone"))
    }
}

pub(crate) fn tone_of(s: Option<&str>) -> Option<pb::Tone> {
    match s? {
        "accent" => Some(pb::Tone::Accent),
        "muted" => Some(pb::Tone::Muted),
        "success" => Some(pb::Tone::Success),
        "danger" => Some(pb::Tone::Danger),
        _ => None,
    }
}

/// `a/b.0` for a node `seg` below the node at `path`. The root is the empty path.
pub(crate) fn join(path: &str, seg: &str) -> String {
    format!("{path}/{seg}")
}

/// The numbers the way people write them: no `.0`.
pub(crate) fn number(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else if v.is_finite() {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        "0".to_string()
    }
}

/// A scalar JSON value as text.
pub(crate) fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => one_line(s),
        Value::Number(n) => n.as_f64().map(number).unwrap_or_default(),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        _ => String::new(),
    }
}

impl<'a> Cx<'a> {
    /// One doc: its root, or the first error when there is none, or nothing.
    pub(crate) fn neoui_doc(&mut self, doc: &pb::UiDoc, w: usize) -> Lines {
        let Some(root) = &doc.root else {
            return match doc.errors.as_ref().and_then(|e| e.first()) {
                Some(err) => vec![sline(
                    truncate(&format!("[ui: {}]", one_line(&err.message)), w),
                    self.p.muted_style(),
                )],
                None => Vec::new(),
            };
        };
        let Ok(value) = serde_json::to_value(root) else {
            return Vec::new();
        };
        let Some(node) = N::new(&value) else {
            return Vec::new();
        };
        let saved = (
            std::mem::replace(&mut self.ui.doc, doc.id.clone()),
            std::mem::replace(&mut self.ui.streaming, doc.streaming == Some(true)),
            self.ui.form.take(),
            std::mem::take(&mut self.ui.in_control),
        );
        let lines = self.ui_sized(node, "", w);
        (
            self.ui.doc,
            self.ui.streaming,
            self.ui.form,
            self.ui.in_control,
        ) = saved;
        lines
    }

    /// The state of the doc being drawn.
    pub(crate) fn ui_state(&self) -> Option<&'a UiState> {
        let o: &'a Options = self.o;
        o.neoui.get(&self.ui.doc)
    }

    pub(crate) fn ui_focused(&self, id: &str) -> bool {
        !self.ui.streaming
            && self
                .ui_state()
                .is_some_and(|s| s.focus.as_deref() == Some(id))
    }

    pub(crate) fn quiet<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.ui.quiet += 1;
        let out = f(self);
        self.ui.quiet -= 1;
        out
    }

    /// Notes a target in drawing order (a streaming doc has none; measuring draws twice).
    pub(crate) fn ui_target(&mut self, id: &str, kind: UiKind, label: String, value: Value) {
        if self.ui.streaming || self.ui.quiet > 0 {
            return;
        }
        self.ui.targets.push(UiTarget {
            doc: self.ui.doc.clone(),
            id: id.to_string(),
            kind,
            label,
            value,
            form: self.ui.form.clone(),
        });
    }

    /// Puts the focus mark on the first line of the focused target.
    pub(crate) fn mark_focus(&mut self, line: &mut Line<'static>) {
        if self.ui.quiet > 0 || self.ui.marked {
            return;
        }
        self.ui.marked = true;
        line.spans.insert(
            0,
            Span::styled(FOCUS_MARK, Style::new().add_modifier(Modifier::HIDDEN)),
        );
    }

    /// Finds the focus mark, takes it out and notes its line.
    pub(crate) fn place_focus(&mut self, lines: &mut Lines) {
        for (i, line) in lines.iter_mut().enumerate() {
            if let Some(at) = line.spans.iter().position(|s| s.content == FOCUS_MARK) {
                line.spans.remove(at);
                self.ui.focus_line = Some(i);
                return;
            }
        }
    }

    /// The compact form of a doc: its title and how many lines are left out.
    pub(crate) fn neoui_summary(&mut self, doc: &pb::UiDoc, w: usize) -> (Lines, usize) {
        let mut lines = self.quiet(|cx| cx.neoui_doc(doc, w));
        let total = lines.len();
        let title = doc.root.as_ref().and_then(|root| {
            let value = serde_json::to_value(root).ok()?;
            find_title(&value)
        });
        if let (Some(title), false) = (title, lines.is_empty()) {
            lines[0] = sline(
                truncate(&title, w),
                Style::new().add_modifier(Modifier::BOLD),
            );
        }
        (lines, total)
    }
}

/// The first title in the doc: a card's, a header's, a callout's.
fn find_title(v: &Value) -> Option<String> {
    let n = N::new(v)?;
    if matches!(n.name(), "Card" | "Header" | "Callout")
        && let Some(t) = n.line("title")
    {
        return Some(t);
    }
    let props = v.get("props")?.as_object()?;
    props.values().find_map(|p| match p {
        Value::Array(items) => items.iter().find_map(find_title),
        other => find_title(other),
    })
}

/// The docs of an item (its component and the text blocks in it), in document order.
pub(crate) fn docs_of<'a>(c: &'a pb::Component, out: &mut Vec<&'a pb::UiDoc>) {
    match c {
        pb::Component::NeoUiComponent(v) => out.push(&v.ui),
        pb::Component::TextComponent(v) => docs_of_blocks(&v.blocks, out),
        pb::Component::CardComponent(v) => v.children.iter().for_each(|c| docs_of(c, out)),
        pb::Component::RowComponent(v) => v.children.iter().for_each(|c| docs_of(c, out)),
        pb::Component::StackComponent(v) => v.children.iter().for_each(|c| docs_of(c, out)),
        _ => {}
    }
}

fn docs_of_blocks<'a>(blocks: &'a [pb::Block], out: &mut Vec<&'a pb::UiDoc>) {
    for b in blocks {
        match b {
            pb::Block::NeoUiBlock(v) => out.push(&v.ui),
            pb::Block::QuoteBlock(v) => docs_of_blocks(&v.blocks, out),
            pb::Block::ListBlock(v) => v.items.iter().for_each(|i| docs_of_blocks(&i.blocks, out)),
            _ => {}
        }
    }
}

/// Counts what is in a doc's tree.
fn scan(v: &Value, interactive: &mut bool, focusable: &mut bool) {
    match v {
        Value::Object(map) => {
            if let Some(name) = map.get("component").and_then(Value::as_str) {
                if map
                    .get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|k| !k.is_empty())
                {
                    // Suggestions are reached with Ctrl-P / Ctrl-N; they don't ask for the keyboard.
                    *interactive |= name != "FollowUpItem";
                    *focusable = true;
                }
                if matches!(name, "Tabs" | "Accordion" | "Carousel") {
                    *focusable = true;
                }
            }
            map.values().for_each(|c| scan(c, interactive, focusable));
        }
        Value::Array(items) => items.iter().for_each(|c| scan(c, interactive, focusable)),
        _ => {}
    }
}

pub(crate) fn info(doc: &pb::UiDoc) -> UiDocInfo {
    let (mut interactive, mut focusable) = (false, false);
    if let Some(root) = &doc.root
        && let Ok(v) = serde_json::to_value(root)
    {
        scan(&v, &mut interactive, &mut focusable);
    }
    UiDocInfo {
        id: doc.id.clone(),
        streaming: doc.streaming == Some(true),
        interactive,
        focusable,
    }
}

/// A commit, issue or pull request a doc shows as a card: something an app can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevRef {
    Commit {
        sha: String,
        title: String,
    },
    /// An issue (`pr` false) or a pull request, with the repository and link when the doc
    /// gave them.
    Github {
        pr: bool,
        number: u64,
        repo: Option<String>,
        url: Option<String>,
    },
}

impl DevRef {
    /// What the card draws of it, to find its line by: the short sha, or `#12`.
    pub fn token(&self) -> String {
        match self {
            DevRef::Commit { sha, .. } => sha.chars().take(7).collect(),
            DevRef::Github { number, .. } => format!("#{number}"),
        }
    }
}

/// The commits, issues and pull requests the item's docs show as cards, in document order.
pub fn neoui_refs(item: &pb::ViewItem) -> Vec<DevRef> {
    let mut docs = Vec::new();
    docs_of(&item.component, &mut docs);
    let mut out = Vec::new();
    for doc in docs.into_iter().filter(|d| d.streaming != Some(true)) {
        if let Some(root) = &doc.root
            && let Ok(v) = serde_json::to_value(root)
        {
            refs_in(&v, &mut out);
        }
    }
    out
}

fn refs_in(v: &Value, out: &mut Vec<DevRef>) {
    match v {
        Value::Object(map) => {
            if let Some(n) = N::new(v) {
                let found = match n.name() {
                    "Commit" => n
                        .line("sha")
                        .filter(|s| s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit()))
                        .map(|sha| DevRef::Commit {
                            sha,
                            title: n.line("title").unwrap_or_default(),
                        }),
                    name @ ("Issue" | "PullRequest") => n
                        .num("number")
                        .filter(|x| x.is_finite() && *x >= 1.0 && x.fract() == 0.0)
                        .map(|x| DevRef::Github {
                            pr: name == "PullRequest",
                            number: x as u64,
                            repo: n.line("repo"),
                            url: n.line("url"),
                        }),
                    _ => None,
                };
                out.extend(found);
            }
            map.values().for_each(|v| refs_in(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| refs_in(v, out)),
        _ => {}
    }
}

/// The docs of an item and what each holds.
pub fn neoui_docs(item: &pb::ViewItem) -> Vec<UiDocInfo> {
    let mut docs = Vec::new();
    docs_of(&item.component, &mut docs);
    docs.into_iter().map(info).collect()
}

/// The item's focus targets in drawing order, docs in document order: what Tab and Shift-Tab
/// move through. Depends on the state: a tab that is not selected and a closed accordion item
/// show nothing, so they have no targets.
pub fn neoui_targets(item: &pb::ViewItem, opts: &Options) -> Vec<UiTarget> {
    let mut docs = Vec::new();
    docs_of(&item.component, &mut docs);
    if docs.is_empty() {
        return Vec::new();
    }
    let mut cx = Cx::new(opts);
    for doc in docs {
        cx.neoui_doc(doc, 80);
    }
    std::mem::take(&mut cx.ui.targets)
}

/// Whether the item holds a doc with fields or buttons, which keeps a tool result in full.
pub(crate) fn interactive(c: &pb::Component) -> bool {
    let mut docs = Vec::new();
    docs_of(c, &mut docs);
    docs.into_iter().any(|d| info(d).interactive)
}
