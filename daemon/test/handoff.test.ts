import { describe, expect, test } from "bun:test";
import type { SessionEvent } from "@suite/protocol/types";
import { formatSummary, handoffSummary, missedTurns, type Turn, turnsOf } from "../src/handoff.ts";
import { SessionStore } from "../src/store.ts";

const switchTo = (value: string): SessionEvent => ({
  type: "setting_changed",
  harness: "suite",
  option: "harness",
  value,
});
const user = (text: string): SessionEvent => ({ type: "user_message", app: "cli", text });
const say = (text: string, messageId = "m", agent?: string): SessionEvent => {
  const chunk: SessionEvent = { type: "agent_message_chunk", messageId, text };
  if (agent !== undefined) chunk.agent = agent;
  return chunk;
};
const end: SessionEvent = { type: "turn_ended", stopReason: "end_turn" };

function logOf(events: SessionEvent[]): { store: SessionStore; id: string } {
  const store = new SessionStore(":memory:");
  store.createSession({ id: "s", cwd: "/" });
  for (const event of events) store.append("s", event);
  return { store, id: "s" };
}

/** pi, pi, then claude twice, then pi: the log of a session that switched back and forth. */
const switching: SessionEvent[] = [
  ...[user("one"), say("first"), say("A: one", "m2"), end],
  ...[user("two"), say("A: two"), end],
  switchTo("claude"),
  ...[user("three"), say("thinking", "a"), say("B: three", "b"), end],
  ...[user("four"), say("B: four"), end],
  switchTo("pi"),
  ...[user("five"), say("A: five"), end],
];

describe("turns", () => {
  test("a turn belongs to the harness chosen before it; its reply is the last message", () => {
    const { store } = logOf(switching);
    const turns = turnsOf(store.read("s"), "pi");
    expect(turns.map((t) => [t.harness, t.user, t.reply])).toEqual([
      ["pi", "one", "A: one"],
      ["pi", "two", "A: two"],
      ["claude", "three", "B: three"],
      ["claude", "four", "B: four"],
      ["pi", "five", "A: five"],
    ]);
  });

  test("guest turns, a guest's messages and unfinished turns don't count", () => {
    const { store } = logOf([
      ...[user("/claude fix"), say("working", "g", "claude"), end],
      user("/claude again"),
      { type: "guest_turn", agent: "claude", task: "again", result: "ok", stopReason: "end_turn" },
      end,
      ...[user("real"), say("sub", "s", "pi"), say("mine", "m"), end],
      ...[user("running"), say("so far")],
    ]);
    // "/claude fix" ended without a guest_turn event (it was cancelled before): it still counts.
    expect(turnsOf(store.read("s"), "pi").map((t) => [t.user, t.reply])).toEqual([
      ["/claude fix", ""],
      ["real", "mine"],
    ]);
  });
});

describe("what a harness missed", () => {
  const turn = (harness: string, user: string): Turn => ({ seq: 0, harness, user, reply: "" });
  const turns = [turn("pi", "1"), turn("claude", "2"), turn("claude", "3"), turn("pi", "4")];

  test("the turns after its own last one; everything when it never ran, or lost its context", () => {
    expect(missedTurns(turns, "pi", false).map((t) => t.user)).toEqual([]);
    expect(missedTurns(turns, "claude", false).map((t) => t.user)).toEqual(["4"]);
    expect(missedTurns(turns, "codex", false)).toHaveLength(4);
    expect(missedTurns(turns, "claude", true)).toHaveLength(4);
  });
});

describe("summary", () => {
  test("is a conversation-so-far block with user and agent texts", () => {
    const text = formatSummary([
      { seq: 1, harness: "pi", user: "hello", reply: "hi there" },
      { seq: 5, harness: "pi", user: "stop", reply: "" },
    ]);
    expect(text).toBe(
      [
        "<conversation-so-far>",
        "Earlier turns of this conversation, run by another agent (tool calls left out):",
        "User: hello",
        "Agent: hi there",
        "User: stop",
        "Agent: (no reply)",
        "</conversation-so-far>",
      ].join("\n"),
    );
    expect(formatSummary([])).toBeUndefined();
  });

  test("older turns are dropped first to stay near 6000 characters; long texts are cut", () => {
    const turns: Turn[] = Array.from({ length: 30 }, (_, i) => ({
      seq: i,
      harness: "pi",
      user: `question ${i}`,
      reply: `answer ${i} ${"x".repeat(990)}`,
    }));
    const text = formatSummary(turns) ?? "";
    expect(text.length).toBeLessThan(6300);
    expect(text).toContain("question 29");
    expect(text).not.toContain("question 0\n");
    expect(text).toMatch(/\(\d+ older turns left out\)/);
    const long = formatSummary([{ seq: 1, harness: "pi", user: "q", reply: "y".repeat(5000) }]);
    expect(long?.length).toBeLessThan(1700);
    expect(long).toContain("…");
  });

  test("handoffSummary reads the log: pi gets what claude ran, claude what pi ran after it", () => {
    const { store, id } = logOf(switching);
    const forPi = handoffSummary(store, id, "pi", "pi");
    expect(forPi).toBeUndefined(); // pi's own last turn is the newest
    const forClaude = handoffSummary(store, id, "claude", "pi");
    expect(forClaude).toContain("User: five\nAgent: A: five");
    expect(forClaude).not.toContain("three");
    const everything = handoffSummary(store, id, "claude", "pi", true) ?? "";
    expect(everything).toContain("User: one\nAgent: A: one");
    expect(everything).toContain("User: four\nAgent: B: four");
    expect(everything.indexOf("User: one")).toBeLessThan(everything.indexOf("User: five"));
    // A harness that never ran gets it all.
    expect(handoffSummary(store, id, "codex", "pi")).toContain("User: one");

    store.append(id, switchTo("claude"));
    const back = handoffSummary(store, id, "pi", "pi");
    expect(back).toBeUndefined();
    store.append(id, user("six"));
    store.append(id, end);
    store.append(id, switchTo("pi"));
    // The turn above ran on claude (the switch before it): pi missed it, after its "five".
    expect(handoffSummary(store, id, "pi", "pi")).toContain("User: six\nAgent: (no reply)");
  });
});
