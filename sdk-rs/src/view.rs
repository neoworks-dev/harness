//! The client side of the view stream: a snapshot plus patches, applied in order.

use suite_protocol::{Block, Component, ViewItem, ViewOp, ViewPatch, ViewSnapshot};

/// A session's view as drawn: keep one per followed session and feed it every
/// [`crate::Event::ViewSnapshot`] and [`crate::Event::ViewPatch`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    pub session: String,
    pub seq: i64,
    pub items: Vec<ViewItem>,
}

impl ViewState {
    pub fn from_snapshot(snapshot: ViewSnapshot) -> Self {
        ViewState {
            session: snapshot.session,
            seq: snapshot.seq,
            items: snapshot.items,
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: ViewSnapshot) {
        *self = ViewState::from_snapshot(snapshot);
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
                ViewOp::SetViewOp(set) => {
                    let item = set.item;
                    touched.push(item.r#ref.clone());
                    match self.items.iter_mut().find(|i| i.r#ref == item.r#ref) {
                        Some(existing) => *existing = item,
                        None => self.items.push(item),
                    }
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
