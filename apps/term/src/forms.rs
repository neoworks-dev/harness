//! OpenUI docs in the chat: which target has focus, what is typed into a field, and the keys
//! that move between targets and change their values (`docs/neoui.md`, "Focus").
//!
//! The renderer draws from a [`UiState`] per doc; this keeps those states per item and turns
//! keys into changes of them. Values the user changes are queued to be sent as
//! `openui.input` (text after a pause, everything else at once); a press is the app's, which
//! sends what is queued first.

use std::collections::HashMap;

use neo_renderer_ratatui::{UiEdit, UiKind, UiOption, UiState, UiTarget, is_date};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

/// How long a typed value waits for the next key before it is sent.
pub const DEBOUNCE_MS: i64 = 150;

/// A changed value on its way to the daemon.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub item: String,
    pub doc: String,
    pub key: String,
    pub value: Value,
    pub due: i64,
}

/// What a key did that the app has to carry out.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// The key was the form's. Not taken: the app may scroll or handle a ctrl-key.
    pub taken: bool,
    /// Esc: back to the editor.
    pub leave: bool,
    /// A button to press: doc and key.
    pub press: Option<(String, String)>,
    pub url: Option<String>,
    pub copy: Option<String>,
}

/// How long a value chosen here is held for the daemon to echo it.
const HOLD_MS: i64 = 2000;

#[derive(Debug, Default)]
pub struct Forms {
    items: HashMap<String, HashMap<String, UiState>>,
    pending: Vec<Pending>,
    /// When each held value (item, doc, key) was set.
    held: HashMap<(String, String, String), i64>,
}

/// The text of a value as a field shows it.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Equal as the daemon would see them: 10 and 10.0 are the same slider position.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// A group's value with the item at `at` flipped: a list keeps the items' order, an object
/// gets every item.
fn flip(value: &Value, items: &[UiOption], at: usize, switch: bool) -> Value {
    let on = |name: &str| match value {
        Value::Array(list) => list.iter().any(|v| v.as_str() == Some(name)),
        Value::Object(map) => map.get(name).and_then(Value::as_bool).unwrap_or(false),
        _ => false,
    };
    let flipped = |i: &UiOption| on(&i.value) != (i.value == items[at].value);
    if switch {
        Value::Object(
            items
                .iter()
                .map(|i| (i.value.clone(), Value::Bool(flipped(i))))
                .collect(),
        )
    } else {
        Value::Array(
            items
                .iter()
                .filter(|i| flipped(i))
                .map(|i| json!(i.value))
                .collect(),
        )
    }
}

fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

// ---- editing a buffer; the cursor counts characters ----

fn byte_at(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(at, _)| at)
}

fn len(text: &str) -> usize {
    text.chars().count()
}

fn insert(e: &mut UiEdit, s: &str) {
    let at = byte_at(&e.text, e.cursor);
    e.text.insert_str(at, s);
    e.cursor += len(s);
}

fn backspace(e: &mut UiEdit) -> bool {
    if e.cursor == 0 {
        return false;
    }
    let (a, b) = (byte_at(&e.text, e.cursor - 1), byte_at(&e.text, e.cursor));
    e.text.replace_range(a..b, "");
    e.cursor -= 1;
    true
}

fn delete(e: &mut UiEdit) -> bool {
    if e.cursor >= len(&e.text) {
        return false;
    }
    let (a, b) = (byte_at(&e.text, e.cursor), byte_at(&e.text, e.cursor + 1));
    e.text.replace_range(a..b, "");
    true
}

/// The start of the line the cursor is on, and the end (before its newline).
fn line_bounds(e: &UiEdit) -> (usize, usize) {
    let chars: Vec<char> = e.text.chars().collect();
    let mut start = e.cursor.min(chars.len());
    while start > 0 && chars[start - 1] != '\n' {
        start -= 1;
    }
    let mut end = e.cursor.min(chars.len());
    while end < chars.len() && chars[end] != '\n' {
        end += 1;
    }
    (start, end)
}

