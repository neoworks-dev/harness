import { afterEach, describe, expect, test } from "bun:test";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
});

async function setup(): Promise<{ daemon: TestDaemon; session: string }> {
  const daemon = await startTestDaemon();
  cleanups.push(() => daemon.stop());
  return { daemon, session: daemon.sessions.create({}).id };
}

function textOf(result: CallToolResult): string {
  const texts: string[] = [];
  for (const c of result.content) if (c.type === "text") texts.push(c.text);
  return texts.join("");
}

describe("the shared todo list", () => {
  test("agents and the user change one list, kept in the session's log", async () => {
    const { daemon, session } = await setup();
    const tool = (args: Record<string, unknown>, guest?: string): Promise<CallToolResult> =>
      daemon.gateway.call("todos", args, undefined, guest ? { session, guest } : { session });

    expect(textOf(await tool({}))).toBe("The todo list is empty.");
    // A change says what it did to which item, and the counts.
    expect(textOf(await tool({ action: "add", text: "Make the header sticky" }))).toBe(
      "Added: t1 [ ] Make the header sticky (echo)\n1 open, 0 done.",
    );
    // The user adds one from an app, silently.
    await daemon.actions.run(
      {
        session,
        ref: "todos",
        action: "neo.todos",
        args: { action: "add", text: "Check mobile" },
      },
      "terminal",
    );
    await tool({ action: "done", id: "t1" }, "claude");
    expect(textOf(await tool({ action: "list" }))).toBe(
      "t1 [x] Make the header sticky (echo)\nt2 [ ] Check mobile (user)",
    );

    // Each change is a todos event with the whole list and who made it.
    const who: (string | undefined)[] = [];
    for (const { event } of daemon.store.readTypes(session, ["todos"])) {
      if (event.type === "todos") who.push(event.by);
    }
    expect(who).toEqual(["echo", "user", "claude"]);

    // Ids stay: removing t1 leaves t2, and the next one is t3.
    expect(textOf(await tool({ action: "remove", id: "t1" }))).toBe(
      "Removed: t1 [x] Make the header sticky (echo)\n1 open, 0 done.",
    );
    expect(textOf(await tool({ action: "add", text: "Ship it" }))).toBe(
      "Added: t3 [ ] Ship it (echo)\n2 open, 0 done.",
    );
    expect(textOf(await tool({ action: "list" }))).toBe(
      "t2 [ ] Check mobile (user)\nt3 [ ] Ship it (echo)",
    );
  });

  test("a bad change says why and changes nothing", async () => {
    const { daemon, session } = await setup();
    const call = (args: Record<string, unknown>): Promise<CallToolResult> =>
      daemon.gateway.call("todos", args, undefined, { session });
    const missing = await call({ action: "done", id: "t9" });
    expect(missing.isError).toBe(true);
    expect(textOf(missing)).toBe("todos: there is no todo t9");
    expect((await call({ action: "add" })).isError).toBe(true);
    expect((await call({ action: "shout" })).isError).toBe(true);
    expect(daemon.store.readTypes(session, ["todos"])).toEqual([]);
    expect(
      daemon.actions.run(
        { session, ref: "todos", action: "neo.todos", args: { action: "edit", id: "t1" } },
        "terminal",
      ),
    ).rejects.toThrow("edit needs id and text");
  });
});
