import { expect, test } from "bun:test";
import type { SessionEvent } from "@suite/protocol/types";
import {
  applyEvent,
  currentSettings,
  initialStatus,
  nextApprovals,
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
    { type: "setting_changed", harness: "pi", option: "effort", value: "high" },
    { type: "setting_changed", harness: "suite", option: "harness", value: "claude" },
    { type: "setting_changed", harness: "suite", option: "approvals", value: "edits" },
  );
  expect(status.harness).toBe("claude");
  expect(status.approvals).toBe("edits");
  expect(currentSettings(status)).toEqual({ model: "haiku" });
  expect(status.settings.pi).toEqual({ effort: "high" });
});

test("only the session's own agent's usage is shown", () => {
  const status = fold(
    { type: "usage", contextUsed: 10 },
    { type: "usage", agent: "claude", contextUsed: 99 },
  );
  expect(status.usage?.contextUsed).toBe(10);
});

test("approvals cycle ask, review, edits, auto", () => {
  expect(nextApprovals("ask")).toBe("review");
  expect(nextApprovals("auto")).toBe("ask");
});
