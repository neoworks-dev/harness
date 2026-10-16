import { afterEach, expect, test } from "bun:test";
import { statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Envelope } from "@suite/protocol/types";
import { validateEnvelope } from "@suite/protocol/validate";
import { AssetStore } from "../src/assets.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function daemon(): Promise<TestDaemon> {
  const d = await startTestDaemon();
  cleanups.push(() => d.stop());
  return d;
}

interface Client {
  received: Envelope[];
  send: (e: unknown) => void;
}

function connect(d: TestDaemon, name: string): Client {
  const received: Envelope[] = [];
  const handle = d.hub.connect({
    send(frame) {
      const result = validateEnvelope(JSON.parse(frame));
      if (!result.ok) throw new Error(`invalid frame: ${result.error}\n${frame}`);
      received.push(result.value);
    },
  });
  const send = (e: unknown): void => handle.receive(JSON.stringify(e));
  send({ topic: "bus.hello", payload: { protocol: 1, client: name } });
  return { received, send };
}

test("the store is content-addressed and sniffs types", async () => {
  const d = await daemon();
  const store = new AssetStore(join(d.dir, "assets-unit"));
  const a = store.put({ bytes: PNG, name: "logo.png", app: "browser" });
  expect(a.handle).toMatch(/^asset:\/\/img-[0-9a-f]{12}$/);
  expect(a).toMatchObject({
    name: "logo.png",
    mime: "image/png",
    size: PNG.length,
    app: "browser",
  });
  expect(a.path.endsWith(".png")).toBe(true);
  expect(statSync(a.path).mode & 0o777).toBe(0o644);
  // Same bytes, same handle (the first name and app are kept).
  expect(store.put({ bytes: PNG, name: "other.png" })).toEqual(a);
  expect(store.get(a.handle)).toEqual(a);
  expect(store.resolve(a.handle)).toBe(a.path);
  expect(store.get("asset://img-ffffffffffff")).toBeUndefined();
  expect(store.get("asset://../../etc/passwd")).toBeUndefined();

  const text = store.put({ bytes: new TextEncoder().encode("hi"), name: "notes.md" });
  expect(text).toMatchObject({ mime: "text/markdown", name: "notes.md" });
  expect(text.handle.startsWith("asset://file-")).toBe(true);
});

test("asset.put (path or data) and asset.get over the bus", async () => {
  const d = await daemon();
  const c = connect(d, "photo-editor");
  const file = join(d.dir, "crop.png");
  writeFileSync(file, PNG);
  c.send({ topic: "asset.put", id: "1", payload: { path: file } });
  const put = c.received.at(-1);
  if (put?.topic !== "asset.info") throw new Error(`got ${put?.topic}`);
  expect(put.payload).toMatchObject({ name: "crop.png", mime: "image/png", app: "photo-editor" });

  c.send({
    topic: "asset.put",
    id: "2",
    payload: { data: Buffer.from("hello").toString("base64"), name: "a.txt" },
  });
  expect(c.received.at(-1)).toMatchObject({ topic: "asset.info", payload: { mime: "text/plain" } });

  c.send({ topic: "asset.get", id: "3", payload: { handle: put.payload.handle } });
  expect(c.received.at(-1)).toEqual({ topic: "asset.info", id: "3", payload: put.payload });
  c.send({ topic: "asset.get", id: "4", payload: { handle: "asset://img-ffffffffffff" } });
  expect(c.received.at(-1)).toMatchObject({ topic: "bus.error", payload: { code: "not_found" } });
  c.send({ topic: "asset.put", id: "5", payload: {} });
  expect(c.received.at(-1)).toMatchObject({ topic: "bus.error", payload: { code: "bad_request" } });
});

test("HTTP /assets takes uploads and serves files with the token", async () => {
  const d = await daemon();
  const { url, token } = d.info;
  const auth = { authorization: `Bearer ${token}` };
  expect((await fetch(`${url}/assets?name=x.png`, { method: "POST", body: PNG })).status).toBe(401);
  const res = await fetch(`${url}/assets?name=x.png&app=browser`, {
    method: "POST",
    body: PNG,
    headers: auth,
  });
  const asset = d.assets.get(String(((await res.json()) as { handle: string }).handle));
  expect(asset).toMatchObject({ name: "x.png", app: "browser" });
  const id = asset?.handle.slice("asset://".length) ?? "";

  const got = await fetch(`${url}/assets/${id}?token=${token}`);
  expect(got.headers.get("content-type")).toBe("image/png");
  expect(got.headers.get("content-security-policy")).toContain("sandbox");
  expect(new Uint8Array(await got.arrayBuffer())).toEqual(PNG);
  expect((await fetch(`${url}/assets/${id}`)).status).toBe(401);
  expect((await fetch(`${url}/assets/img-ffffffffffff`, { headers: auth })).status).toBe(404);
});

test("file.changed: agent edits are announced, app saves are passed on to the others", async () => {
  const d = await daemon();
  const editor = connect(d, "code-editor");
  const browser = connect(d, "browser");
  const session = d.focus.current().session;
  d.store.append(session, {
    type: "tool_call",
    toolCallId: "w",
    title: "write",
    kind: "edit",
    status: "pending",
  });
  d.store.append(session, {
    type: "tool_call_update",
    toolCallId: "w",
    status: "completed",
    content: [{ type: "diff", path: "/site/a.txt", newText: "x" }],
  });
  const changed = (c: Client): Envelope[] => c.received.filter((e) => e.topic === "file.changed");
  expect(changed(editor)).toEqual([
    { topic: "file.changed", payload: { path: "/site/a.txt", app: "agent", session } },
  ]);

  editor.send({ topic: "file.changed", payload: { path: "/site/b.txt" } });
  expect(changed(browser).at(-1)).toEqual({
    topic: "file.changed",
    payload: { path: "/site/b.txt", app: "code-editor" },
  });
  expect(changed(editor)).toHaveLength(1);
});
