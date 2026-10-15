import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ViewItem, ViewPatch, ViewSnapshot } from "@suite/protocol/types";
import { BrowserClient } from "../src/browser.ts";
import { SuiteClient } from "../src/client.ts";
import { FakeViewDaemon, THEME } from "./fake-view-daemon.ts";
import { until } from "./helpers.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function item(ref: string, text: string): ViewItem {
  return {
    ref,
    kind: "agent",
    component: { type: "text", blocks: [{ type: "paragraph", spans: [{ text }] }] },
  };
}

async function rig(): Promise<{ daemon: FakeViewDaemon; client: BrowserClient }> {
  const daemon = new FakeViewDaemon();
  cleanups.push(() => daemon.stop());
  const client = await BrowserClient.connect({
    client: "web",
    url: daemon.url,
    token: daemon.token,
  });
  cleanups.push(() => client.close());
  return { daemon, client };
}

describe("theme", () => {
  test("the daemon's theme is kept and announced", async () => {
    const { client } = await rig();
    expect(await client.waitForTheme()).toEqual(THEME);
    expect(client.theme).toEqual(THEME);
  });

  test("a changed theme reaches listeners", async () => {
    const { daemon, client } = await rig();
    await client.waitForTheme();
    const seen: string[] = [];
    client.onTheme((theme) => seen.push(theme.name));
    daemon.broadcast({ topic: "theme", payload: { ...THEME, name: "light" } });
    await until(() => seen.length === 1, "theme");
    expect(seen).toEqual(["light"]);
    expect(client.theme?.name).toBe("light");
  });
});

describe("subscribeView", () => {
  test("snapshot first, then patches; unsubscribing sends view.unsubscribe", async () => {
    const { daemon, client } = await rig();
    daemon.items = [item("a", "hello")];
    const snapshots: ViewSnapshot[] = [];
    const patches: ViewPatch[] = [];
    const sub = client.subscribeView("s1", {
      onSnapshot: (s) => snapshots.push(s),
      onPatch: (p) => patches.push(p),
    });
    await until(() => snapshots.length === 1, "snapshot");
    expect(snapshots[0]).toEqual({ session: "s1", seq: 1, items: [item("a", "hello")] });

    const ops: ViewPatch["ops"] = [{ op: "set", item: item("b", "world") }];
    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 2, ops } });
    // Another session's patches are not ours.
    daemon.broadcast({ topic: "view.patch", payload: { session: "other", seq: 9, ops } });
    await until(() => patches.length === 1, "patch");
    expect(patches).toEqual([{ session: "s1", seq: 2, ops }]);

    sub.close();
    await until(() => daemon.topics().includes("view.unsubscribe"), "unsubscribe");
    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 3, ops } });
    await Bun.sleep(50);
    expect(patches).toHaveLength(1);
  });

  test("patches that arrive before the snapshot are not delivered", async () => {
    const { daemon, client } = await rig();
    const patches: ViewPatch[] = [];
    const snapshots: ViewSnapshot[] = [];
    // A patch right in front of the snapshot would apply to a state that doesn't exist yet.
    const ops: ViewPatch["ops"] = [{ op: "remove", ref: "x" }];
    client.subscribeView("s1", {
      onSnapshot: (s) => snapshots.push(s),
      onPatch: (p) => patches.push(p),
    });
    daemon.broadcast({ topic: "view.patch", payload: { session: "s1", seq: 1, ops } });
    await until(() => snapshots.length === 1, "snapshot");
    expect(patches).toEqual([]);
  });

  test("a refused subscription reports through onError", async () => {
    const { daemon, client } = await rig();
    const errors: Error[] = [];
    client.subscribeView("missing", {
      onSnapshot: () => {},
      onPatch: () => {},
      onError: (e) => errors.push(e),
    });
    await until(() => errors.length === 1, "error");
    expect(errors[0]).toMatchObject({ name: "SuiteError", code: "not_found" });
    expect(daemon.topics()).toContain("view.subscribe");
  });

  test("a reconnect re-subscribes and delivers a fresh snapshot", async () => {
    const { daemon, client } = await rig();
    daemon.items = [item("a", "one")];
    const snapshots: ViewSnapshot[] = [];
    client.subscribeView("s1", { onSnapshot: (s) => snapshots.push(s), onPatch: () => {} });
    await until(() => snapshots.length === 1, "first snapshot");

    daemon.items = [item("a", "one"), item("b", "two")];
    daemon.seq = 7;
    daemon.dropConnections();
    await until(() => snapshots.length === 2, "snapshot after reconnect", 8000);
    expect(snapshots[1]?.seq).toBe(7);
    expect(snapshots[1]?.items).toHaveLength(2);
    expect(daemon.topics().filter((t) => t === "view.subscribe")).toHaveLength(2);
  });

  test("a subscription made before the connection is up starts with it", async () => {
    const daemon = new FakeViewDaemon();
    cleanups.push(() => daemon.stop());
    daemon.items = [item("a", "early")];
    const snapshots: ViewSnapshot[] = [];
    const connecting = BrowserClient.connect({
      client: "web",
      url: daemon.url,
      token: daemon.token,
    });
    const client = await connecting;
    cleanups.push(() => client.close());
    client.subscribeView("s1", { onSnapshot: (s) => snapshots.push(s), onPatch: () => {} });
    await until(() => snapshots.length === 1, "snapshot");
  });
});

