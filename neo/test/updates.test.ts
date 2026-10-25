import { expect, test } from "bun:test";
import type { AgentUpdate } from "../src/agent.ts";
import type { AgentEvent } from "../src/loop/index.ts";
import { createUpdateMapper } from "../src/updates.ts";

function shown(toolName: string, text: string, details: unknown): string | undefined {
  const updates: AgentUpdate[] = [];
  const map = createUpdateMapper((update) => updates.push(update));
  map({
    type: "tool_execution_end",
    toolCallId: "t1",
    toolName,
    result: { content: [{ type: "text", text }], details },
    isError: false,
  } as AgentEvent);
  const [update] = updates;
  if (update?.type !== "tool_call_update") throw new Error("no update");
  const [first] = update.content ?? [];
  return first?.type === "text" ? first.text : undefined;
}

test("a fetch shows its size and title, not the URL the header has already", () => {
  const saved = "Saved https://a.dev/x as markdown: /tmp/neo-web/s/a.dev-x.md (264 lines).";
  const details = { url: "https://a.dev/x", path: "/tmp/neo-web/s/a.dev-x.md", lines: 264 };
  expect(shown("web_fetch", `${saved}\n\nOutline (line, heading):\n3 # A`, details)).toBe(
    "264 lines · a.dev-x.md\n\nOutline (line, heading):\n3 # A",
  );
  expect(shown("web_fetch", saved, { ...details, title: "Pricing" })).toBe("264 lines · Pricing");
});

test("a search shows how many it found first", () => {
  const results = [{ title: "a" }, { title: "b" }];
  expect(shown("web_search", "1. a\n\n2. b", { kind: "web", results })).toBe(
    "2 results\n\n1. a\n\n2. b",
  );
  expect(shown("web_search", 'No results for "x".', { kind: "web", results: [] })).toBe(
    'No results for "x".',
  );
});
