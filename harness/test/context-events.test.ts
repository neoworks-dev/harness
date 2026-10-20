import { expect, test } from "bun:test";
import type { ContextEvent } from "../src/agent.ts";
import { formatPrompt } from "../src/context-events.ts";
import { SYSTEM_PROMPT } from "../src/system-prompt.ts";

test("no events leaves the text unchanged", () => {
  expect(formatPrompt("hello\nworld", [])).toBe("hello\nworld");
});

test("renders connect, disconnect and focus lines before the text", () => {
  const events: ContextEvent[] = [
    {
      type: "app_connected",
      app: "lsp",
      kind: "service",
      v: 1,
      tools: ["diagnostics", "definition"],
    },
    { type: "app_disconnected", app: "photo-editor" },
    { type: "focus_changed", app: "browser", context: { url: "https://example.com/a?b=1" } },
  ];
  expect(formatPrompt("Can you help?", events)).toBe(
    [
      '<app connected="lsp" kind="service" v="1" tools="diagnostics, definition"/>',
      '<app disconnected="photo-editor"/>',
      '<focus app="browser" url="https://example.com/a?b=1"/>',
      "Can you help?",
    ].join("\n"),
  );
});

test("keeps connector events in order and emits only the last focus after them", () => {
  const events: ContextEvent[] = [
    { type: "focus_changed", app: "browser", context: { url: "https://a" } },
    { type: "app_connected", app: "code-editor", kind: "app", v: 2, tools: ["open"] },
    { type: "focus_changed", app: "code-editor", context: { project: "~/site", file: "a.ts" } },
    { type: "app_disconnected", app: "browser" },
  ];
  expect(formatPrompt("go", events).split("\n")).toEqual([
    '<app connected="code-editor" kind="app" v="2" tools="open"/>',
    '<app disconnected="browser"/>',
    '<focus app="code-editor" project="~/site" file="a.ts"/>',
    "go",
  ]);
});

test("a guest turn carries the task as an attribute and the reply as the body", () => {
  const events: ContextEvent[] = [
    {
      type: "guest_turn",
      agent: "claude",
      task: 'fix "parse"',
      result: "Fixed the off-by-one.\nTests pass.",
      stopReason: "end_turn",
    },
    { type: "focus_changed", app: "code-editor" },
  ];
  expect(formatPrompt("thanks", events).split("\n")).toEqual([
    '<guest agent="claude" task="fix &quot;parse&quot;" stop="end_turn">',
    "Fixed the off-by-one.",
    "Tests pass.",
    "</guest>",
    '<focus app="code-editor"/>',
    "thanks",
  ]);
});

test("reverted edits list the files; nothing restored says nothing", () => {
  const events: ContextEvent[] = [
    { type: "edits_reverted", paths: ["a.ts", "b.ts"] },
    { type: "edits_reverted", paths: ["c.ts"], skipped: ["d.ts"] },
    { type: "edits_reverted", paths: [], skipped: ["e.ts"] },
  ];
  expect(formatPrompt("go", events).split("\n")).toEqual([
    '<reverted paths="a.ts, b.ts"/>',
    '<reverted paths="c.ts" skipped="d.ts"/>',
    "go",
  ]);
});

test("focus without context has only the app attribute", () => {
  expect(formatPrompt("x", [{ type: "focus_changed", app: "term" }])).toBe(
    '<focus app="term"/>\nx',
  );
});

test("escapes attribute values but not the user text", () => {
  const events: ContextEvent[] = [
    { type: "focus_changed", app: "b", context: { url: 'https://x/?a=1&b="2"<3>' } },
  ];
  expect(formatPrompt("a < b & c", events)).toBe(
    '<focus app="b" url="https://x/?a=1&amp;b=&quot;2&quot;&lt;3&gt;"/>\na < b & c',
  );
});

test("the system prompt uses underscore tool names and holds no session state", () => {
  expect(SYSTEM_PROMPT).toContain("tools_search");
  expect(SYSTEM_PROMPT).toContain("tools_call");
  expect(SYSTEM_PROMPT).toContain("apps_launch");
  expect(SYSTEM_PROMPT).not.toMatch(/tools\.\w|apps\.\w/);
});

test("first connect says what the app is for; a reconnect or a tool change says only that", () => {
  const lines = formatPrompt("go", [
    {
      type: "app_connected",
      app: "photo-editor",
      kind: "app",
      v: 1,
      tools: ["open", "crop"],
      description: "Edits one image at a time.",
    },
    { type: "app_connected", app: "lsp", kind: "service", v: 1, tools: ["hover"], reconnect: true },
    {
      type: "app_connected",
      app: "browser",
      kind: "app",
      v: 3,
      tools: ["navigate", "download", "screenshot"],
      previous: 2,
      added: ["screenshot"],
      removed: ["images"],
    },
    {
      type: "app_connected",
      app: "notes",
      kind: "app",
      v: 2,
      tools: ["add"],
      previous: 1,
    },
  ]).split("\n");
  expect(lines).toEqual([
    '<app connected="photo-editor" kind="app" v="1" tools="open, crop" about="Edits one image at a time."/>',
    '<app reconnected="lsp" v="1"/>',
    '<app updated="browser" v="3" added="screenshot" removed="images"/>',
    '<app updated="notes" v="2"/>',
    "go",
  ]);
});

test("an app's UI components are in its announcement, and an update says the list again", () => {
  const lines = formatPrompt("go", [
    {
      type: "app_connected",
      app: "photo-editor",
      kind: "app",
      v: 1,
      tools: ["open", "crop"],
      components: ["BrightnessCard", "PhotoCard"],
      description: "Edits one image at a time.",
    },
    { type: "app_connected", app: "notes", kind: "app", v: 1, tools: ["add"] },
    {
      type: "app_connected",
      app: "browser",
      kind: "app",
      v: 4,
      tools: ["navigate"],
      previous: 3,
      components: ["PageCard"],
    },
    {
      type: "app_connected",
      app: "maps",
      kind: "app",
      v: 3,
      tools: ["go"],
      previous: 2,
      components: [],
    },
  ]).split("\n");
  expect(lines).toEqual([
    '<app connected="photo-editor" kind="app" v="1" tools="open, crop" components="BrightnessCard, PhotoCard" about="Edits one image at a time."/>',
    '<app connected="notes" kind="app" v="1" tools="add"/>',
    '<app updated="browser" v="4" components="PageCard"/>',
    '<app updated="maps" v="3" components=""/>',
    "go",
  ]);
});

test("every harness is told to call ui_library before its first openui block", () => {
  expect(SYSTEM_PROMPT).toContain(OPENUI_LINE);
});

const OPENUI_LINE =
  "You can show UI by writing a ```openui block. Call ui_library before your first one.";