fn kill_range(e: &mut UiEdit, from: usize, to: usize) -> bool {
    if from >= to {
        return false;
    }
    let (a, b) = (byte_at(&e.text, from), byte_at(&e.text, to));
    e.text.replace_range(a..b, "");
    e.cursor = from;
    true
}

fn word_back(e: &UiEdit) -> usize {
    let chars: Vec<char> = e.text.chars().collect();
    let mut at = e.cursor.min(chars.len());
    while at > 0 && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    while at > 0 && !chars[at - 1].is_whitespace() {
        at -= 1;
    }
    at
}

/// Moves the cursor one line up or down, to the same column when the line is long enough.
fn move_line(e: &mut UiEdit, down: bool) -> bool {
    let (start, end) = line_bounds(e);
    let col = e.cursor - start;
    let total = len(&e.text);
    if down {
        if end >= total {
            return false;
        }
        let next_start = end + 1;
        let probe = UiEdit {
            text: e.text.clone(),
            cursor: next_start,
        };
        let (_, next_end) = line_bounds(&probe);
        e.cursor = (next_start + col).min(next_end);
    } else {
        if start == 0 {
            return false;
        }
        let probe = UiEdit {
            text: e.text.clone(),
            cursor: start - 1,
        };
        let (prev_start, prev_end) = line_bounds(&probe);
        e.cursor = (prev_start + col).min(prev_end);
    }
    true
}

impl Forms {
    /// The states of an item's docs, by doc id, for the renderer.
    pub fn doc_states(&self, r: &str) -> HashMap<String, UiState> {
        self.items.get(r).cloned().unwrap_or_default()
    }

    /// All states, by item ref: what the chat draws with.
    pub fn all(&self) -> &HashMap<String, HashMap<String, UiState>> {
        &self.items
    }

    fn state(&mut self, r: &str, doc: &str) -> &mut UiState {
        self.items
            .entry(r.to_string())
            .or_default()
            .entry(doc.to_string())
            .or_default()
    }

    fn st(&self, r: &str, doc: &str) -> Option<&UiState> {
        self.items.get(r)?.get(doc)
    }

    /// The index in `targets` of the focused one.
    pub fn focus_index(&self, r: &str, targets: &[UiTarget]) -> Option<usize> {
        let docs = self.items.get(r)?;
        targets.iter().position(|t| {
            docs.get(&t.doc)
                .is_some_and(|s| s.focus.as_deref() == Some(t.id.as_str()))
        })
    }

