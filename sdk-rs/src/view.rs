//! The client side of the view stream: a snapshot plus patches, applied in order.

use neo_protocol::{Block, Component, ViewItem, ViewOp, ViewPage, ViewPatch, ViewSnapshot};

/// A session's view as drawn: keep one per followed session and feed it every
/// [`crate::Event::ViewSnapshot`] and [`crate::Event::ViewPatch`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    pub session: String,
    pub seq: i64,
    pub items: Vec<ViewItem>,
    /// How many items come ahead of `items`, left out of a tail snapshot; [`ViewState::prepend`]
    /// takes them in as they are paged.
    pub before: i64,
    /// The open surfaces, in the order they opened: neoui programs drawn in their `place` (a
    /// bar, a panel, a page), not in the chat. Never in `items`.
    pub surfaces: Vec<ViewItem>,
}

impl ViewState {
    pub fn from_snapshot(snapshot: ViewSnapshot) -> Self {
        ViewState {
            session: snapshot.session,
            seq: snapshot.seq,
            items: snapshot.items,
            before: snapshot.before.unwrap_or(0),
            surfaces: snapshot.surfaces.unwrap_or_default(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: ViewSnapshot) {
        *self = ViewState::from_snapshot(snapshot);
    }

    /// Older items, from [`crate::Client::view_page`], go in front of the ones held. A page for
    /// another session is ignored, and so is one of items already held.
    pub fn prepend(&mut self, page: ViewPage) {
        if page.session != self.session {
            return;
        }
        let mut items: Vec<ViewItem> = page
            .items
            .into_iter()
            .filter(|p| !self.items.iter().any(|i| i.r#ref == p.r#ref))
            .collect();
        items.append(&mut self.items);
        self.items = items;
        self.before = page.before.unwrap_or(0);
    }

    /// Applies a patch; returns the refs of the items it touched (for redrawing only those).
    /// Patches for another session are ignored.
    pub fn apply_patch(&mut self, patch: ViewPatch) -> Vec<String> {
        if patch.session != self.session {
            return Vec::new();
        }
        self.seq = patch.seq;
        let mut touched = Vec::new();
        for op in patch.ops {
            match op {
                // A surface is kept apart from the chat's items.
                ViewOp::SetViewOp(set) if set.item.place.is_some() => {
                    let item = set.item;
                    let r = item.r#ref.clone();
                    match self.surfaces.iter_mut().find(|s| s.r#ref == r) {
                        Some(existing) => *existing = item,
                        None => self.surfaces.push(item),
                    }
                    touched.push(r);
                }
                ViewOp::RemoveViewOp(op) if self.surfaces.iter().any(|s| s.r#ref == op.r#ref) => {
                    self.surfaces.retain(|s| s.r#ref != op.r#ref);
                    touched.push(op.r#ref);
                }
                ViewOp::SetViewOp(set) => {
                    let item = set.item;
                    let r = item.r#ref.clone();
                    match self.items.iter_mut().find(|i| i.r#ref == r) {
                        Some(existing) => *existing = item,
                        // An item ahead of the tail held: it comes with its page.
                        None if self.before > 0 && set.appended != Some(true) => continue,
                        None => self.items.push(item),
                    }
                    touched.push(r);
                }
                ViewOp::BlocksViewOp(op) => {
                    let Some(item) = self.items.iter_mut().find(|i| i.r#ref == op.r#ref) else {
                        continue;
                    };
                    if let Some(blocks) = text_blocks(&mut item.component, &op.path) {
                        let from = usize::try_from(op.from).unwrap_or(0).min(blocks.len());
                        blocks.truncate(from);
                        blocks.extend(op.blocks);
                        touched.push(op.r#ref);
                    }
                }
                ViewOp::RemoveViewOp(op) => {
                    self.items.retain(|i| i.r#ref != op.r#ref);
                    touched.push(op.r#ref);
                }
                ViewOp::Unknown(_) => {}
            }
        }
        touched
    }
}

/// The blocks of the text component at `path` (child indexes from `component`).
fn text_blocks<'a>(component: &'a mut Component, path: &[i64]) -> Option<&'a mut Vec<Block>> {
    let Some((first, rest)) = path.split_first() else {
        return match component {
            Component::TextComponent(text) => Some(&mut text.blocks),
            _ => None,
        };
    };
    let children = match component {
        Component::CardComponent(c) => &mut c.children,
        Component::RowComponent(c) => &mut c.children,
        Component::StackComponent(c) => &mut c.children,
        _ => return None,
    };
    let child = children.get_mut(usize::try_from(*first).ok()?)?;
    text_blocks(child, rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> ViewState {
        serde_json::from_value::<ViewSnapshot>(json!({
            "session": "s", "seq": 1, "items": [
                {"ref": "a", "kind": "agent", "component": {"type": "text", "blocks": [
                    {"type": "paragraph", "spans": [{"text": "one"}]},
                    {"type": "paragraph", "spans": [{"text": "tw"}]}
                ]}},
                {"ref": "b", "kind": "tool", "component": {"type": "card", "children": [
                    {"type": "stack", "children": [{"type": "text", "blocks": []}]}
                ]}}
            ]
        }))
        .map(ViewState::from_snapshot)
        .unwrap()
    }

    fn patch(ops: serde_json::Value) -> ViewPatch {
        serde_json::from_value(json!({"session": "s", "seq": 2, "ops": ops})).unwrap()
    }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                Block::ParagraphBlock(p) => p.spans.iter().map(|s| s.text.as_str()).collect(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn blocks_replace_the_tail_at_any_depth() {
        let mut view = state();
        let touched = view.apply_patch(patch(json!([
            {"op": "blocks", "ref": "a", "path": [], "from": 1,
             "blocks": [{"type": "paragraph", "spans": [{"text": "two"}]}]},
            {"op": "blocks", "ref": "b", "path": [0, 0], "from": 0,
             "blocks": [{"type": "paragraph", "spans": [{"text": "deep"}]}]}
        ])));
        assert_eq!(touched, ["a", "b"]);
        let Component::TextComponent(a) = &view.items[0].component else {
            panic!()
        };
        assert_eq!(texts(&a.blocks), ["one", "two"]);
        let Component::CardComponent(b) = &view.items[1].component else {
            panic!()
        };
        let Component::StackComponent(stack) = &b.children[0] else {
            panic!()
        };
        let Component::TextComponent(deep) = &stack.children[0] else {
            panic!()
        };
        assert_eq!(texts(&deep.blocks), ["deep"]);
        assert_eq!(view.seq, 2);
    }

    #[test]
    fn set_replaces_or_appends_and_remove_drops() {
        let mut view = state();
        view.apply_patch(patch(json!([
            {"op": "set", "item": {"ref": "c", "kind": "notice", "component": {"type": "text", "blocks": []}}},
            {"op": "set", "item": {"ref": "a", "kind": "agent", "component": {"type": "text", "blocks": []}}},
            {"op": "remove", "ref": "b"}
        ])));
        let refs: Vec<&str> = view.items.iter().map(|i| i.r#ref.as_str()).collect();
        assert_eq!(refs, ["a", "c"]);
    }

    #[test]
    fn a_tail_leaves_out_older_items_until_their_page_comes() {
        let mut view = state();
        view.before = 2;
        let empty = json!({"type": "text", "blocks": []});
        view.apply_patch(patch(json!([
            // An older item changing: not held, left out.
            {"op": "set", "item": {"ref": "old", "kind": "tool", "component": empty}},
            {"op": "set", "appended": true, "item": {"ref": "new", "kind": "agent", "component": empty}}
        ])));
        let refs = |v: &ViewState| v.items.iter().map(|i| i.r#ref.clone()).collect::<Vec<_>>();
        assert_eq!(refs(&view), ["a", "b", "new"]);
        let page: ViewPage = serde_json::from_value(json!({
            "session": "s", "before": 1,
            "items": [{"ref": "old", "kind": "tool", "component": empty}]
        }))
        .unwrap();
        view.prepend(page);
        assert_eq!(refs(&view), ["old", "a", "b", "new"]);
        assert_eq!(view.before, 1);
    }

    #[test]
    fn surfaces_are_kept_apart_from_the_items() {
        let mut view = state();
        let surface = |place: &str, label: &str| {
            json!({"ref": "s-bar", "kind": "tool", "place": place, "label": label,
                   "component": {"type": "text", "blocks": []}})
        };
        let touched = view.apply_patch(patch(json!([
            {"op": "set", "item": surface("bottom", "one")},
            {"op": "set", "item": surface("side", "two")}
        ])));
        assert_eq!(touched, ["s-bar", "s-bar"]);
        assert_eq!(view.items.len(), 2);
        assert_eq!(view.surfaces.len(), 1);
        assert_eq!(view.surfaces[0].label.as_deref(), Some("two"));
        view.apply_patch(patch(json!([{"op": "remove", "ref": "s-bar"}])));
        assert!(view.surfaces.is_empty());
        assert_eq!(view.items.len(), 2);
        let snapshot: ViewSnapshot = serde_json::from_value(json!({
            "session": "s", "seq": 3, "items": [], "surfaces": [surface("page", "p")]
        }))
        .unwrap();
        view.apply_snapshot(snapshot);
        assert_eq!(view.surfaces.len(), 1);
    }

    #[test]
    fn patches_for_other_sessions_are_ignored() {
        let mut view = state();
        let other: ViewPatch = serde_json::from_value(
            json!({"session": "x", "seq": 9, "ops": [{"op": "remove", "ref": "a"}]}),
        )
        .unwrap();
        assert!(view.apply_patch(other).is_empty());
        assert_eq!(view.items.len(), 2);
    }
}
