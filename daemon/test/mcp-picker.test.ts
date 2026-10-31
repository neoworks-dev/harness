// /mcp as a picker of the agent's own MCP servers (Claude Code's adapter answers it in markdown).
import { afterEach, expect, test } from "bun:test";
import { join } from "node:path";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { parseMcpStatus } from "../src/mcp-picker.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

test("the adapter's markdown reads back as servers with their status", () => {
  const markdown = [
    "4 MCP servers: 1 failed, 1 need authentication, 1 connected, 1 disabled",
    "**Failed**\n- `broken`: Connection closed: ECONNREFUSED",
    "**Needs authentication**\n- `posthog` (claude.ai): 2 tools",
    "**Connected**\n- `serena`: 12 tools\n- `quiet`",
    "**Disabled:** `old`, `older one`",
    "Run `/mcp reconnect <server>` to reconnect one server.",
  ].join("\n\n");
  expect(parseMcpStatus(markdown)).toEqual([
    { name: "broken", status: "failed", detail: "Connection closed: ECONNREFUSED" },
    { name: "posthog", status: "needs-auth", detail: "2 tools" },
    { name: "serena", status: "connected", detail: "12 tools" },
    { name: "quiet", status: "connected" },
    { name: "old", status: "disabled" },
    { name: "older one", status: "disabled" },
  ]);
  expect(parseMcpStatus("No MCP servers are configured.")).toEqual([]);
});

function events(daemon: TestDaemon, session: string): SessionEvent[] {
  return daemon.store.read(session).map((r) => r.event);
}

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

test("/mcp is a picker that stays open: picking a server acts on it, nothing goes to the chat", async () => {
  const daemon = await startTestDaemon({
    agent: new FakeAgent("main"),
    guests: {
      fake: {
        title: "Fake",
        command: ["bun", join(import.meta.dir, "fake-acp-agent.ts")],
        permissions: "ask",
        env: { FAKE_ACP_MCP: "1" },
      },
    },
  });
  cleanups.push(() => daemon.stop());
  const session = daemon.sessions.create({}).id;
  await daemon.commands.run(session, "/harness fake");

  await daemon.commands.run(session, "/mcp");
  const choices = (): Extract<SessionEvent, { type: "choice" }>[] =>
    events(daemon, session).filter((e) => e.type === "choice");
  await until(() => choices().length === 1, "the picker");
  const first = choices()[0];
  if (!first) throw new Error("no picker");
  expect(first.stays).toBe(true);
  expect(first.options.map((o) => o.name)).toEqual([
    "broken · failed",
    "serena · connected",
    "posthog · disabled",
  ]);

  // Space on posthog: it is enabled, the picker shows it at once as being worked on, then as it is.
  await daemon.actions.run(
    {
      session,
      ref: "x",
      action: "neo.choose",
      args: { id: first.id, option: "posthog", stay: true },
    },
    "test",
  );
  await until(
    () =>
      choices()
        .at(-1)
        ?.options.some((o) => o.name === "posthog · connected") === true,
    "posthog enabled",
  );
  expect(choices().some((c) => c.options.some((o) => o.name === "posthog · …"))).toBe(true);
  expect(choices().at(-1)?.title).toContain("posthog: connected");
  // The adapter's answers never reached the chat.
  expect(events(daemon, session).some((e) => e.type === "agent_message_chunk")).toBe(false);
  expect(events(daemon, session).some((e) => e.type === "user_message")).toBe(false);
});
