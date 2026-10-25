//! `ChatView`: the entity a GPUI app puts in its window to show a session's view.
//!
//! It owns a `ViewState` (snapshot plus patches), the theme, one cached layout per item and the
//! local UI state (selected item, opened thoughts, the button just copied). Drawing goes through
//! GPUI's virtualized `list`, anchored to the bottom: a patch re-lays-out only the items it
//! touched and tells the list to re-measure only those, and the list keeps following the end
//! while the user hasn't scrolled away.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ListAlignment, ListOffset, ListState, MouseButton, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, list,
    prelude::FluentBuilder, px, relative, transparent_black,
};
use neo_protocol::{ItemState, Theme, ViewItem, ViewPatch, ViewSnapshot};
use neo_sdk::ViewState;
use serde_json::Value;

use crate::draw::{self, ChatFonts, DrawCx, PressFn, color};
use crate::dump::dump_items;
use crate::layout::{ButtonEffect, ItemLayout, layout_item};
use crate::theme::ChatTheme;

/// How long a copied button shows its confirmation.
const COPIED_FOR: Duration = Duration::from_millis(1500);
/// Rendered above and below the viewport so scrolling doesn't pop.
const OVERDRAW: f32 = 400.0;

/// Finds the local file of an `asset://` handle. It may block (the host asks the daemon), so
/// the chat view runs it on the background executor.
type ImageResolver = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// How long after an image's file was found its item is measured again, once GPUI has probably
/// decoded it.
const REMEASURE_AFTER: Duration = Duration::from_millis(400);

/// What the host has to react to.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// A button other than `copy:` was pressed: send `ui.action {session, ref, action, args}`,
    /// e.g. `client.action(session, &item, &action, args)`.
    Action {
        item: String,
        action: String,
        args: Option<Value>,
    },
    /// A `copy:` button put this text on the clipboard (already done; for a toast, say).
    Copied(String),
    /// The keyboard moved past the chat (Escape, or Down on the last item with buttons): give
    /// focus back to the host's input.
    Leave,
}

pub struct ChatView {
    state: ViewState,
    theme: ChatTheme,
    fonts: ChatFonts,
    /// One per `state.items`, in order; unchanged items keep their `Rc`.
    layouts: Vec<Rc<ItemLayout>>,
    list: ListState,
    focus: FocusHandle,
    /// Ref of the item the keyboard acts on (only items with buttons can be selected).
    selected: Option<String>,
    /// Thoughts the user opened.
    expanded: HashSet<String>,
    /// Item ref and button index of the last `copy:` press, until it times out.
    copied: Option<(String, usize)>,
    copy_epoch: u64,
    remote_images: bool,
    image_resolver: Option<ImageResolver>,
    /// `asset://` handles whose file is known.
    assets: Rc<HashMap<String, PathBuf>>,
    /// Handles being resolved right now (or resolved already), so each is asked once.
    asset_asked: HashSet<String>,
}

impl EventEmitter<ChatEvent> for ChatView {}

impl Focusable for ChatView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ChatView {
    pub fn new(cx: &mut Context<Self>) -> ChatView {
        ChatView {
            state: ViewState::default(),
            theme: ChatTheme::default(),
            fonts: ChatFonts::default(),
            layouts: Vec::new(),
            list: ListState::new(0, ListAlignment::Bottom, px(OVERDRAW)),
            focus: cx.focus_handle(),
            selected: None,
            expanded: HashSet::new(),
            copied: None,
            copy_epoch: 0,
            remote_images: false,
            image_resolver: None,
            assets: Rc::default(),
            asset_asked: HashSet::new(),
        }
    }

    /// Lets the chat draw `asset://…` images: `resolve` maps a handle to the stored file's path
    /// (`client.get_asset(handle)`), or `None` when it can't. It runs on the background
    /// executor, once per handle (asked again when a later patch or snapshot shows the handle
    /// after a failure); until it answers, and for handles it can't resolve, the fallback box
    /// is shown. Only `asset://` sources are ever passed to it, and nothing else is read.
    pub fn set_image_resolver(
        &mut self,
        resolve: impl Fn(&str) -> Option<PathBuf> + Send + Sync + 'static,
        cx: &mut Context<Self>,
    ) {
        self.image_resolver = Some(Arc::new(resolve));
        self.asset_asked.clear();
        self.request_assets(cx);
    }

    /// The file a resolved `asset://` handle was found at.
    pub fn asset_path(&self, handle: &str) -> Option<&Path> {
        self.assets.get(handle).map(PathBuf::as_path)
    }

