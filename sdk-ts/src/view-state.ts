// The client-side copy of a session's view: the latest snapshot with patches applied. Browser-safe
// (no runtime imports). Updates are copy-on-write: items already handed out are never mutated, so
// a renderer can keep them and compare by identity.
import type {
  Block,
  Component,
  ViewItem,
  ViewOp,
  ViewPatch,
  ViewSnapshot,
} from "@suite/protocol/types";

/** What one op did to the state, for renderers that update their output incrementally. */
export type ViewChange =
  | { type: "added"; item: ViewItem; index: number }
  | { type: "replaced"; item: ViewItem; index: number }
  | { type: "removed"; ref: string; index: number }
  | {
      type: "blocks";
      /** The item after the change. */
      item: ViewItem;
      index: number;
      path: number[];
      /** The text component's blocks from this index on were replaced. */
      from: number;
    };

/** The component at `path` (child indexes from `component`; [] is `component`), if there is one. */
export function componentAtPath(
  component: Component,
  path: readonly number[],
): Component | undefined {
  let node: Component | undefined = component;
  for (const index of path) {
    if (node?.type !== "card" && node?.type !== "row" && node?.type !== "stack") return undefined;
    node = node.children[index];
  }
  return node;
}

/**
 * A copy of `component` with `blocks[from:]` of the text component at `path` replaced, or
 * undefined if `path` doesn't lead to a text component.
 */
function replaceBlocks(
  component: Component,
  path: readonly number[],
  from: number,
  blocks: Block[],
): Component | undefined {
  const [head, ...rest] = path;
  if (head === undefined) {
    if (component.type !== "text") return undefined;
    return { ...component, blocks: [...component.blocks.slice(0, from), ...blocks] };
  }
  if (component.type !== "card" && component.type !== "row" && component.type !== "stack") {
    return undefined;
  }
  const child = component.children[head];
  if (!child) return undefined;
  const replaced = replaceBlocks(child, rest, from, blocks);
  if (!replaced) return undefined;
  return { ...component, children: component.children.map((c, i) => (i === head ? replaced : c)) };
}

export class ViewState {
  /** The session of the latest snapshot; undefined for state built from bare items. */
  session?: string;
  /** The seq the state is current as of (0 for state built from bare items). */
  seq = 0;
  private list: ViewItem[];

  constructor(items: ViewItem[] = []) {
    this.list = [...items];
  }

  get items(): readonly ViewItem[] {
    return this.list;
  }

  get(ref: string): ViewItem | undefined {
    return this.list.find((item) => item.ref === ref);
  }

  /** Replaces everything. */
  applySnapshot(snapshot: ViewSnapshot): void {
    this.session = snapshot.session;
    this.seq = snapshot.seq;
    this.list = [...snapshot.items];
  }

  /** Replaces the items and forgets the session and seq. */
  reset(items: ViewItem[] = []): void {
    this.session = undefined;
    this.seq = 0;
    this.list = [...items];
  }

  /**
   * Applies the ops in order and returns what changed. A patch for another session, or older than
   * the state (a straggler from before a snapshot), is ignored. Ops that don't fit the state (an
   * unknown ref, a path that isn't a text component) are skipped, not fatal.
   */
  applyPatch(patch: ViewPatch): ViewChange[] {
    if (this.session !== undefined && patch.session !== this.session) return [];
    if (patch.seq < this.seq) return [];
    this.seq = patch.seq;
    const changes: ViewChange[] = [];
    for (const op of patch.ops) {
      const change = this.applyOp(op);
      if (change) changes.push(change);
    }
    return changes;
  }

  private applyOp(op: ViewOp): ViewChange | undefined {
    switch (op.op) {
      case "set": {
        const index = this.list.findIndex((item) => item.ref === op.item.ref);
        if (index < 0) {
          this.list.push(op.item);
          return { type: "added", item: op.item, index: this.list.length - 1 };
        }
        this.list[index] = op.item;
        return { type: "replaced", item: op.item, index };
      }
      case "remove": {
        const index = this.list.findIndex((item) => item.ref === op.ref);
        if (index < 0) return undefined;
        this.list.splice(index, 1);
        return { type: "removed", ref: op.ref, index };
      }
      case "blocks": {
        const index = this.list.findIndex((item) => item.ref === op.ref);
        const item = this.list[index];
        if (!item) return undefined;
        const component = replaceBlocks(item.component, op.path, op.from, op.blocks);
        if (!component) return undefined;
        const next: ViewItem = { ...item, component };
        this.list[index] = next;
        return { type: "blocks", item: next, index, path: op.path, from: op.from };
      }
    }
  }
}