describe("action", () => {
  test("sends ui.action with the args and resolves on the ack", async () => {
    const { daemon, client } = await rig();
    await client.action("s1", "c-1", "browser.activate", { tab: 3 });
    await client.action("s1", "c-1", "plain.tool");
    const sent = daemon.received.filter((e) => e.topic === "ui.action").map((e) => e.payload);
    expect(sent).toEqual([
      { session: "s1", ref: "c-1", action: "browser.activate", args: { tab: 3 } },
      { session: "s1", ref: "c-1", action: "plain.tool" },
    ]);
  });
});

describe("entries", () => {
  test("the browser client rejects a wrong token", async () => {
    const daemon = new FakeViewDaemon();
    cleanups.push(() => daemon.stop());
    const outcome = await BrowserClient.connect({
      client: "web",
      url: daemon.url,
      token: "wrong",
    }).then(
      () => "connected",
      (err: unknown) => err,
    );
    expect(outcome).toBeInstanceOf(Error);
  });

  test("refreshAddress supplies the new address after the daemon restarts", async () => {
    const first = new FakeViewDaemon();
    const second = new FakeViewDaemon();
    cleanups.push(() => second.stop());
    let calls = 0;
    const client = await BrowserClient.connect({
      client: "web",
      url: first.url,
      token: first.token,
      refreshAddress: () => {
        calls += 1;
        return Promise.resolve({ url: second.url, token: second.token });
      },
    });
    cleanups.push(() => client.close());
    const snapshots: ViewSnapshot[] = [];
    client.subscribeView("s1", { onSnapshot: (s) => snapshots.push(s), onPatch: () => {} });
    await until(() => snapshots.length === 1, "snapshot");
    first.stop();
    await until(() => snapshots.length === 2, "snapshot from the new daemon", 8000);
    expect(calls).toBeGreaterThan(0);
  });

  test("the Bun client finds the daemon through daemon.json and gets the same view API", async () => {
    const daemon = new FakeViewDaemon();
    cleanups.push(() => daemon.stop());
    const dir = mkdtempSync(join(tmpdir(), "suite-view-"));
    writeFileSync(
      join(dir, "daemon.json"),
      JSON.stringify({ url: daemon.url, token: daemon.token }),
    );
    const client = await SuiteClient.connect({
      client: "bun",
      env: { SUITE_SOCKET: join(dir, "bus.sock") },
    });
    cleanups.push(() => client.close());
    expect((await client.waitForTheme()).name).toBe("test");
  });

  test("the browser entry and what it pulls in import nothing from node", async () => {
    const src = join(import.meta.dir, "../src");
    // Bundled by a child process: Bun's bundler misreads files in a process that ran daemons.
    const build = Bun.spawn(
      [
        process.execPath,
        "build",
        join(src, "browser.ts"),
        "--target=browser",
        "--outdir",
        mkdtempSync(join(tmpdir(), "suite-bundle-")),
      ],
      { stdout: "pipe", stderr: "pipe" },
    );
    expect(await build.exited).toBe(0);
    for (const file of ["browser.ts", "bus-client.ts", "view-state.ts"]) {
      const source = await Bun.file(join(src, file)).text();
      expect(source).not.toMatch(/from "(node:|bun)/);
      expect(source).not.toContain("process.");
    }
  });
});