    /// Starts resolving the `asset://` handles of the current items that nobody asked about yet.
    fn request_assets(&mut self, cx: &mut Context<Self>) {
        let Some(resolve) = self.image_resolver.clone() else {
            return;
        };
        let wanted: Vec<String> = self
            .layouts
            .iter()
            .flat_map(|layout| layout.asset_srcs())
            .filter(|src| !self.assets.contains_key(*src) && !self.asset_asked.contains(*src))
            .map(String::from)
            .collect();
        for handle in wanted {
            if !self.asset_asked.insert(handle.clone()) {
                continue;
            }
            let resolve = resolve.clone();
            cx.spawn(async move |this, cx| {
                let asked = handle.clone();
                let path = cx
                    .background_executor()
                    .spawn(async move { resolve(&asked) })
                    .await;
                let found = this
                    .update(cx, |this, cx| {
                        let Some(path) = path else {
                            // Ask again the next time the handle shows up.
                            this.asset_asked.remove(&handle);
                            return false;
                        };
                        Rc::make_mut(&mut this.assets).insert(handle.clone(), path);
                        this.remeasure(&handle);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if found {
                    // The picture decodes after the first draw, so measure the item again.
                    cx.background_executor().timer(REMEASURE_AFTER).await;
                    this.update(cx, |this, cx| {
                        this.remeasure(&handle);
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
    }

    /// Tells the list that items showing `handle` may have a different height now.
    fn remeasure(&mut self, handle: &str) {
        for ix in 0..self.layouts.len() {
            if self.layouts[ix].asset_srcs().contains(&handle) {
                self.list.splice(ix..ix + 1, 1);
            }
        }
    }

    /// The font families to draw with (the host's embedded font, say).
    pub fn set_fonts(&mut self, fonts: ChatFonts, cx: &mut Context<Self>) {
        self.fonts = fonts;
        cx.notify();
    }

    /// Whether http(s) images are drawn. GPUI fetches them with the app's HTTP client, and apps
    /// start without one (`Application::with_http_client`), so this is off until the host says
    /// otherwise; the fallback box is shown meanwhile.
    pub fn set_remote_images(&mut self, on: bool, cx: &mut Context<Self>) {
        self.remote_images = on;
        cx.notify();
    }

    /// The daemon sends the theme after connecting and when it changes; until then the default
    /// theme (`protocol/themes/default.json`) is used.
    pub fn set_theme(&mut self, theme: &Theme, cx: &mut Context<Self>) {
        let theme = ChatTheme::new(theme);
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        self.sync(None);
        cx.notify();
    }

    /// Replaces everything. Also the answer to a reconnect: items that did not change keep their
    /// layout, so scroll position and selection survive it.
    pub fn apply_snapshot(&mut self, snapshot: ViewSnapshot, cx: &mut Context<Self>) {
        if snapshot.session != self.state.session {
            // Another conversation: nothing carries over.
            self.layouts.clear();
            self.list.reset(0);
            self.selected = None;
            self.expanded.clear();
            self.copied = None;
        }
        self.state.apply_snapshot(snapshot);
        self.sync(None);
        self.request_assets(cx);
        cx.notify();
    }

    /// Applies one `view.patch`: only the items it touches are laid out again. Patches for another
    /// session, or older than what the snapshot already had, are ignored.
    pub fn apply_patch(&mut self, patch: ViewPatch, cx: &mut Context<Self>) {
        if patch.session != self.state.session || patch.seq < self.state.seq {
            return;
        }
        let touched: HashSet<String> = self.state.apply_patch(patch).into_iter().collect();
        self.sync(Some(&touched));
        self.request_assets(cx);
        cx.notify();
    }

    /// Forgets the conversation (the host stopped following the session).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.state = ViewState::default();
        self.layouts.clear();
        self.list.reset(0);
        self.selected = None;
        self.expanded.clear();
        self.copied = None;
        self.asset_asked.clear();
        cx.notify();
    }

    pub fn session(&self) -> &str {
        &self.state.session
    }

    pub fn seq(&self) -> i64 {
        self.state.seq
    }

    pub fn items(&self) -> &[ViewItem] {
        &self.state.items
    }

    /// The layout model of each item, in order.
    pub fn layouts(&self) -> &[Rc<ItemLayout>] {
        &self.layouts
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The model as text, the same format as the fixture snapshots.
    pub fn dump(&self) -> String {
        let layouts: Vec<ItemLayout> = self.layouts.iter().map(|l| (**l).clone()).collect();
        dump_items(&layouts, &self.theme)
    }

    /// Whether the agent seems to be working: a tool is pending or running, or the last thing in
    /// the chat is the user's own message.
    pub fn busy(&self) -> bool {
        self.state
            .items
            .iter()
            .any(|i| matches!(i.state, Some(ItemState::Pending | ItemState::Running)))
            || self
                .state
                .items
                .last()
                .is_some_and(|i| i.kind == neo_protocol::ItemKind::User)
    }

    /// Brings `layouts` (and the list's measurements) in line with `state.items`. `touched` names
    /// the items whose content may have changed; `None` means any of them.
    fn sync(&mut self, touched: Option<&HashSet<String>>) {
        let old = std::mem::take(&mut self.layouts);
        let mut by_ref = std::collections::HashMap::with_capacity(old.len());
        for layout in &old {
            by_ref.insert(layout.r#ref.as_str(), layout);
        }
        let fresh: Vec<Rc<ItemLayout>> = self
            .state
            .items
            .iter()
            .map(|item| {
                let previous = by_ref.get(item.r#ref.as_str());
                if let Some(previous) = previous
                    && touched.is_some_and(|t| !t.contains(&item.r#ref))
                {
                    return Rc::clone(previous);
                }
                let layout = layout_item(item, &self.theme);
                match previous {
                    Some(previous) if ***previous == layout => Rc::clone(previous),
                    _ => Rc::new(layout),
                }
            })
            .collect();

        // Tell the list about the one contiguous span that differs.
        let same = |a: &Rc<ItemLayout>, b: &Rc<ItemLayout>| Rc::ptr_eq(a, b);
        let prefix = old
            .iter()
            .zip(&fresh)
            .take_while(|(a, b)| same(a, b))
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(fresh[prefix..].iter().rev())
            .take_while(|(a, b)| same(a, b))
            .count();
        if prefix + suffix < old.len() || prefix + suffix < fresh.len() {
            self.list
                .splice(prefix..old.len() - suffix, fresh.len() - prefix - suffix);
        }
        drop(by_ref);
        self.layouts = fresh;

        let selectable = |r: &str| self.layouts.iter().any(|l| l.r#ref == r && l.has_buttons());
        if self.selected.as_deref().is_some_and(|r| !selectable(r)) {
            self.selected = None;
        }
        if self.copied.as_ref().is_some_and(|(r, i)| {
            !self
                .layouts
                .iter()
                .any(|l| l.r#ref == *r && *i < l.buttons.len())
        }) {
            self.copied = None;
        }
        self.expanded
            .retain(|r| self.state.items.iter().any(|i| i.r#ref == *r));
    }

    fn index_of(&self, item_ref: &str) -> Option<usize> {
        self.layouts.iter().position(|l| l.r#ref == item_ref)
    }

    // --- keyboard and buttons ---

    /// Selects an item with buttons (Up/Down, a click). Returns whether it could be selected.
    pub fn select(&mut self, item_ref: &str, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.index_of(item_ref) else {
            return false;
        };
        if !self.layouts[ix].has_buttons() {
            return false;
        }
        self.selected = Some(item_ref.to_string());
        self.list.scroll_to_reveal_item(ix);
        cx.notify();
        true
    }

    /// Moves the selection to the last item with buttons (what an input line does on Up) and
    /// focuses the chat. False when no item has buttons.
    pub fn focus_last_actionable(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let last = self
            .layouts
            .iter()
            .rev()
            .find(|l| l.has_buttons())
            .map(|l| l.r#ref.clone());
        match last {
            Some(r) if self.select(&r, cx) => {
                window.focus(&self.focus);
                true
            }
            _ => false,
        }
    }

    /// Up (`-1`) or Down (`1`) among the items with buttons. Down past the last one leaves the chat.
    pub fn move_selection(&mut self, step: i32, cx: &mut Context<Self>) {
        let actionable: Vec<&str> = self
            .layouts
            .iter()
            .filter(|l| l.has_buttons())
            .map(|l| l.r#ref.as_str())
            .collect();
        if actionable.is_empty() {
            return;
        }
        let current = self
            .selected
            .as_deref()
            .and_then(|s| actionable.iter().position(|r| *r == s));
        let target = match (current, step.signum()) {
            (None, 1) => 0,
            (None, _) => actionable.len() - 1,
            (Some(i), 1) if i + 1 == actionable.len() => {
                cx.emit(ChatEvent::Leave);
                return;
            }
            (Some(i), 1) => i + 1,
            (Some(i), _) => i.saturating_sub(1),
        };
        let target = actionable[target].to_string();
        self.select(&target, cx);
    }

    /// A key reached the chat. `key` is the typed character, or the key's name (`up`, `down`,
    /// `escape`). Returns whether the chat used it.
    pub fn handle_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "up" => self.move_selection(-1, cx),
            "down" => self.move_selection(1, cx),
            "escape" => cx.emit(ChatEvent::Leave),
            _ => {
                let Some(selected) = self.selected.clone() else {
                    return false;
                };
                let Some(ix) = self.index_of(&selected) else {
                    return false;
                };
                let Some(button) = self.layouts[ix].button_for_key(key) else {
                    return false;
                };
                self.press_button(&selected, button, cx);
            }
        }
        true
    }

    /// Presses button `index` of item `item_ref`: `copy:` goes to the clipboard, anything else
    /// to the host as [`ChatEvent::Action`].
    pub fn press_button(&mut self, item_ref: &str, index: usize, cx: &mut Context<Self>) {
        let Some(spec) = self
            .layouts
            .iter()
            .find(|l| l.r#ref == item_ref)
            .and_then(|l| l.buttons.get(index))
        else {
            return;
        };
        match spec.effect() {
            ButtonEffect::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.copied = Some((item_ref.to_string(), index));
                self.copy_epoch += 1;
                let epoch = self.copy_epoch;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(COPIED_FOR).await;
                    this.update(cx, |this, cx| {
                        if this.copy_epoch == epoch {
                            this.copied = None;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
                cx.emit(ChatEvent::Copied(text));
            }
            ButtonEffect::Action { action, args } => cx.emit(ChatEvent::Action {
                item: item_ref.to_string(),
                action,
                args,
            }),
        }
        cx.notify();
    }

    fn toggle_thought(&mut self, item_ref: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.index_of(item_ref) else {
            return;
        };
        if !self.expanded.remove(item_ref) {
            self.expanded.insert(item_ref.to_string());
        }
        // Its height changed.
        self.list.splice(ix..ix + 1, 1);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return;
        }
        let key = keystroke.key_char.as_deref().unwrap_or(&keystroke.key);
        if self.handle_key(key, cx) {
            cx.stop_propagation();
        }
    }

    /// Scrolls to the newest item and follows the end again.
    pub fn scroll_to_bottom(&self) {
        self.list.scroll_to(ListOffset {
            item_ix: self.layouts.len(),
            offset_in_item: px(0.),
        });
    }

    // --- drawing ---

    fn render_item(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(layout) = self.layouts.get(ix).cloned() else {
            return div().into_any_element();
        };
        let theme = self.theme.clone();
        let item_ref = layout.r#ref.clone();
        let selected = self.selected.as_deref() == Some(item_ref.as_str());
        let ringed = selected && self.focus.is_focused(window);
        let opened = self.expanded.contains(&item_ref);
        let copied = self
            .copied
            .as_ref()
            .filter(|(r, _)| *r == item_ref)
            .map(|(_, i)| *i);

        let press: PressFn = {
            let item_ref = item_ref.clone();
            Rc::new(
                cx.processor(move |this: &mut ChatView, index: usize, _, cx| {
                    this.select(&item_ref, cx);
                    this.press_button(&item_ref, index, cx);
                }),
            )
        };
        let dcx = DrawCx::new(self.fonts.clone(), press, copied, self.remote_images)
            .with_assets(self.assets.clone());

        let frame = &layout.frame;
        let mut framed = div()
            .flex()
            .flex_col()
            .gap(px(theme.s))
            .min_w_0()
            .pt(px(frame.pad.top))
            .pr(px(frame.pad.right))
            .pb(px(frame.pad.bottom))
            .pl(px(frame.pad.left))
            .rounded(px(frame.radius.max(6.0)))
            .border_1()
            .border_color(if ringed {
                color(theme.accent)
            } else {
                frame.border.map_or_else(transparent_black, color)
            });
        framed = match frame.max_width {
            Some(max) => framed.max_w(relative(max)),
            None => framed.w_full(),
        };
        if let Some(bg) = frame.bg {
            framed = framed.bg(color(bg));
        }

        if let Some(header) = &layout.header {
            let mut row = div().flex().flex_row().items_center().gap(px(theme.s));
            if let Some(icon) = &header.icon {
                let icon = draw::node(&crate::layout::Node::Text(icon.clone()), &dcx);
                row = row.child(if layout.state == Some(ItemState::Running) {
                    draw::pulse(icon)
                } else {
                    icon
                });
            }
            row = row.child(draw::node(
                &crate::layout::Node::Text(header.label.clone()),
                &dcx,
            ));
            if layout.collapsible {
                let toggled = item_ref.clone();
                framed = framed.child(
                    row.id(SharedString::from(format!("{item_ref}/header")))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_thought(&toggled, cx);
                        })),
                );
            } else {
                framed = framed.child(row);
            }
        }
        if !layout.collapsible || opened {
            framed = framed.child(
                div()
                    .ml(px(frame.body_indent))
                    .min_w_0()
                    .child(draw::node(&layout.body, &dcx)),
            );
        }

        let mut outer = div()
            .id(SharedString::from(item_ref.clone()))
            .w_full()
            .pb(px(theme.m))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .when(frame.align_end, |row| row.justify_end())
                    .child(framed),
            );
        if layout.has_buttons() {
            outer = outer.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if this.select(&item_ref, cx) {
                        window.focus(&this.focus);
                    }
                }),
            );
        }
        outer.into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("neo-chat")
            .key_context("NeoChat")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .bg(color(self.theme.bg))
            .text_color(color(self.theme.fg))
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, ix, window, cx| this.render_item(ix, window, cx)),
                )
                .size_full()
                .p(px(self.theme.l)),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;

    use gpui::{AppContext as _, Entity, TestAppContext};
    use serde_json::json;

    use super::*;

    fn snapshot(session: &str, seq: i64, items: Value) -> ViewSnapshot {
        serde_json::from_value(json!({"session": session, "seq": seq, "items": items})).unwrap()
    }

    fn patch(session: &str, seq: i64, ops: Value) -> ViewPatch {
        serde_json::from_value(json!({"session": session, "seq": seq, "ops": ops})).unwrap()
    }

    fn paragraph(text: &str) -> Value {
        json!({"type": "paragraph", "spans": [{"text": text}]})
    }

    fn text_item(item_ref: &str, kind: &str, text: &str) -> Value {
        json!({"ref": item_ref, "kind": kind,
            "component": {"type": "text", "blocks": [paragraph(text)]}})
    }

    fn buttons_item(item_ref: &str) -> Value {
        json!({"ref": item_ref, "kind": "tool", "label": "x", "state": "done", "component": {
        "type": "card", "title": "Result", "children": [
            {"type": "button", "label": "Copy", "action": "copy:abc"},
            {"type": "button", "label": "Open", "action": "app.open", "args": {"id": 7}, "key": "o"}
        ]}})
    }

    /// A view plus everything it emitted.
    fn chat(cx: &mut TestAppContext) -> (Entity<ChatView>, Rc<RefCell<Vec<ChatEvent>>>) {
        let view = cx.new(ChatView::new);
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, event: &ChatEvent, _| {
                sink.borrow_mut().push(event.clone());
            })
            .detach();
        });
        (view, events)
    }

    fn refs(view: &Entity<ChatView>, cx: &mut TestAppContext) -> Vec<String> {
        view.read_with(cx, |v, _| {
            v.layouts().iter().map(|l| l.r#ref.clone()).collect()
        })
    }

    #[gpui::test]
    fn patches_relayout_only_what_they_touch(cx: &mut TestAppContext) {
        let (view, _) = chat(cx);
        view.update(cx, |v, cx| {
            v.apply_snapshot(
                snapshot(
                    "s",
                    1,
                    json!([
                        text_item("a", "agent", "one"),
                        text_item("b", "agent", "two"),
                        text_item("c", "agent", "three")
                    ]),
                ),
                cx,
            )
        });
        let before = view.read_with(cx, |v, _| v.layouts().to_vec());
        assert_eq!(view.read_with(cx, |v, _| v.list.item_count()), 3);

        // set (replace), blocks (streaming tail) on another item.
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    2,
                    json!([
                        {"op": "set", "item": text_item("a", "agent", "ONE")},
                        {"op": "blocks", "ref": "c", "path": [], "from": 1,
                         "blocks": [paragraph("more")]}
                    ]),
                ),
                cx,
            )
        });
        let after = view.read_with(cx, |v, _| v.layouts().to_vec());
        assert!(!Rc::ptr_eq(&before[0], &after[0]), "a was replaced");
        assert!(Rc::ptr_eq(&before[1], &after[1]), "b was not touched");
        assert!(!Rc::ptr_eq(&before[2], &after[2]), "c grew");
        let dump = view.read_with(cx, |v, _| v.dump());
        assert!(
            dump.contains("\"ONE\"") && dump.contains("\"more\""),
            "{dump}"
        );

        // A set with identical content keeps the layout.
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    3,
                    json!([{"op": "set", "item": text_item("b", "agent", "two")}]),
                ),
                cx,
            )
        });
        let same = view.read_with(cx, |v, _| v.layouts().to_vec());
        assert!(Rc::ptr_eq(&after[1], &same[1]));

        // Append (set of a new ref) and remove.
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    4,
                    json!([
                        {"op": "set", "item": text_item("d", "notice", "new")},
                        {"op": "remove", "ref": "a"}
                    ]),
                ),
                cx,
            )
        });
        assert_eq!(refs(&view, cx), ["b", "c", "d"]);
        assert_eq!(view.read_with(cx, |v, _| v.list.item_count()), 3);
        let seq = view.read_with(cx, |v, _| v.seq());
        assert_eq!(seq, 4);
    }

    #[gpui::test]
    fn nested_text_paths_are_patched(cx: &mut TestAppContext) {
        let (view, _) = chat(cx);
        let item = json!({"ref": "t", "kind": "tool", "component": {"type": "card", "children": [
            {"type": "stack", "children": [{"type": "text", "blocks": []}]}]}});
        view.update(cx, |v, cx| {
            v.apply_snapshot(snapshot("s", 1, json!([item])), cx)
        });
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    2,
                    json!([{"op": "blocks", "ref": "t", "path": [0, 0], "from": 0,
                            "blocks": [paragraph("deep")]}]),
                ),
                cx,
            )
        });
        assert!(view.read_with(cx, |v, _| v.dump()).contains("\"deep\""));
    }

    #[gpui::test]
    fn stale_and_foreign_patches_are_ignored(cx: &mut TestAppContext) {
        let (view, _) = chat(cx);
        view.update(cx, |v, cx| {
            v.apply_snapshot(
                snapshot("s", 5, json!([text_item("a", "agent", "one")])),
                cx,
            )
        });
        let remove =
            |session: &str, seq| patch(session, seq, json!([{"op": "remove", "ref": "a"}]));
        view.update(cx, |v, cx| v.apply_patch(remove("other", 9), cx));
        view.update(cx, |v, cx| v.apply_patch(remove("s", 4), cx));
        assert_eq!(refs(&view, cx), ["a"]);
        view.update(cx, |v, cx| v.apply_patch(remove("s", 6), cx));
        assert!(refs(&view, cx).is_empty());
    }

    #[gpui::test]
    fn a_new_snapshot_keeps_unchanged_layouts(cx: &mut TestAppContext) {
        let (view, _) = chat(cx);
        let items = json!([text_item("a", "agent", "one"), buttons_item("b")]);
        view.update(cx, |v, cx| {
            v.apply_snapshot(snapshot("s", 1, items.clone()), cx);
            assert!(v.select("b", cx));
        });
        let before = view.read_with(cx, |v, _| v.layouts().to_vec());
        // What a reconnect sends: the same session, a later seq.
        view.update(cx, |v, cx| v.apply_snapshot(snapshot("s", 9, items), cx));
        let after = view.read_with(cx, |v, _| v.layouts().to_vec());
        assert!(Rc::ptr_eq(&before[0], &after[0]) && Rc::ptr_eq(&before[1], &after[1]));
        assert_eq!(
            view.read_with(cx, |v, _| v.selected().map(String::from)),
            Some("b".into())
        );

        // Another session starts fresh.
        view.update(cx, |v, cx| {
            v.apply_snapshot(snapshot("t", 1, json!([text_item("z", "agent", "hi")])), cx)
        });
        assert_eq!(refs(&view, cx), ["z"]);
        assert_eq!(
            view.read_with(cx, |v, _| v.selected().map(String::from)),
            None
        );
    }

    #[gpui::test]
    fn keys_trigger_buttons_of_the_selected_item(cx: &mut TestAppContext) {
        let (view, events) = chat(cx);
        view.update(cx, |v, cx| {
            v.apply_snapshot(
                snapshot(
                    "s",
                    1,
                    json!([
                        buttons_item("b1"),
                        text_item("t", "agent", "x"),
                        buttons_item("b2")
                    ]),
                ),
                cx,
            )
        });
        let key = |cx: &mut TestAppContext, k: &str| view.update(cx, |v, cx| v.handle_key(k, cx));

        // Nothing selected: button keys do nothing.
        assert!(!key(cx, "o"));
        assert!(events.borrow().is_empty());

        // Down selects the first item with buttons, Down again the next (the text is skipped).
        assert!(key(cx, "down"));
        assert_eq!(
            view.read_with(cx, |v, _| v.selected().map(String::from)),
            Some("b1".into())
        );
        assert!(key(cx, "down"));
        assert_eq!(
            view.read_with(cx, |v, _| v.selected().map(String::from)),
            Some("b2".into())
        );
        assert!(key(cx, "up"));
        assert_eq!(
            view.read_with(cx, |v, _| v.selected().map(String::from)),
            Some("b1".into())
        );

        // "o" is the second button's own key; "2" would be its position, but it has a key.
        assert!(key(cx, "o"));
        assert!(!key(cx, "2"));
        // The first button has no key, so its position triggers it: copy, handled locally.
        assert!(key(cx, "1"));
        assert_eq!(
            *events.borrow(),
            [
                ChatEvent::Action {
                    item: "b1".into(),
                    action: "app.open".into(),
                    args: Some(json!({"id": 7})),
                },
                ChatEvent::Copied("abc".into()),
            ]
        );
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("abc".into())
        );

        // Past the last item the keyboard leaves the chat; Escape does too.
        events.borrow_mut().clear();
        key(cx, "down");
        key(cx, "down");
        key(cx, "escape");
        assert_eq!(*events.borrow(), [ChatEvent::Leave, ChatEvent::Leave]);
    }

    #[gpui::test]
    fn copy_never_reaches_the_host_as_an_action(cx: &mut TestAppContext) {
        let (view, events) = chat(cx);
        view.update(cx, |v, cx| {
            v.apply_snapshot(snapshot("s", 1, json!([buttons_item("b")])), cx);
            v.press_button("b", 0, cx);
            v.press_button("b", 1, cx);
            v.press_button("b", 5, cx); // no such button
            v.press_button("nope", 0, cx);
        });
        let events = events.borrow();
        assert!(matches!(events[0], ChatEvent::Copied(_)));
        assert!(matches!(events[1], ChatEvent::Action { .. }));
        assert_eq!(events.len(), 2);
    }

    #[gpui::test]
    fn thoughts_open_and_survive_patches(cx: &mut TestAppContext) {
        let (view, _) = chat(cx);
        view.update(cx, |v, cx| {
            v.apply_snapshot(
                snapshot("s", 1, json!([text_item("t", "thought", "hm")])),
                cx,
            );
            v.toggle_thought("t", cx);
        });
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    2,
                    json!([{"op": "set", "item": text_item("t", "thought", "hmm")}]),
                ),
                cx,
            )
        });
        assert!(view.read_with(cx, |v, _| v.expanded.contains("t")));
        view.update(cx, |v, cx| v.toggle_thought("t", cx));
        assert!(!view.read_with(cx, |v, _| v.expanded.contains("t")));
    }

    #[gpui::test]
    fn theme_changes_relayout_everything(cx: &mut TestAppContext) {
        let (view, _) = chat(cx);
        view.update(cx, |v, cx| {
            v.apply_snapshot(
                snapshot("s", 1, json!([text_item("a", "agent", "one")])),
                cx,
            )
        });
        let mut theme = default_theme_for_tests();
        theme.colors.fg = "#ffffff".into();
        view.update(cx, |v, cx| v.set_theme(&theme, cx));
        assert_eq!(
            view.read_with(cx, |v, _| v.layouts()[0].body.clone()),
            view.read_with(cx, |v, _| layout_item(&v.state.items[0], &v.theme).body)
        );
        assert!(view.read_with(cx, |v, _| v.theme.fg.rgb == 0xffffff));
    }

    fn default_theme_for_tests() -> Theme {
        crate::theme::default_theme()
    }

    #[gpui::test]
    fn draws_every_fixture(cx: &mut TestAppContext) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let mut drawn = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let file = entry.path().join("resolved.json");
            let Ok(data) = std::fs::read_to_string(&file) else {
                continue;
            };
            let items: Value = serde_json::from_str(&data).unwrap();
            let window = cx.add_window(|_, cx| ChatView::new(cx));
            let view = window.root(cx).unwrap();
            view.update(cx, |v, cx| {
                v.apply_snapshot(snapshot("s", 1, items), cx);
                // Selected items draw their ring.
                v.move_selection(1, cx);
            });
            for _ in 0..2 {
                cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                    .unwrap();
            }
            assert!(
                view.read_with(cx, |v, _| v.list.viewport_bounds().size.width > px(0.)),
                "{}: the list was not laid out",
                file.display()
            );
            drawn += 1;
        }
        assert!(drawn >= 8);
    }

    #[gpui::test]
    fn draws_data_images_and_the_fallback(cx: &mut TestAppContext) {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let items = json!([{"ref": "i", "kind": "agent", "component": {"type": "stack", "children": [
            {"type": "image", "src": format!("data:image/png;base64,{PNG}"),
             "width": 1, "height": 1, "caption": "pixel"},
            {"type": "image", "src": "https://example.com/a.png"},
            {"type": "image", "src": "asset://img-1", "width": 4, "height": 3}
        ]}}]);
        let window = cx.add_window(|_, cx| ChatView::new(cx));
        let view = window.root(cx).unwrap();
        for remote in [false, true] {
            view.update(cx, |v, cx| {
                v.set_remote_images(remote, cx);
                v.apply_snapshot(snapshot("s", 1, items.clone()), cx);
            });
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
        }
        let dump = view.read_with(cx, |v, _| v.dump());
        assert!(dump.contains("source=data ImageData(Png"), "{dump}");
        assert!(dump.contains("[image 4×3 · asset://img-1]"), "{dump}");
    }

    #[gpui::test]
    fn follows_the_end_only_while_at_the_bottom(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| ChatView::new(cx));
        let view = window.root(cx).unwrap();
        let many: Vec<Value> = (0..200)
            .map(|i| text_item(&format!("m{i}"), "agent", &format!("message {i}")))
            .collect();
        view.update(cx, |v, cx| {
            v.apply_snapshot(snapshot("s", 1, json!(many)), cx)
        });
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
        };
        let append = |cx: &mut TestAppContext, seq: i64, name: &str| {
            view.update(cx, |v, cx| {
                v.apply_patch(
                    patch(
                        "s",
                        seq,
                        json!([{"op": "set", "item": text_item(name, "agent", "latest")}]),
                    ),
                    cx,
                )
            });
            draw(cx);
        };
        // `item_ix == count` is the list's "pinned to the end" state.
        let pinned = |cx: &mut TestAppContext| {
            view.read_with(cx, |v, _| {
                v.list.logical_scroll_top().item_ix == v.layouts.len()
            })
        };
        draw(cx);
        assert!(pinned(cx), "starts at the end");
        append(cx, 2, "n1");
        assert!(pinned(cx), "stays at the end when an item arrives");

        // Streaming into the last item keeps following too.
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    3,
                    json!([{"op": "blocks", "ref": "n1", "path": [], "from": 1,
                            "blocks": [paragraph("tail")]}]),
                ),
                cx,
            )
        });
        draw(cx);
        assert!(pinned(cx));

        // Scrolled away: new items must not pull the view down.
        view.update(cx, |v, _| v.list.scroll_by(px(-2000.)));
        draw(cx);
        assert!(!pinned(cx));
        append(cx, 4, "n2");
        assert!(!pinned(cx), "the user's position is kept");

        view.update(cx, |v, _| v.scroll_to_bottom());
        draw(cx);
        assert!(pinned(cx));
    }

    fn image_item(item_ref: &str, src: &str) -> Value {
        json!({"ref": item_ref, "kind": "tool", "label": "x", "state": "done", "component": {
            "type": "card", "children": [{"type": "image", "src": src, "width": 40, "height": 20}]}})
    }

    #[gpui::test]
    fn asset_images_are_resolved_off_thread_and_only_for_asset_handles(cx: &mut TestAppContext) {
        use std::sync::Mutex;

        let (view, _) = chat(cx);
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = asked.clone();
        view.update(cx, |v, cx| {
            v.set_image_resolver(
                move |handle| {
                    log.lock().unwrap().push(handle.to_string());
                    (handle == "asset://img-1").then(|| PathBuf::from("/store/img-1.png"))
                },
                cx,
            )
        });
        let items = json!([
            image_item("a", "asset://img-1"),
            image_item("b", "asset://img-2"),
            image_item("c", "https://example.com/x.png"),
            image_item("d", "/etc/passwd"),
            image_item("e", "file:///etc/passwd"),
            image_item("f", "asset://"),
        ]);
        let before = view.update(cx, |v, cx| {
            v.apply_snapshot(snapshot("s", 1, items), cx);
            v.dump()
        });
        cx.run_until_parked();

        view.read_with(cx, |v, _| {
            assert_eq!(
                v.asset_path("asset://img-1"),
                Some(Path::new("/store/img-1.png"))
            );
            assert_eq!(v.asset_path("asset://img-2"), None, "unresolvable");
            // The layout model (and so the snapshots) doesn't depend on the resolver.
            assert_eq!(v.dump(), before);
        });
        let mut seen = asked.lock().unwrap().clone();
        seen.sort();
        assert_eq!(
            seen,
            ["asset://img-1", "asset://img-2"],
            "nothing else is asked"
        );

        // Cached: a patch asks again only about the one that failed.
        view.update(cx, |v, cx| {
            v.apply_patch(
                patch(
                    "s",
                    2,
                    json!([{"op": "set", "item": text_item("g", "agent", "more")}]),
                ),
                cx,
            )
        });
        cx.run_until_parked();
        let mut seen = asked.lock().unwrap().clone();
        seen.sort();
        assert_eq!(
            seen,
            ["asset://img-1", "asset://img-2", "asset://img-2"],
            "img-1 is cached, img-2 is retried"
        );
    }

    #[gpui::test]
    fn asset_images_draw_as_fallbacks_and_then_as_pictures(cx: &mut TestAppContext) {
        let png = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"); // any readable file
        let window = cx.add_window(|_, cx| ChatView::new(cx));
        let view = window.root(cx).unwrap();
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
        };
        view.update(cx, |v, cx| {
            v.apply_snapshot(
                snapshot("s", 1, json!([image_item("a", "asset://img-1")])),
                cx,
            )
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.asset_path("asset://img-1").is_none()));
        let dump = view.read_with(cx, |v, _| v.dump());
        assert!(dump.contains("source=fallback"), "{dump}");
        draw(cx);

        view.update(cx, |v, cx| {
            v.set_image_resolver(move |_| Some(png.clone()), cx)
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.asset_path("asset://img-1").is_some()));
        draw(cx);
    }
}
