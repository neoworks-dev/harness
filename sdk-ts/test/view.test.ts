import { describe, expect, test } from "bun:test";
import type { Block, Component, ViewItem, ViewOp, ViewPatch } from "@suite/protocol/types";
import { componentAtPath, hasChildren, ViewState } from "../src/view.ts";

function para(text: string): Block {
  return { type: "paragraph", spans: [{ text }] };
}

function agent(ref: string, ...blocks: Block[]): ViewItem {
  return { ref, kind: "agent", component: { type: "text", blocks } };
}

function patch(seq: number, ...ops: ViewPatch["ops"]): ViewPatch {
  return { session: "s1", seq, ops };
}

function nested(): ViewItem {
  const component: Component = {
    type: "card",
    children: [
      { type: "text", blocks: [para("zero")] },
      {
        type: "stack",
        children: [
          { type: "code", code: "x" },
          { type: "text", blocks: [para("a")] },
        ],
      },
    ],
  };
  return { ref: "tool", kind: "tool", component };
}

describe("ViewState", () => {
  test("a snapshot replaces everything", () => {
    const state = new ViewState([agent("old")]);
    state.applySnapshot({ session: "s1", seq: 4, items: [agent("a"), agent("b")] });
    expect(state.items.map((i) => i.ref)).toEqual(["a", "b"]);
    expect([state.session, state.seq]).toEqual(["s1", 4]);
  });

  test("set appends a new ref and replaces an existing one in place", () => {
    const state = new ViewState();
    state.applySnapshot({ session: "s1", seq: 1, items: [agent("a"), agent("b")] });
    const changes = state.applyPatch(
      patch(2, { op: "set", item: agent("c") }, { op: "set", item: agent("a", para("new")) }),
    );
    expect(changes.map((c) => [c.type, c.type === "removed" ? c.ref : c.item.ref])).toEqual([
      ["added", "c"],
      ["replaced", "a"],
    ]);
    expect(state.items.map((i) => i.ref)).toEqual(["a", "b", "c"]);
    expect(state.get("a")?.component).toEqual({ type: "text", blocks: [para("new")] });
    expect(state.seq).toBe(2);
  });

  test("remove drops the item and ignores unknown refs", () => {
    const state = new ViewState([agent("a"), agent("b")]);
    const changes = state.applyPatch(
      patch(1, { op: "remove", ref: "a" }, { op: "remove", ref: "nope" }),
    );
    expect(changes).toEqual([{ type: "removed", ref: "a", index: 0 }]);
    expect(state.items.map((i) => i.ref)).toEqual(["b"]);
  });

  test("blocks replaces the tail of a top-level text component", () => {
    const state = new ViewState([agent("a", para("one"), para("tw"))]);
    const before = state.get("a");
    const changes = state.applyPatch(
      patch(2, { op: "blocks", ref: "a", path: [], from: 1, blocks: [para("two"), para("three")] }),
    );
    expect(changes).toMatchObject([{ type: "blocks", index: 0, path: [], from: 1 }]);
    expect(state.get("a")?.component).toEqual({
      type: "text",
      blocks: [para("one"), para("two"), para("three")],
    });
    // Copy-on-write: what was handed out earlier is untouched.
    expect(before?.component).toEqual({ type: "text", blocks: [para("one"), para("tw")] });
  });

  test("blocks can also shrink the list, or start from the end", () => {
    const state = new ViewState([agent("a", para("1"), para("2"), para("3"))]);
    state.applyPatch(patch(1, { op: "blocks", ref: "a", path: [], from: 1, blocks: [] }));
    expect(state.get("a")?.component).toEqual({ type: "text", blocks: [para("1")] });
    state.applyPatch(patch(2, { op: "blocks", ref: "a", path: [], from: 1, blocks: [para("2")] }));
    expect(state.get("a")?.component).toEqual({ type: "text", blocks: [para("1"), para("2")] });
  });

  test("blocks follows a nested path through containers", () => {
    const state = new ViewState([nested()]);
    state.applyPatch(
      patch(1, { op: "blocks", ref: "tool", path: [1, 1], from: 0, blocks: [para("b")] }),
    );
    expect(state.get("tool")?.component).toEqual({
      type: "card",
      children: [
        { type: "text", blocks: [para("zero")] },
        {
          type: "stack",
          children: [
            { type: "code", code: "x" },
            { type: "text", blocks: [para("b")] },
          ],
        },
      ],
    });
    state.applyPatch(
      patch(2, { op: "blocks", ref: "tool", path: [0], from: 1, blocks: [para("more")] }),
    );
    expect(state.get("tool")?.component).toMatchObject({
      children: [{ blocks: [para("zero"), para("more")] }, { type: "stack" }],
    });
  });

  test("blocks on a bad ref or a path that is not a text component is skipped", () => {
    const state = new ViewState([nested()]);
    const bad: ViewOp[] = [
      { op: "blocks", ref: "ghost", path: [], from: 0, blocks: [] },
      { op: "blocks", ref: "tool", path: [], from: 0, blocks: [] }, // the card
      { op: "blocks", ref: "tool", path: [1, 0], from: 0, blocks: [] }, // the code component
      { op: "blocks", ref: "tool", path: [5], from: 0, blocks: [] }, // out of range
      { op: "blocks", ref: "tool", path: [0, 0], from: 0, blocks: [] }, // below a text
    ];
    expect(state.applyPatch(patch(1, ...bad))).toEqual([]);
  });

  test("ops apply in order within a patch", () => {
    const state = new ViewState();
    state.applyPatch(
      patch(
        1,
        { op: "set", item: agent("a", para("x")) },
        { op: "blocks", ref: "a", path: [], from: 0, blocks: [para("y")] },
        { op: "set", item: agent("b") },
        { op: "remove", ref: "b" },
      ),
    );
    expect(state.items).toEqual([agent("a", para("y"))]);
  });

  test("patches for another session or older than the snapshot are ignored", () => {
    const state = new ViewState();
    state.applySnapshot({ session: "s1", seq: 5, items: [agent("a")] });
    expect(state.applyPatch({ session: "s2", seq: 6, ops: [{ op: "remove", ref: "a" }] })).toEqual(
      [],
    );
    expect(state.applyPatch(patch(4, { op: "remove", ref: "a" }))).toEqual([]);
    expect(state.items).toHaveLength(1);
    expect(state.applyPatch(patch(6, { op: "remove", ref: "a" }))).toHaveLength(1);
  });

  test("state built from bare items accepts patches of any session", () => {
    const state = new ViewState([agent("a")]);
    expect(state.applyPatch(patch(1, { op: "remove", ref: "a" }))).toHaveLength(1);
  });

  test("an agent's view only takes that agent's patches, and the session's view only its own", () => {
    const own = new ViewState();
    own.applySnapshot({ session: "s1", seq: 1, items: [agent("a")] });
    const claude = new ViewState();
    claude.applySnapshot({ session: "s1", agent: "claude", seq: 1, items: [agent("a")] });

    const remove: ViewPatch = { session: "s1", seq: 2, ops: [{ op: "remove", ref: "a" }] };
    const removeForClaude: ViewPatch = { ...remove, agent: "claude" };
    expect(own.applyPatch(removeForClaude)).toEqual([]);
    expect(claude.applyPatch(remove)).toEqual([]);
    expect(claude.applyPatch(removeForClaude)).toHaveLength(1);
    expect(own.applyPatch(remove)).toHaveLength(1);
    expect(claude.agent).toBe("claude");
  });
});

describe("hasChildren and componentAtPath", () => {
  test("cards, rows and stacks have children; the rest do not", () => {
    expect(hasChildren({ type: "card", children: [] })).toBe(true);
    expect(hasChildren({ type: "row", children: [] })).toBe(true);
    expect(hasChildren({ type: "stack", children: [] })).toBe(true);
    expect(hasChildren({ type: "text", blocks: [] })).toBe(false);
    expect(hasChildren({ type: "code", code: "x" })).toBe(false);
  });

  test("componentAtPath follows child indexes and gives up on a bad path", () => {
    const { component } = nested();
    expect(componentAtPath(component, [])).toBe(component);
    expect(componentAtPath(component, [1, 0])).toEqual({ type: "code", code: "x" });
    expect(componentAtPath(component, [0, 0])).toBeUndefined();
    expect(componentAtPath(component, [5])).toBeUndefined();
  });
});
