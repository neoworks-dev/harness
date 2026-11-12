import { expect, test } from "bun:test";
import type { Component, ViewItem } from "@neoworks-dev/neo-protocol/types";
import {
  activePrompt,
  argsWithNote,
  copyText,
  focusAfterFilter,
  isLongChoice,
  matchingButtons,
  moveFocus,
  openPrompts,
  plainText,
  promptEngaged,
  promptOf,
  scrollToRow,
  startButton,
  takesNote,
  TYPING_PAUSE_MS,
  type PromptButton,
} from "../ui/plugins/composer/prompt.ts";
import { withTimeLeft } from "../ui/lib/time-left.ts";

const text = (value: string): Component => ({
  type: "text",
  blocks: [{ type: "paragraph", spans: [{ text: value }] }],
});

const permission: ViewItem = {
  ref: "p-1",
  kind: "tool",
  label: "neo asks: Write a.txt",
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
            action: "neo.permit",
            args: { id: "1", option: "allow" },
            key: "1",
            note: true,
          },
          {
            type: "button",
            label: "Deny",
            action: "neo.permit",
            args: { id: "1", option: "deny" },
            key: "2",
            note: true,
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
            action: "neo.choose",
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
            action: "neo.choose",
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
  expect(prompt?.title).toBe("neo asks: Write a.txt");
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

test("a note goes along in the args of answers the daemon marks for one", () => {
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
  expect(takesNote({ label: "x", action: "neo.permit", current: false, note: true })).toBe(false);
  // A setting's choice isn't marked: no note.
  const [first] = promptOf(choice)?.buttons ?? [];
  expect(takesNote(first)).toBe(false);
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
  expect(copyText({ label: "Allow", action: "neo.permit", current: false })).toBeUndefined();
});

// ---- type to find an option of a long choice ----

/** Twelve models of three providers; the first two with a description, the third in effect. */
function models(): PromptButton[] {
  const providers = ["claude", "openrouter", "openai"];
  const names = ["Haiku 4.5", "Sonnet 4.5", "GPT 5", "Gemini 2.5"];
  return Array.from({ length: 12 }, (_, i) => {
    const provider = providers[i % 3];
    const name = names[i % 4];
    const button: PromptButton = {
      label: `${name} · ${provider}`,
      action: "neo.choose",
      args: { id: "c", option: `${provider}/${i}/${name}` },
      current: i === 2,
    };
    if (i < 2) button.description = `reasons about code ${i}`;
    return button;
  });
}

test("only a choice with more than nine options is filtered", () => {
  const all = models();
  expect(isLongChoice(all)).toBe(true);
  expect(isLongChoice(all.slice(0, 9))).toBe(false);
  expect(isLongChoice(all.map((b) => ({ ...b, action: "neo.permit" })))).toBe(false);
  expect(isLongChoice(promptOf(permission)?.buttons ?? [])).toBe(false);
});

test("every word must occur in the label, the option id or the description, in any case", () => {
  const all = models();
  const labels = (filter: string): (string | undefined)[] =>
    matchingButtons(all, filter).map((i) => all[i]?.label);
  expect(matchingButtons(all, "")).toEqual(all.map((_, i) => i));
  expect(matchingButtons(all, "   ")).toHaveLength(12);
  expect(labels("OPENROUTER")).toEqual([
    "Sonnet 4.5 · openrouter",
    "Haiku 4.5 · openrouter",
    "Gemini 2.5 · openrouter",
    "GPT 5 · openrouter",
  ]);
  expect(labels("openrouter haiku")).toEqual(["Haiku 4.5 · openrouter"]);
  // The id has what the label lacks, the description counts, the order stays.
  expect(matchingButtons(all, "openai/8")).toEqual([8]);
  expect(matchingButtons(all, "reasons")).toEqual([0, 1]);
  expect(matchingButtons(all, "  code   1 ")).toEqual([1]);
  expect(matchingButtons(all, "gemini nope")).toEqual([]);
});

test("the focus moves over the matches only and wraps", () => {
  const matches = [1, 4, 9];
  expect(moveFocus(matches, 1, 1)).toBe(4);
  expect(moveFocus(matches, 9, 1)).toBe(1);
  expect(moveFocus(matches, 1, -1)).toBe(9);
  expect(moveFocus(matches, 5, 1)).toBe(1);
  expect(moveFocus(matches, 5, -1)).toBe(9);
  expect(moveFocus([], 5, 1)).toBe(5);
});

test("a new filter focuses the first match, an empty one the current option", () => {
  const all = models();
  expect(focusAfterFilter(all, "openai")).toBe(2);
  expect(focusAfterFilter(all, "sonnet openrouter")).toBe(1);
  expect(focusAfterFilter(all, "")).toBe(2);
  expect(focusAfterFilter(all, "nope")).toBeUndefined();
});

test("the list scrolls just far enough to show the focused row", () => {
  // Rows of 28 in a window of 224 (eight rows).
  expect(scrollToRow(3, 28, 0, 224)).toBeUndefined();
  expect(scrollToRow(7, 28, 0, 224)).toBeUndefined();
  expect(scrollToRow(8, 28, 0, 224)).toBe(28);
  expect(scrollToRow(20, 28, 0, 224)).toBe(21 * 28 - 224);
  expect(scrollToRow(2, 28, 200, 224)).toBe(56);
  expect(scrollToRow(8, 28, 28, 224)).toBeUndefined();
  // Not laid out yet: nothing to do.
  expect(scrollToRow(8, 28, 0, 0)).toBeUndefined();
});

test("a button's deadline comes along, and counts down as minutes and seconds", () => {
  const item: ViewItem = {
    ref: "p-1",
    kind: "tool",
    label: "Read .env?",
    state: "pending",
    component: {
      type: "row",
      children: [
        { type: "button", label: "Allow", action: "neo.permit" },
        { type: "button", label: "Reject", action: "neo.permit", deadline: 180_000 },
      ],
    },
  };
  const buttons = promptOf(item)?.buttons ?? [];
  expect(buttons.map((b) => b.deadline)).toEqual([undefined, 180_000]);
  expect(withTimeLeft("Reject", 180_000, 0)).toBe("Reject · 3:00");
  expect(withTimeLeft("Reject", 180_000, 19_001)).toBe("Reject · 2:41");
  expect(withTimeLeft("Reject", 180_000, 200_000)).toBe("Reject · 0:00");
  expect(withTimeLeft("Allow", undefined, 0)).toBe("Allow");
});
