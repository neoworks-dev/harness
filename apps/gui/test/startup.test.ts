import { expect, test } from "bun:test";
import type { ConnectorInfo, SessionInfo } from "@suite/protocol/types";
import { chooseSession, pickAppId } from "../ui/lib/startup.ts";

function session(id: string, cwd: string, lastActivity: number): SessionInfo {
  return { id, cwd, lastActivity, createdAt: 0, lastSeq: 0, state: "idle", queued: 0 };
}

const sessions = [
  session("s_aaa1", "/w/site", 10),
  session("s_aaa2", "/w/site/", 30),
  session("s_bbb", "/w/api", 20),
];

test("a session asked for by id or unique prefix wins", () => {
  expect(chooseSession(sessions, undefined, "/w/api", { session: "s_aaa1" })).toEqual({
    existing: "s_aaa1",
  });
  expect(chooseSession(sessions, undefined, "/w/api", { session: "s_b" })).toEqual({
    existing: "s_bbb",
  });
  expect(chooseSession(sessions, undefined, "/w/api", { session: "s_a" })).toEqual({
    error: '"s_a" matches 2 sessions: be more specific',
  });
  expect(chooseSession(sessions, undefined, "/w/api", { session: "nope" })).toEqual({
    error: 'no session "nope"',
  });
});

test("the agent's session when it works here, else the latest here, else a new one", () => {
  const focus = { session: "s_aaa1", reason: "initial" as const };
  expect(chooseSession(sessions, focus, "/w/site", {})).toEqual({ existing: "s_aaa1" });
  expect(chooseSession(sessions, focus, "/w/api", {})).toEqual({ existing: "s_bbb" });
  expect(chooseSession(sessions, undefined, "/w/site", {})).toEqual({ existing: "s_aaa2" });
  expect(chooseSession(sessions, undefined, "/elsewhere", {})).toEqual({ new: true });
  expect(chooseSession(sessions, focus, "/w/site", { new: true })).toEqual({ new: true });
});

test("a second window registers under the next free name", () => {
  const app = (name: string, connected: boolean): ConnectorInfo =>
    ({ app: name, connected }) as ConnectorInfo;
  expect(pickAppId([])).toBe("desktop");
  expect(pickAppId([app("desktop", false)])).toBe("desktop");
  expect(pickAppId([app("desktop", true), app("desktop-2", true)])).toBe("desktop-3");
});
