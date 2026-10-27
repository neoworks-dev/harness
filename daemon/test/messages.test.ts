// The messenger on its own: who a message can go to, what is logged and queued, the limits, and
// how wait blocks. (daemon/test/messaging.test.ts runs it through real subagents.)
import { describe, expect, test } from "bun:test";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import { INBOX_LIMIT, Messenger, PAIR_LIMIT, TEXT_LIMIT } from "../src/messages.ts";

function setup(): { messenger: Messenger; log: SessionEvent[]; woken: string[] } {
  const log: SessionEvent[] = [];
  const woken: string[] = [];
  const messenger = new Messenger({
    append: (_session, event) => {
      log.push(event);
    },
    wake: (session) => {
      woken.push(session);
    },
  });
  return { messenger, log, woken };
}

const text = (result: CallToolResult): string => (result.content[0] as { text: string }).text;
const S = "s_1";

/** A background claude d_1 and a neo d_2 in session S. */
function twoSubagents(): ReturnType<typeof setup> {
  const world = setup();
  world.messenger.join(S, { id: "d_1", agent: "claude" }, true);
  world.messenger.join(S, { id: "d_2", agent: "neo" }, true);
  return world;
}

describe("sending", () => {
  test("by id, by harness name and to main; logged and queued formatted for the recipient", () => {
    const { messenger, log } = twoSubagents();
    expect(text(messenger.send(S, undefined, { to: "d_1", text: "look at x" }))).toBe(
      "Sent to d_1 (claude).",
    );
    expect(text(messenger.send(S, undefined, { to: "NEO", text: "hello" }))).toBe(
      "Sent to d_2 (neo).",
    );
    expect(text(messenger.send(S, "claude", { to: "main", text: "a question" }))).toBe(
      "Sent to main.",
    );
    expect(log).toEqual([
      { type: "agent_message", from: "main", to: "d_1", toAgent: "claude", text: "look at x" },
      { type: "agent_message", from: "main", to: "d_2", toAgent: "neo", text: "hello" },
      {
        type: "agent_message",
        from: "d_1",
        to: "main",
        fromAgent: "claude",
        text: "a question",
      },
    ]);
    expect(messenger.takeOf(S, "d_1")).toEqual(['<message from="main">\nlook at x\n</message>']);
    expect(messenger.takeOf(S, "d_1")).toEqual([]);
    expect(messenger.take(S, undefined)).toEqual([
      '<message from="d_1" agent="claude">\na question\n</message>',
    ]);
  });

  test("a subagent reaches its peers; the recipient is told who", () => {
    const { messenger } = twoSubagents();
    messenger.send(S, "neo", { to: "claude", text: "your tests are red" });
    expect(messenger.takeOf(S, "d_1")).toEqual([
      '<message from="d_2" agent="neo">\nyour tests are red\n</message>',
    ]);
  });

  test("wakes the session's agent for main and the child session for a subagent in a directory", () => {
    const { messenger, woken } = setup();
    messenger.join(S, { id: "d_1", agent: "claude", child: "s_child" }, true);
    messenger.join(S, { id: "d_2", agent: "neo" }, true);
    messenger.send(S, "neo", { to: "main", text: "hi" });
    messenger.send(S, undefined, { to: "d_1", text: "hi" });
    messenger.send(S, undefined, { to: "d_2", text: "hi" });
    expect(woken).toEqual([S, "s_child"]);
  });

  test("refuses what can't be delivered, in words the sender can act on", () => {
    const { messenger } = twoSubagents();
    messenger.join(S, { id: "d_3", agent: "claude" }, true);
    const refusals = [
      [messenger.send(S, undefined, { to: "d_9", text: "x" }), 'No running subagent "d_9"'],
      [messenger.send(S, undefined, { to: "claude", text: "x" }), "2 claude subagents run"],
      [messenger.send(S, undefined, { to: "main", text: "x" }), "can't message yourself"],
      [messenger.send(S, "claude", { to: "d_1", text: "x" }), "can't message yourself"],
      [messenger.send(S, undefined, { to: "d_1", text: "  " }), "text is empty"],
      [messenger.send(S, undefined, { to: "d_1", text: "x".repeat(TEXT_LIMIT + 1) }), "too long"],
    ] as const;
    for (const [result, words] of refusals) {
      expect(result.isError).toBe(true);
      expect(text(result)).toContain(words);
    }
    // A caller that is no party at all: the user's own /claude turn, for one.
    expect(messenger.send(S, "codex", { to: "main", text: "x" })).toMatchObject({ isError: true });
  });

  test("an ended subagent says so; its inbox is gone", () => {
    const { messenger, log } = twoSubagents();
    messenger.send(S, undefined, { to: "d_1", text: "before" });
    messenger.leave(S, "d_1", "finished");
    const result = messenger.send(S, undefined, { to: "d_1", text: "after" });
    expect(result.isError).toBe(true);
    expect(text(result)).toBe("claude d_1 has ended (finished); it can't read messages.");
    expect(messenger.takeOf(S, "d_1")).toEqual([]);
    expect(log).toHaveLength(1);
  });

  test("a subagent the main agent waits for can't write to it, but can be written to", () => {
    const { messenger } = setup();
    messenger.join(S, { id: "d_1", agent: "claude" }, false);
    const refused = messenger.send(S, "claude", { to: "main", text: "a question" });
    expect(refused.isError).toBe(true);
    expect(text(refused)).toContain("main is waiting for you to finish");
    expect(messenger.send(S, undefined, { to: "d_1", text: "fine" }).isError).toBeUndefined();
  });
});

