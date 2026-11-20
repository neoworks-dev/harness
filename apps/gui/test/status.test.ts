import { expect, test } from "bun:test";
import type { HarnessSettings, SessionEvent } from "@neoworks-dev/neo-protocol/types";
import {
  applyEvent,
  CACHE_COLD_MS,
  cacheCold,
  currentSettings,
  initialStatus,
  nextApprovals,
  optionLabel,
  optionName,
  type SessionStatus,
} from "../ui/lib/status.ts";

let seq = 0;
function fold(...events: SessionEvent[]): SessionStatus {
  return events.reduce(
    (status, event) => applyEvent(status, { session: "s", seq: ++seq, ts: seq * 1000, event }),
    initialStatus(),
  );
}

test("a turn runs from the user's message until it ends", () => {
  const running = fold({ type: "user_message", text: "hi", app: "desktop" });
  expect(running.running).toBe(true);
  expect(running.turnStartedAt).toBeGreaterThan(0);
  expect(
    applyEvent(running, {
      session: "s",
      seq: 99,
      ts: 0,
      event: { type: "turn_ended", stopReason: "end_turn" },
    }).running,
  ).toBe(false);
});

test("settings are kept per harness and the status shows the current one's", () => {
  const status = fold(
    { type: "setting_changed", harness: "claude", option: "model", value: "haiku" },
    { type: "setting_changed", harness: "neo", option: "effort", value: "high" },
    { type: "setting_changed", harness: "daemon", option: "harness", value: "claude" },
    { type: "setting_changed", harness: "daemon", option: "approvals", value: "edits" },
  );
  expect(status.harness).toBe("claude");
  expect(status.approvals).toBe("edits");
  expect(currentSettings(status)).toEqual({ model: "haiku" });
  expect(status.settings.neo).toEqual({ effort: "high" });
});

test("only the session's own agent's usage is shown", () => {
  const status = fold(
    { type: "usage", contextUsed: 10 },
    { type: "usage", agent: "claude", contextUsed: 99 },
  );
  expect(status.usage?.contextUsed).toBe(10);
});

test("a cost reported once stays until a newer one", () => {
  const status = fold(
    { type: "usage", contextUsed: 10, cost: 1.5 },
    { type: "usage", contextUsed: 20 },
  );
  expect(status.usage?.contextUsed).toBe(20);
  expect(status.usage?.cost).toBe(1.5);
});

test("approvals cycle ask, review, edits, auto", () => {
  expect(nextApprovals("ask")).toBe("review");
  expect(nextApprovals("auto")).toBe("ask");
});

const offered: HarnessSettings[] = [
  {
    name: "claude",
    title: "Claude Code",
    current: true,
    options: [
      {
        category: "model",
        current: "haiku",
        values: [
          { value: "default", name: "Default" },
          { value: "haiku", name: "Haiku 5.5" },
        ],
      },
    ],
  },
  { name: "codex", title: "Codex", current: false, options: [] },
];

test("a model shows under the name its harness gives it", () => {
  expect(optionName(offered, "claude", "model", "haiku")).toBe("Haiku 5.5");
  expect(optionLabel(offered, "claude", "model", "haiku")).toBe("Haiku 5.5");
});

test("a value the harness does not list shows as it is", () => {
  expect(optionLabel(offered, "claude", "model", "opus")).toBe("opus");
  expect(optionLabel(offered, "claude", "thought_level", "high")).toBe("high");
  expect(optionLabel(offered, "codex", "model", "haiku")).toBe("haiku");
  expect(optionLabel(offered, "unknown", "model", "haiku")).toBe("haiku");
  expect(optionLabel([], "claude", "model", "haiku")).toBe("haiku");
});

test("the prompt cache goes cold an hour after the last turn, not after a /clear", () => {
  const ended = fold(
    { type: "user_message", text: "hi", app: "desktop" },
    { type: "turn_ended", stopReason: "end_turn" },
  );
  const at = ended.lastTurnAt ?? 0;
  expect(cacheCold(ended, at + CACHE_COLD_MS - 1)).toBe(false);
  expect(cacheCold(ended, at + CACHE_COLD_MS + 1)).toBe(true);
  expect(cacheCold(initialStatus(), Date.now())).toBe(false);
  const cleared = applyEvent(ended, { session: "s", seq: ++seq, ts: at, event: { type: "context_cleared" } });
  expect(cacheCold(cleared, at + 2 * CACHE_COLD_MS)).toBe(false);
  const again = applyEvent(cleared, {
    session: "s",
    seq: ++seq,
    ts: at,
    event: { type: "user_message", text: "back", app: "desktop" },
  });
  expect(again.cleared).toBe(false);
  expect(cacheCold(again, at + 2 * CACHE_COLD_MS)).toBe(false);
});
