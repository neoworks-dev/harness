import { afterEach, expect, test } from "bun:test";
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { validateEnvelope } from "@neoworks-dev/neo-protocol/validate";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { resolvePaths } from "../src/paths.ts";
import { SessionStore } from "../src/store.ts";
import { oneLine } from "../src/summaries.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function daemon(): Promise<TestDaemon> {
  const d = await startTestDaemon();
  cleanups.push(() => d.stop());
  return d;
}

test("oneLine takes the first non-empty line, collapsed and cut", () => {
  expect(oneLine("\n\n  hello   world \nmore", 80)).toBe("hello world");
  expect(oneLine("abcdefghij", 5)).toBe("abcd…");
  expect(oneLine("  \n ", 5)).toBeUndefined();
});

test("session info carries a title from the first message, a preview and lines changed", async () => {
  const d = await daemon();
  const id = d.sessions.create({}).id;
  d.store.append(id, { type: "user_message", app: "cli", text: "\nFix the header\nplease" });
  d.store.append(id, { type: "agent_message_chunk", messageId: "m1", text: "Done: the " });
  d.store.append(id, { type: "agent_message_chunk", messageId: "m1", text: "header sticks." });
  const diff = { type: "diff" as const, path: "/s/a.ts", oldText: "a\nb\n", newText: "a\nc\nd\n" };
  d.store.append(id, { type: "tool_call_update", toolCallId: "t1", content: [diff] });
  // An update repeating the same diff doesn't count twice.
  d.store.append(id, {
    type: "tool_call_update",
    toolCallId: "t1",
    status: "completed",
    content: [diff],
  });
  expect(d.sessions.info(id)).toMatchObject({
    title: "Fix the header",
    preview: "Done: the header sticks.",
    added: 2,
    removed: 1,
  });
  // A second message doesn't rename it; the preview follows the newest message.
  d.store.append(id, { type: "user_message", app: "cli", text: "and the footer" });
  expect(d.sessions.info(id)).toMatchObject({ title: "Fix the header", preview: "and the footer" });
});

test("summaries rebuild from the log", async () => {
  const d = await daemon();
  const id = d.sessions.create({ title: "Given" }).id;
  d.store.append(id, { type: "user_message", app: "cli", text: "hi" });
  await d.stop();
  cleanups.pop();
  const again = await startTestDaemon({ dir: d.dir });
  cleanups.push(() => again.stop());
  expect(again.sessions.info(id)).toMatchObject({ title: "Given", preview: "hi", added: 0 });
});

test("clients get session.changed when a session runs, ends and changes", async () => {
  const d = await daemon();
  const changes: SessionInfo[] = [];
  const handle = d.hub.connect({
    send(frame) {
      const result = validateEnvelope(JSON.parse(frame));
      if (!result.ok) throw new Error(`invalid frame: ${result.error}`);
      if (result.value.topic === "session.changed") changes.push(result.value.payload);
    },
  });
  handle.receive(JSON.stringify({ topic: "bus.hello", payload: { protocol: 1, client: "test" } }));
  const id = d.sessions.create({}).id;
  d.sessions.input(id, "cli", "hello there");
  await d.sessions.idle(id);
  for (let i = 0; i < 100 && changes.at(-1)?.state !== "idle"; i++) await Bun.sleep(20);
  expect(changes.at(-1)).toMatchObject({ id, state: "idle", title: "hello there" });
  expect(changes.at(-1)?.preview).toContain("hello there");
});

test("sessions from before titles get one from their first message", async () => {
  const d = await daemon();
  await d.stop();
  cleanups.pop();
  // A log written without the summaries (as an older daemon did).
  const store = new SessionStore(resolvePaths(d.env).db);
  const id = store.createSession({ id: "s_old", cwd: d.dir }).id;
  store.append(id, { type: "user_message", app: "cli", text: "Old question" });
  store.append(id, { type: "user_message", app: "cli", text: "later" });
  store.close();
  const again = await startTestDaemon({ dir: d.dir });
  cleanups.push(() => again.stop());
  expect(again.sessions.info(id)).toMatchObject({ title: "Old question", preview: "later" });
});