describe("limits", () => {
  test("a pair gets PAIR_LIMIT messages per direction until the user writes again", () => {
    const { messenger, log } = twoSubagents();
    for (let i = 0; i < PAIR_LIMIT; i++) {
      expect(messenger.send(S, "claude", { to: "neo", text: `ping ${i}` }).isError).toBeUndefined();
      messenger.takeOf(S, "d_2");
    }
    const refused = messenger.send(S, "claude", { to: "neo", text: "one more" });
    expect(refused.isError).toBe(true);
    expect(text(refused)).toContain(`Message limit reached: ${PAIR_LIMIT} messages from d_1`);
    expect(log).toHaveLength(PAIR_LIMIT);
    // The other direction has its own count.
    expect(messenger.send(S, "neo", { to: "claude", text: "pong" }).isError).toBeUndefined();
    messenger.newRound(S);
    expect(messenger.send(S, "claude", { to: "neo", text: "again" }).isError).toBeUndefined();
  });

  test("an agent that doesn't read holds INBOX_LIMIT messages at most", () => {
    const { messenger } = twoSubagents();
    for (let i = 0; i < INBOX_LIMIT; i++) {
      // Different senders, so the pair limit isn't what stops it.
      messenger.send(S, i % 2 ? "claude" : "neo", { to: "main", text: `n ${i}` });
    }
    const refused = messenger.send(S, "claude", { to: "main", text: "overflow" });
    expect(text(refused)).toContain(`${INBOX_LIMIT} unread messages`);
    expect(messenger.take(S, undefined)).toHaveLength(INBOX_LIMIT);
  });

  test("a background subagent's final reply is not counted", () => {
    const { messenger, log, woken } = twoSubagents();
    for (let i = 0; i < PAIR_LIMIT; i++) messenger.send(S, "claude", { to: "main", text: "n" });
    messenger.take(S, undefined);
    messenger.leave(S, "d_1", "finished");
    messenger.report(S, { id: "d_1", agent: "claude" }, "all done", true);
    expect(log.at(-1)).toEqual({
      type: "agent_message",
      from: "d_1",
      to: "main",
      fromAgent: "claude",
      text: "all done",
      reply: true,
    });
    expect(messenger.take(S, undefined)).toEqual([
      '<message from="d_1" agent="claude" reply="true">\nall done\n</message>',
    ]);
    expect(woken.at(-1)).toBe(S);
  });

  test("a reply of a subagent the user stopped doesn't wake the agent", () => {
    const { messenger, woken } = twoSubagents();
    messenger.leave(S, "d_1", "stopped");
    messenger.report(S, { id: "d_1", agent: "claude" }, "[claude stopped: cancelled]", false);
    expect(woken).toEqual([]);
    expect(messenger.take(S, undefined)).toHaveLength(1);
  });
});

