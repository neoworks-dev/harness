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
