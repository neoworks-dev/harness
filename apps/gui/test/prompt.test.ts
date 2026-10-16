import { expect, test } from "bun:test";
import type { Component, ViewItem } from "@suite/protocol/types";
import {
  activePrompt,
  argsWithNote,
  copyText,
  openPrompts,
  plainText,
  promptEngaged,
  promptOf,
  startButton,
  takesNote,
  TYPING_PAUSE_MS,
} from "../ui/lib/prompt.ts";

const text = (value: string): Component => ({
  type: "text",
  blocks: [{ type: "paragraph", spans: [{ text: value }] }],
});

const permission: ViewItem = {
  ref: "p-1",
  kind: "tool",
  label: "pi asks: Write a.txt",
  state: "pending",
  component: {
    type: "stack",
    children: [
      { type: "kv", items: [{ key: "path", value: "a.txt" }] },
      {
        type: "row",
        children: [
          {
            type: "button",
            label: "Allow",
            action: "suite.permit",
            args: { id: "1", option: "allow" },
            key: "1",
          },
          {
            type: "button",
            label: "Deny",
            action: "suite.permit",
            args: { id: "1", option: "deny" },
            key: "2",
          },
        ],
      },
    ],
  },
};

const choice: ViewItem = {
  ref: "q-2",
  kind: "tool",
  label: "Model",
  state: "pending",
  component: {
    type: "stack",
    children: [
      {
        type: "row",
        children: [
          {
            type: "button",
            label: "sonnet",
            action: "suite.choose",
            args: { id: "2", option: "s" },
          },
          text("fast and cheap"),
        ],
      },
      {
        type: "row",
        children: [
          {
            type: "button",
            label: "opus",
            action: "suite.choose",
            args: { id: "2", option: "o" },
            current: true,
          },
          text("the big one"),
        ],
      },
    ],
  },
};

test("a permission card is its details and its buttons", () => {
  const prompt = promptOf(permission);
  expect(prompt?.title).toBe("pi asks: Write a.txt");
  expect(prompt?.body.map((c) => c.type)).toEqual(["kv"]);
  expect(prompt?.buttons.map((b) => b.label)).toEqual(["Allow", "Deny"]);
});

test("a choice's option text becomes the description of its button, and nothing else is body", () => {
  const prompt = promptOf(choice);
  expect(prompt?.body).toEqual([]);
  expect(prompt?.buttons.map((b) => b.description)).toEqual(["fast and cheap", "the big one"]);
  expect(startButton(prompt?.buttons ?? [])).toBe(1);
});

test("a prompt starts on its first button when none is current", () => {
  expect(startButton(promptOf(permission)?.buttons ?? [])).toBe(0);
});

test("only items with buttons that still wait are open prompts, oldest first", () => {
  const answered: ViewItem = { ...permission, ref: "p-0", state: "done" };
  const plain: ViewItem = { ref: "a", kind: "agent", component: text("hi") };
  const items = [answered, plain, permission, choice];
  expect(openPrompts(items).map((p) => p.ref)).toEqual(["p-1", "q-2"]);
});

test("Esc dismisses a prompt and the next open one takes over", () => {
  const items = [permission, choice];
  expect(activePrompt(items, new Set())?.ref).toBe("p-1");
  expect(activePrompt(items, new Set(["p-1"]))?.ref).toBe("q-2");
  expect(activePrompt(items, new Set(["p-1", "q-2"]))).toBeUndefined();
});

test("a note goes along in the args of answers that carry args", () => {
  const [allow] = promptOf(permission)?.buttons ?? [];
  if (!allow) throw new Error("the permission has no buttons");
  expect(takesNote(allow)).toBe(true);
  expect(argsWithNote(allow, "  use tabs ")).toEqual({
    id: "1",
    option: "allow",
    note: "use tabs",
  });
  expect(argsWithNote(allow, "  ")).toEqual({ id: "1", option: "allow" });
  expect(takesNote({ label: "x", action: "copy:abc", current: false })).toBe(false);
  expect(takesNote({ label: "x", action: "suite.permit", current: false })).toBe(false);
  expect(takesNote(undefined)).toBe(false);
});

test("a prompt waits for a pause in typing, then stays", () => {
  const base = { hasPrompt: true, engaged: false, typedAt: 1000 };
  expect(promptEngaged({ ...base, now: 1000 + TYPING_PAUSE_MS - 1 })).toBe(false);
  expect(promptEngaged({ ...base, now: 1000 + TYPING_PAUSE_MS })).toBe(true);
  expect(promptEngaged({ ...base, engaged: true, now: 1001 })).toBe(true);
  expect(promptEngaged({ ...base, hasPrompt: false, engaged: true, now: 9999 })).toBe(false);
});

test("text blocks flatten to lines", () => {
  expect(
    plainText([
      { type: "paragraph", spans: [{ text: "a" }, { text: "b", bold: true }] },
      {
        type: "list",
        ordered: false,
        items: [{ blocks: [{ type: "paragraph", spans: [{ text: "x" }] }] }],
      },
    ]),
  ).toBe("ab\n- x");
});

test("a copy button has text for the clipboard and nothing to send", () => {
  expect(copyText({ label: "Copy", action: "copy:asset://x", current: false })).toBe("asset://x");
  expect(copyText({ label: "Allow", action: "suite.permit", current: false })).toBeUndefined();
});
