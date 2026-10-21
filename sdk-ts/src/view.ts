// The client-side copy of a session's view: the latest snapshot with patches applied. Browser-safe
// (no runtime imports). Updates are copy-on-write: items already handed out are never mutated, so
// a renderer can keep them and compare by identity.
import type {
  Block,
  CardComponent,
  Component,
  RowComponent,
  StackComponent,
  ViewItem,
  ViewOp,
  ViewPatch,
  ViewSnapshot,
} from "@neoworks-dev/neo-protocol/types";

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

/** Whether `component` holds other components (a card, row or stack). */
export function hasChildren(
  component: Component,
): component is CardComponent | RowComponent | StackComponent {
  return component.type === "card" || component.type === "row" || component.type === "stack";
}

/** The component at `path` (child indexes from `component`; [] is `component`), if there is one. */
export function componentAtPath(
  component: Component,
  path: readonly number[],
): Component | undefined {
  let node: Component | undefined = component;
  for (const index of path) {
    if (node === undefined || !hasChildren(node)) return undefined;
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
  if (!hasChildren(component)) return undefined;
  const child = component.children[head];
  if (!child) return undefined;
  const replaced = replaceBlocks(child, rest, from, blocks);
  if (!replaced) return undefined;
  return {
    ...component,
    children: component.children.map((sibling, i) => (i === head ? replaced : sibling)),
  };
}

/**
 * The client-side copy of one view: the latest snapshot with every later patch applied. Feed it
 * from `subscribeView`, then draw `items`; `applyPatch` tells you what changed, so a renderer can
 * update its output instead of redrawing.
 */
export class ViewState {
  /** The session of the latest snapshot; undefined for state built from bare items. */
  session?: string;
  /** The agent of the latest snapshot, if it was an agent's view rather than the session's own. */
  agent?: string;
  /** The seq the state is current as of (0 for state built from bare items). */
  seq = 0;
  private current: ViewItem[];

  constructor(items: ViewItem[] = []) {
    this.current = [...items];
  }

  /** The items in display order. A change replaces an item object; it never edits one. */
  get items(): readonly ViewItem[] {
    return this.current;
  }

  /** The item with this ref, if there is one. */
  get(ref: string): ViewItem | undefined {
    return this.current.find((item) => item.ref === ref);
  }

  /** Replaces everything with the snapshot. */
  applySnapshot(snapshot: ViewSnapshot): void {
    this.session = snapshot.session;
    this.agent = snapshot.agent;
    this.seq = snapshot.seq;
    this.current = [...snapshot.items];
  }

  /** Replaces the items and forgets the session, agent and seq. */
  reset(items: ViewItem[] = []): void {
    this.session = undefined;
    this.agent = undefined;
    this.seq = 0;
    this.current = [...items];
  }

  /**
   * Applies the ops in order and returns what changed. A patch for another session or agent, or
   * older than the state (a straggler from before a snapshot), is ignored. Ops that don't fit the
   * state (an unknown ref, a path that isn't a text component) are skipped, not fatal.
   */
  applyPatch(patch: ViewPatch): ViewChange[] {
    if (this.session !== undefined && !this.isViewOf(patch)) return [];
    if (patch.seq < this.seq) return [];
    this.seq = patch.seq;
    const changes: ViewChange[] = [];
    for (const op of patch.ops) {
      const change = this.applyOp(op);
      if (change) changes.push(change);
    }
    return changes;
  }

  private isViewOf(patch: ViewPatch): boolean {
    return patch.session === this.session && patch.agent === this.agent;
  }

  private indexOf(ref: string): number {
    return this.current.findIndex((item) => item.ref === ref);
  }

  private applyOp(op: ViewOp): ViewChange | undefined {
    switch (op.op) {
      case "set": {
        const index = this.indexOf(op.item.ref);
        if (index < 0) {
          this.current.push(op.item);
          return { type: "added", item: op.item, index: this.current.length - 1 };
        }
        this.current[index] = op.item;
        return { type: "replaced", item: op.item, index };
      }
      case "remove": {
        const index = this.indexOf(op.ref);
        if (index < 0) return undefined;
        this.current.splice(index, 1);
        return { type: "removed", ref: op.ref, index };
      }
      case "blocks": {
        const index = this.indexOf(op.ref);
        const item = this.current[index];
        if (!item) return undefined;
        const component = replaceBlocks(item.component, op.path, op.from, op.blocks);
        if (!component) return undefined;
        const next: ViewItem = { ...item, component };
        this.current[index] = next;
        return { type: "blocks", item: next, index, path: op.path, from: op.from };
      }
    }
  }
}
