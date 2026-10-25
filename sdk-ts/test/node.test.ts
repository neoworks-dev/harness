import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DisconnectedError } from "../src/errors.ts";
import { rejection } from "./support/wait.ts";
import { daemonJsonAddress, daemonJsonPath } from "../src/node.ts";

function dirWith(daemonJson?: string): string {
  const dir = mkdtempSync(join(tmpdir(), "neo-node-"));
  if (daemonJson !== undefined) writeFileSync(join(dir, "daemon.json"), daemonJson);
  return dir;
}

test("daemon.json lives next to the socket named by NEO_SOCKET, else the runtime dir", () => {
  expect(daemonJsonPath({ NEO_SOCKET: "/run/x/bus.sock" })).toBe("/run/x/daemon.json");
  expect(daemonJsonPath({ XDG_RUNTIME_DIR: "/run/user/1" })).toBe("/run/user/1/neo/daemon.json");
  expect(daemonJsonPath({})).toMatch(/^\/tmp\/neo-\d+\/daemon\.json$/);
});

test("reads the url and token, and nothing else, from daemon.json", async () => {
  const dir = dirWith(JSON.stringify({ url: "http://127.0.0.1:1", token: "t", pid: 5 }));
  const address = await daemonJsonAddress({ NEO_SOCKET: join(dir, "bus.sock") });
  expect(address).toEqual({ url: "http://127.0.0.1:1", token: "t" });
});

test("a missing or foreign daemon.json is a DisconnectedError", async () => {
  const missing = daemonJsonAddress({ NEO_SOCKET: join(dirWith(), "bus.sock") });
  expect(await rejection(missing)).toBeInstanceOf(DisconnectedError);
  expect(String(await rejection(missing))).toContain("not running");
  for (const content of ["not json", "null", JSON.stringify({ url: "http://x" })]) {
    const foreign = daemonJsonAddress({ NEO_SOCKET: join(dirWith(content), "bus.sock") });
    expect(String(await rejection(foreign))).toContain("not a daemon info file");
  }
});
