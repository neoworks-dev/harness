import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { examples } from "../examples/examples.ts";
import { validateEnvelope } from "../src/validate.ts";
import type { Envelope, SessionEventTag } from "../gen/types.ts";

const dir = join(import.meta.dir, "..", "examples");

describe("protocol examples", () => {
  test("every JSON example has a typed source and vice versa", () => {
    const files = readdirSync(dir)
      .filter((f) => f.endsWith(".json"))
      .map((f) => f.replace(/\.json$/, ""))
      .sort();
    expect(files).toEqual(Object.keys(examples).sort());
  });

  for (const [name, typed] of Object.entries(examples)) {
    test(`${name}: validates and round-trips through the TS types`, () => {
      const parsed: unknown = JSON.parse(readFileSync(join(dir, `${name}.json`), "utf8"));
      const result = validateEnvelope(parsed);
      if (!result.ok) throw new Error(result.error);
      // Decode → typed value → encode gives back the same JSON as the typed source.
      const decoded: Envelope = result.value;
      expect(JSON.parse(JSON.stringify(decoded))).toEqual(typed);
    });
  }

  test("covers every session event type", () => {
    const seen = new Set<string>();
    for (const e of Object.values(examples)) {
      if (e.topic === "session.events") seen.add(e.payload.event.type);
    }
    // A Record over the tag union makes tsc fail when a new event type has no entry here.
    const all: Record<SessionEventTag, true> = {
      user_message: true,
      agent_message_chunk: true,
      agent_thought_chunk: true,
      tool_call: true,
      tool_call_update: true,
      turn_ended: true,
      app_connected: true,
      app_disconnected: true,
      focus_changed: true,
      harness_message: true,
      tool_result: true,
      guest_turn: true,
      permission_request: true,
      permission_resolved: true,
      setting_changed: true,
      choice: true,
      choice_made: true,
    };
    expect([...seen].sort()).toEqual(Object.keys(all).sort());
  });
});

describe("validateEnvelope rejects", () => {
  const bad: Record<string, unknown> = {
    "unknown topic": { topic: "nope" },
    "missing payload": { topic: "session.input", id: "1" },
    "extra field": { topic: "bus.ack", id: "1", extra: true },
    "unknown event type": {
      topic: "session.events",
      payload: { session: "s", seq: 1, ts: 0, event: { type: "nope" } },
    },
    "seq zero": {
      topic: "session.events",
      payload: { session: "s", seq: 0, ts: 0, event: { type: "app_disconnected", app: "x" } },
    },
    "bad enum": {
      topic: "session.events",
      payload: { session: "s", seq: 1, ts: 0, event: { type: "turn_ended", stopReason: "bored" } },
    },
  };
  for (const [name, value] of Object.entries(bad)) {
    test(name, () => expect(validateEnvelope(value).ok).toBe(false));
  }
});