    pub fn focused<'t>(&self, r: &str, targets: &'t [UiTarget]) -> Option<&'t UiTarget> {
        self.focus_index(r, targets).map(|i| &targets[i])
    }

    /// Gives up the focus: what was typed stays queued and goes out, and is held like any
    /// value chosen here until the daemon echoes it. Tabs and accordions keep their state.
    pub fn blur(&mut self, r: &str, now: i64) {
        if let Some(docs) = self.items.get_mut(r) {
            for (doc, st) in docs.iter_mut() {
                st.focus = None;
                st.select = None;
                st.cursors.clear();
                for (key, edit) in std::mem::take(&mut st.edits) {
                    st.values.insert(key.clone(), Value::String(edit.text));
                    self.held.insert((r.to_string(), doc.clone(), key), now);
                }
            }
        }
        for p in self.pending.iter_mut().filter(|p| p.item == r) {
            p.due = p.due.min(now);
        }
    }

    fn move_focus(&mut self, r: &str, targets: &[UiTarget], to: usize, now: i64) {
        let from = self.focus_index(r, targets);
        self.blur(r, now);
        let Some(t) = targets.get(to) else {
            return;
        };
        let text = t.kind.is_text().then(|| text_of(&self.value_of(r, t)));
        let st = self.state(r, &t.doc);
        st.focus = Some(t.id.clone());
        // Coming up from below, a group's cursor starts on its last item.
        if let UiKind::CheckBoxGroup { items } | UiKind::SwitchGroup { items } = &t.kind
            && from.is_some_and(|from| to < from)
        {
            st.cursors
                .insert(t.id.clone(), items.len().saturating_sub(1));
        }
        if let Some(text) = text {
            let cursor = len(&text);
            st.edits.insert(t.id.clone(), UiEdit { text, cursor });
        }
    }

    /// The form takes the keys: the focus stays where it was, else starts on the first target.
    pub fn enter(&mut self, r: &str, targets: &[UiTarget], now: i64) {
        match self.focus_index(r, targets) {
            Some(i) => {
                // Back from elsewhere: the field's text is the daemon's again.
                if targets[i].kind.is_text() && self.edit_of(r, &targets[i]).is_none() {
                    self.move_focus(r, targets, i, now);
                }
            }
            None => self.move_focus(r, targets, 0, now),
        }
    }

    fn edit_of(&self, r: &str, t: &UiTarget) -> Option<&UiEdit> {
        self.st(r, &t.doc)?.edits.get(&t.id)
    }

    /// The daemon sent the item again: values it has caught up with (or that it has had long
    /// enough to) are not held here any more, and a focus on something that is gone moves to the
    /// first target. The text being typed stays.
    pub fn refresh(&mut self, r: &str, targets: &[UiTarget], now: i64) {
        if !self.items.contains_key(r) {
            return;
        }
        let had_focus = self
            .items
            .get(r)
            .is_some_and(|docs| docs.values().any(|s| s.focus.is_some()));
        if had_focus && self.focus_index(r, targets).is_none() {
            self.move_focus(r, targets, 0, now);
        }
        if let Some(docs) = self.items.get_mut(r) {
            for (doc, st) in docs.iter_mut() {
                let held = &mut self.held;
                st.values.retain(|key, v| {
                    let id = (r.to_string(), doc.clone(), key.clone());
                    let echoed = targets
                        .iter()
                        .any(|t| t.doc == *doc && t.id == *key && same(&t.value, v));
                    let old = held.get(&id).is_none_or(|at| now - at > HOLD_MS);
                    if echoed || old {
                        held.remove(&id);
                    }
                    !(echoed || old)
                });
            }
        }
    }

    // ---- values on their way out ----

    fn schedule(&mut self, r: &str, t: &UiTarget, value: Value, due: i64) {
        self.pending
            .retain(|p| !(p.item == r && p.doc == t.doc && p.key == t.id));
        self.pending.push(Pending {
            item: r.to_string(),
            doc: t.doc.clone(),
            key: t.id.clone(),
            value,
            due,
        });
    }

    /// What is queued and due.
    pub fn due(&mut self, now: i64) -> Vec<Pending> {
        let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.due <= now);
        self.pending = rest;
        due
    }

    /// Everything queued for the item, due or not: right before a press.
    pub fn flush(&mut self, r: &str) -> Vec<Pending> {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.item == r);
        self.pending = rest;
        mine
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    // ---- keys ----

    /// The value of a target now: what the user chose here, else the daemon's.
    fn value_of(&self, r: &str, t: &UiTarget) -> Value {
        self.st(r, &t.doc)
            .and_then(|s| s.values.get(&t.id))
            .cloned()
            .unwrap_or_else(|| t.value.clone())
    }

    fn set_value(&mut self, r: &str, t: &UiTarget, value: Value, now: i64) {
        if same(&self.value_of(r, t), &value) {
            return;
        }
        self.state(r, &t.doc)
            .values
            .insert(t.id.clone(), value.clone());
        self.held
            .insert((r.to_string(), t.doc.clone(), t.id.clone()), now);
        self.schedule(r, t, value, now);
    }

    /// The first button of the form the target sits in (of the doc, outside a form).
    fn submit(&self, targets: &[UiTarget], t: &UiTarget) -> Option<usize> {
        targets
            .iter()
            .position(|b| {
                b.doc == t.doc
                    && b.form == t.form
                    && matches!(
                        b.kind,
                        UiKind::Button {
                            disabled: false,
                            ..
                        }
                    )
            })
            .or_else(|| {
                targets.iter().position(|b| {
                    b.doc == t.doc
                        && matches!(
                            b.kind,
                            UiKind::Button {
                                disabled: false,
                                ..
                            }
                        )
                })
            })
    }

    fn press(&self, targets: &[UiTarget], at: usize) -> Outcome {
        let b = &targets[at];
        let mut out = Outcome {
            taken: true,
            ..Outcome::default()
        };
        let UiKind::Button {
            url,
            copy,
            disabled,
        } = &b.kind
        else {
            return out;
        };
        if *disabled {
            return out;
        }
        if let Some(url) = url {
            out.url = Some(url.clone());
        } else if let Some(text) = copy {
            out.copy = Some(text.clone());
        } else {
            out.press = Some((b.doc.clone(), b.id.clone()));
        }
        out
    }

    /// A key while the form has the keyboard. `targets` is the item's, in drawing order.
    pub fn key(&mut self, r: &str, targets: &[UiTarget], key: KeyEvent, now: i64) -> Outcome {
        let mut out = Outcome::default();
        if targets.is_empty() {
            return out;
        }
        let Some(at) = self.focus_index(r, targets) else {
            self.move_focus(r, targets, 0, now);
            out.taken = true;
            return out;
        };
        let t = &targets[at];
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let n = targets.len();
        let next = |by: isize| ((at as isize + by).rem_euclid(n as isize)) as usize;
        let open = self
            .st(r, &t.doc)
            .and_then(|s| s.select.clone())
            .filter(|(k, _)| *k == t.id);

        // What every target does.
        match key.code {
            KeyCode::Esc => {
                out.taken = true;
                if open.is_some() {
                    self.state(r, &t.doc).select = None;
                } else {
                    out.leave = true;
                }
                return out;
            }
            KeyCode::Tab if !ctrl => {
                self.move_focus(r, targets, next(1), now);
                out.taken = true;
                return out;
            }
            KeyCode::BackTab => {
                self.move_focus(r, targets, next(-1), now);
                out.taken = true;
                return out;
            }
            _ => {}
        }

        if let (Some((_, highlighted)), UiKind::Select { options }) = (&open, &t.kind) {
            out.taken = true;
            let last = options.len().saturating_sub(1);
            match key.code {
                KeyCode::Up => {
                    self.state(r, &t.doc).select =
                        Some((t.id.clone(), highlighted.saturating_sub(1)))
                }
                KeyCode::Down => {
                    self.state(r, &t.doc).select = Some((t.id.clone(), (highlighted + 1).min(last)))
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(o) = options.get(*highlighted) {
                        self.set_value(r, t, json!(o.value), now);
                    }
                    self.state(r, &t.doc).select = None;
                }
                _ => out.taken = !ctrl,
            }
            return out;
        }

        match &t.kind {
            UiKind::Input { .. } | UiKind::TextArea | UiKind::Date => {
                self.text_key(r, targets, at, key, now)
            }
            UiKind::Select { options } => {
                out.taken = true;
                match key.code {
                    KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Down if !ctrl => {
                        if key.code == KeyCode::Down {
                            // Down is the way to the next field; the list opens with Enter.
                            self.move_focus(r, targets, next(1), now);
                        } else {
                            let current = text_of(&self.value_of(r, t));
                            let at = options.iter().position(|o| o.value == current).unwrap_or(0);
                            self.state(r, &t.doc).select = Some((t.id.clone(), at));
                        }
                    }
                    KeyCode::Left => self.pick(r, t, options, -1, now),
                    KeyCode::Right => self.pick(r, t, options, 1, now),
                    KeyCode::Up => self.move_focus(r, targets, next(-1), now),
                    _ => out.taken = false,
                }
                out
            }
            UiKind::Radio { options } => {
                out.taken = true;
                match key.code {
                    KeyCode::Left => self.pick(r, t, options, -1, now),
                    KeyCode::Right => self.pick(r, t, options, 1, now),
                    KeyCode::Up => self.move_focus(r, targets, next(-1), now),
                    KeyCode::Down => self.move_focus(r, targets, next(1), now),
                    KeyCode::Enter => return self.submit_out(targets, t),
                    _ => out.taken = false,
                }
                out
            }
            UiKind::CheckBox | UiKind::Switch => {
                out.taken = true;
                match key.code {
                    KeyCode::Char(' ') if !ctrl => {
                        let on = self.value_of(r, t).as_bool().unwrap_or(false);
                        self.set_value(r, t, json!(!on), now);
                    }
                    KeyCode::Up => self.move_focus(r, targets, next(-1), now),
                    KeyCode::Down => self.move_focus(r, targets, next(1), now),
                    KeyCode::Enter => return self.submit_out(targets, t),
                    _ => out.taken = false,
                }
                out
            }
            UiKind::CheckBoxGroup { items } | UiKind::SwitchGroup { items } => {
                out.taken = true;
                let switch = matches!(t.kind, UiKind::SwitchGroup { .. });
                let cur = self
                    .st(r, &t.doc)
                    .and_then(|s| s.cursors.get(&t.id).copied())
                    .unwrap_or(0)
                    .min(items.len().saturating_sub(1));
                match key.code {
                    KeyCode::Char(' ') if !ctrl && !items.is_empty() => {
                        let value = flip(&self.value_of(r, t), items, cur, switch);
                        self.set_value(r, t, value, now);
                    }
                    KeyCode::Up if cur > 0 => {
                        self.state(r, &t.doc).cursors.insert(t.id.clone(), cur - 1);
                    }
                    KeyCode::Down if cur + 1 < items.len() => {
                        self.state(r, &t.doc).cursors.insert(t.id.clone(), cur + 1);
                    }
                    KeyCode::Up => self.move_focus(r, targets, next(-1), now),
                    KeyCode::Down => self.move_focus(r, targets, next(1), now),
                    KeyCode::Enter => return self.submit_out(targets, t),
                    _ => out.taken = false,
                }
                out
            }
            UiKind::Slider { min, max, step } => {
                out.taken = true;
                let v = self.value_of(r, t).as_f64().unwrap_or(*min);
                let to = |v: f64| {
                    let snapped = ((v - min) / step).round() * step + min;
                    number(snapped.clamp(*min, *max))
                };
                match key.code {
                    KeyCode::Left => self.set_value(r, t, to(v - step), now),
                    KeyCode::Right => self.set_value(r, t, to(v + step), now),
                    KeyCode::Home => self.set_value(r, t, to(*min), now),
                    KeyCode::End => self.set_value(r, t, to(*max), now),
                    KeyCode::Up => self.move_focus(r, targets, next(-1), now),
                    KeyCode::Down => self.move_focus(r, targets, next(1), now),
                    KeyCode::Enter => return self.submit_out(targets, t),
                    _ => out.taken = false,
                }
                out
            }
            UiKind::Button { .. } => {
                match key.code {
                    KeyCode::Enter | KeyCode::Char(' ') if !ctrl && !alt => {
                        return self.press(targets, at);
                    }
                    KeyCode::Left | KeyCode::Up => self.move_focus(r, targets, next(-1), now),
                    KeyCode::Right | KeyCode::Down => self.move_focus(r, targets, next(1), now),
                    _ => return out,
                }
                out.taken = true;
                out
            }
            UiKind::Tabs { count } => {
                out.taken = true;
                let cur = self
                    .st(r, &t.doc)
                    .and_then(|s| s.tabs.get(&t.id).copied())
                    .unwrap_or(0)
                    .min(count.saturating_sub(1));
                let to = match key.code {
                    KeyCode::Left => Some(cur.saturating_sub(1)),
                    KeyCode::Right => Some((cur + 1).min(count.saturating_sub(1))),
                    KeyCode::Enter | KeyCode::Char(' ') => Some((cur + 1) % (*count).max(1)),
                    KeyCode::Up => {
                        self.move_focus(r, targets, next(-1), now);
                        None
                    }
                    KeyCode::Down => {
                        self.move_focus(r, targets, next(1), now);
                        None
                    }
                    _ => {
                        out.taken = false;
                        None
                    }
                };
                if let Some(to) = to {
                    self.state(r, &t.doc).tabs.insert(t.id.clone(), to);
                }
                out
            }
            UiKind::Carousel { count } => {
                out.taken = true;
                let last = count.saturating_sub(1);
                let cur = self
                    .st(r, &t.doc)
                    .and_then(|s| s.slides.get(&t.id).copied())
                    .unwrap_or(0)
                    .min(last);
                let to = match key.code {
                    KeyCode::Left => Some(cur.saturating_sub(1)),
                    KeyCode::Right => Some((cur + 1).min(last)),
                    KeyCode::Home => Some(0),
                    KeyCode::End => Some(last),
                    KeyCode::Enter | KeyCode::Char(' ') => Some((cur + 1) % (*count).max(1)),
                    KeyCode::Up => {
                        self.move_focus(r, targets, next(-1), now);
                        None
                    }
                    KeyCode::Down => {
                        self.move_focus(r, targets, next(1), now);
                        None
                    }
                    _ => {
                        out.taken = false;
                        None
                    }
                };
                if let Some(to) = to {
                    self.state(r, &t.doc).slides.insert(t.id.clone(), to);
                }
                out
            }
            UiKind::Accordion => {
                out.taken = true;
                let open = self
                    .st(r, &t.doc)
                    .and_then(|s| s.open.get(&t.id).copied())
                    .unwrap_or(t.value.as_bool().unwrap_or(false));
                let to = match key.code {
                    KeyCode::Enter | KeyCode::Char(' ') => Some(!open),
                    KeyCode::Right => Some(true),
                    KeyCode::Left => Some(false),
                    KeyCode::Up => {
                        self.move_focus(r, targets, next(-1), now);
                        None
                    }
                    KeyCode::Down => {
                        self.move_focus(r, targets, next(1), now);
                        None
                    }
                    _ => {
                        out.taken = false;
                        None
                    }
                };
                if let Some(to) = to {
                    self.state(r, &t.doc).open.insert(t.id.clone(), to);
                }
                out
            }
        }
    }

    fn submit_out(&self, targets: &[UiTarget], t: &UiTarget) -> Outcome {
        match self.submit(targets, t) {
            Some(b) => self.press(targets, b),
            None => Outcome {
                taken: true,
                ..Outcome::default()
            },
        }
    }

    /// Left/Right on a select or radio group: the option before or after the current one.
    fn pick(&mut self, r: &str, t: &UiTarget, options: &[UiOption], by: isize, now: i64) {
        if options.is_empty() {
            return;
        }
        let current = text_of(&self.value_of(r, t));
        let to = match options.iter().position(|o| o.value == current) {
            Some(i) => (i as isize + by).clamp(0, options.len() as isize - 1) as usize,
            None if by < 0 => options.len() - 1,
            None => 0,
        };
        if options[to].value != current {
            self.set_value(r, t, json!(options[to].value), now);
        }
    }

    fn text_key(
        &mut self,
        r: &str,
        targets: &[UiTarget],
        at: usize,
        key: KeyEvent,
        now: i64,
    ) -> Outcome {
        let t = &targets[at];
        let area = matches!(t.kind, UiKind::TextArea);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let n = targets.len();
        let next = |by: isize| ((at as isize + by).rem_euclid(n as isize)) as usize;
        let mut out = Outcome {
            taken: true,
            ..Outcome::default()
        };
        let Some(edit) = self.state(r, &t.doc).edits.get_mut(&t.id) else {
            self.move_focus(r, targets, at, now);
            return out;
        };
        let mut changed = false;
        match key.code {
            KeyCode::Char(c) if !ctrl && !alt => {
                insert(edit, c.encode_utf8(&mut [0; 4]));
                changed = true;
            }
            KeyCode::Char('a') if ctrl => edit.cursor = line_bounds(edit).0,
            KeyCode::Char('e') if ctrl => edit.cursor = line_bounds(edit).1,
            KeyCode::Char('u') if ctrl => {
                let (start, _) = line_bounds(edit);
                let to = edit.cursor;
                changed = kill_range(edit, start, to);
            }
            KeyCode::Char('k') if ctrl => {
                let (_, end) = line_bounds(edit);
                let from = edit.cursor;
                changed = kill_range(edit, from, end);
            }
            KeyCode::Char('w') if ctrl => {
                let from = word_back(edit);
                let to = edit.cursor;
                changed = kill_range(edit, from, to);
            }
            KeyCode::Char('j') if ctrl && area => {
                insert(edit, "\n");
                changed = true;
            }
            KeyCode::Char(_) if ctrl || alt => out.taken = false,
            KeyCode::Backspace => changed = backspace(edit),
            KeyCode::Delete => changed = delete(edit),
            KeyCode::Left => edit.cursor = edit.cursor.saturating_sub(1),
            KeyCode::Right => edit.cursor = (edit.cursor + 1).min(len(&edit.text)),
            KeyCode::Home => edit.cursor = line_bounds(edit).0,
            KeyCode::End => edit.cursor = line_bounds(edit).1,
            KeyCode::Enter if area => {
                insert(edit, "\n");
                changed = true;
            }
            KeyCode::Enter => return self.submit_out(targets, t),
            KeyCode::Up if area && move_line(edit, false) => {}
            KeyCode::Down if area && move_line(edit, true) => {}
            // Past the first or last line (or in a single line): on to the next field.
            KeyCode::Up => self.move_focus(r, targets, next(-1), now),
            KeyCode::Down => self.move_focus(r, targets, next(1), now),
            _ => out.taken = false,
        }
        if changed {
            self.text_changed(r, t, now);
        }
        out
    }

    /// Queues the field's text to be sent after a pause; a date only when it is one.
    fn text_changed(&mut self, r: &str, t: &UiTarget, now: i64) {
        let Some(text) = self.edit_of(r, t).map(|e| e.text.clone()) else {
            return;
        };
        if matches!(t.kind, UiKind::Date) && !text.is_empty() && !is_date(&text) {
            return;
        }
        self.schedule(r, t, Value::String(text), now + DEBOUNCE_MS);
    }

    /// Pasted text into the focused text field. False: nothing took it.
    pub fn paste(&mut self, r: &str, targets: &[UiTarget], text: &str, now: i64) -> bool {
        let Some(at) = self.focus_index(r, targets) else {
            return false;
        };
        let t = &targets[at];
        if !t.kind.is_text() {
            return false;
        }
        let area = matches!(t.kind, UiKind::TextArea);
        let clean: String = text
            .chars()
            .filter(|c| !c.is_control() || (area && *c == '\n'))
            .collect();
        let Some(edit) = self.state(r, &t.doc).edits.get_mut(&t.id) else {
            return false;
        };
        insert(edit, &clean);
        self.text_changed(r, t, now);
        true
    }

    /// The Select or text target is described for the keys line.
    pub fn is_open(&self, r: &str, t: &UiTarget) -> bool {
        self.st(r, &t.doc)
            .and_then(|s| s.select.as_ref())
            .is_some_and(|(k, _)| *k == t.id)
    }
}
