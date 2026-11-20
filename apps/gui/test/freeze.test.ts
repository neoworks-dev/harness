import { expect, test } from "bun:test";
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { basis, refreeze, REARRANGE_MS } from "../ui/plugins/sessions/freeze.ts";
import { isSettled, NO_MARKS, toggleSettled } from "../ui/plugins/sessions/marks.ts";

const info = (id: string, more: Partial<SessionInfo> = {}): SessionInfo => ({
  id,
  cwd: "/w",
  createdAt: 0,
  lastActivity: 10,
  lastSeq: 1,
  state: "idle",
  queued: 0,
  ...more,
});

test("the order holds while sessions work, and moves when one is added or time is up", () => {
  const first = refreeze(undefined, [info("a"), info("b")], 0);
  const working = [info("a", { state: "running", lastActivity: 99 }), info("b")];
  // Activity alone: the same snapshot, so the same order.
  expect(refreeze(first, working, 60_000)).toBe(first);
  expect(basis(first, working)[0]).toMatchObject({ state: "idle", lastActivity: 10 });
  // A new session, or the time up: arranged again from what they do now.
  const added = refreeze(first, [...working, info("c")], 60_000);
  expect(added).not.toBe(first);
  expect(basis(added, working)[0]).toMatchObject({ state: "running" });
  expect(refreeze(first, working, REARRANGE_MS)).not.toBe(first);
});

test("a subagent's session is arranged as it is now", () => {
  const frozen = refreeze(undefined, [info("p"), info("k", { parent: "p" })], 0);
  const now = [info("p"), info("k", { parent: "p", state: "running" })];
  expect(basis(frozen, now)[1]?.state).toBe("running");
});

test("settling a session settles its subagents' sessions too, and back", () => {
  const all = [info("p"), info("k", { parent: "p" }), info("kk", { parent: "k" }), info("o")];
  const settled = toggleSettled(NO_MARKS, all[0]!, all, false);
  expect(all.map((s) => isSettled(settled, s))).toEqual([true, true, true, false]);
  const back = toggleSettled(settled, all[0]!, all, true);
  expect(all.some((s) => isSettled(back, s))).toBe(false);
});