describe("who is calling", () => {
  test("the session's agent, a subagent by its harness, the subagent that is a session", () => {
    const { messenger } = setup();
    messenger.join(S, { id: "d_1", agent: "claude" }, true);
    messenger.join(S, { id: "d_2", agent: "neo", child: "s_child" }, true);
    expect(messenger.identify(S, undefined)).toEqual({ room: S, id: "main" });
    expect(messenger.identify(S, "claude")).toEqual({ room: S, id: "d_1" });
    expect(messenger.identify(S, "codex")).toBeUndefined();
    expect(messenger.identify("s_child", undefined)).toEqual({ room: S, id: "d_2" });
    messenger.leave(S, "d_2", "finished");
    expect(messenger.identify("s_child", undefined)).toEqual({ room: "s_child", id: "main" });
  });

  test("one subagent per harness and directory", () => {
    const { messenger } = setup();
    messenger.join(S, { id: "d_1", agent: "claude" }, true);
    messenger.join(S, { id: "d_2", agent: "claude", child: "s_c" }, true);
    expect(messenger.running(S, "claude")?.id).toBe("d_1");
    expect(messenger.running(S, "claude", "s_c")?.id).toBe("d_2");
    expect(messenger.running(S, "neo")).toBeUndefined();
  });
});

describe("wait", () => {
  test("returns what is already there", async () => {
    const { messenger } = twoSubagents();
    messenger.send(S, "claude", { to: "main", text: "done early" });
    const result = await messenger.wait(S, undefined, {});
    expect(text(result)).toContain("done early");
  });

  test("blocks until a message arrives", async () => {
    const { messenger } = twoSubagents();
    const waiting = messenger.wait(S, undefined, { timeoutSeconds: 30 });
    let settled = false;
    void waiting.then(() => (settled = true));
    await Bun.sleep(30);
    expect(settled).toBe(false);
    messenger.send(S, "claude", { to: "main", text: "here" });
    expect(text(await waiting)).toContain("here");
    // Taken: the next wait doesn't see it again.
    expect(messenger.take(S, undefined)).toEqual([]);
  });

  test("a subagent waits for the session's agent", async () => {
    const { messenger } = twoSubagents();
    const waiting = messenger.wait(S, "neo", { timeoutSeconds: 30 });
    messenger.send(S, undefined, { to: "neo", text: "go ahead" });
    expect(text(await waiting)).toContain("go ahead");
  });

  test("returns when a subagent it names ends", async () => {
    const { messenger } = twoSubagents();
    const waiting = messenger.wait(S, undefined, { for: ["d_1"], timeoutSeconds: 30 });
    await Bun.sleep(10);
    messenger.leave(S, "d_1", "finished");
    expect(text(await waiting)).toBe("claude d_1 has ended (finished).");
    // One that ended before is reported at once.
    expect(text(await messenger.wait(S, undefined, { for: ["d_1"] }))).toContain("has ended");
  });

  test("times out and says who runs; the agent is told when nothing runs at all", async () => {
    const { messenger } = twoSubagents();
    const result = await messenger.wait(S, undefined, { timeoutSeconds: 1 });
    expect(text(result)).toContain("Nothing new in 1s. Running: d_1 (claude), d_2 (neo).");
    messenger.leave(S, "d_1", "finished");
    messenger.leave(S, "d_2", "finished");
    expect(text(await messenger.wait(S, undefined, {}))).toBe(
      "No subagent is running and nothing is waiting for you.",
    );
  });

  test("a named subagent that doesn't exist is an error; a cancelled call returns", async () => {
    const { messenger } = twoSubagents();
    const unknown = await messenger.wait(S, undefined, { for: ["d_9"] });
    expect(unknown.isError).toBe(true);
    const stop = new AbortController();
    const waiting = messenger.wait(S, undefined, { timeoutSeconds: 30 }, stop.signal);
    stop.abort();
    expect(text(await waiting)).toContain("Nothing new");
  });
});

test("what happened in the background is told to main like a message, and wakes it", () => {
  const { messenger, log, woken } = setup();
  messenger.tell(S, "background", "make ended");
  expect(log).toEqual([
    { type: "agent_message", from: "background", to: "main", text: "make ended" },
  ]);
  expect(woken).toEqual([S]);
  expect(messenger.take(S, undefined)).toEqual([
    '<message from="background">\nmake ended\n</message>',
  ]);
});
