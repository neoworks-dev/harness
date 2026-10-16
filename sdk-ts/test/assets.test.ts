import { afterEach, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import type { FileChanged } from "@suite/protocol/types";
import { assetUrl } from "../src/browser.ts";
import { SuiteClient, SuiteError } from "../src/client.ts";
import { start, until } from "./helpers.ts";

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

test("putAsset (data and path) and getAsset", async () => {
  const r = await start();
  cleanups.push(() => r.daemon.stop(200));
  cleanups.push(() => r.client.close());
  const { client, daemon } = r;

  const data = await client.putAsset({
    data: Buffer.from(PNG).toString("base64"),
    name: "logo.png",
  });
  expect(data).toMatchObject({ name: "logo.png", mime: "image/png", size: PNG.length });
  expect(data.handle).toMatch(/^asset:\/\/img-[0-9a-f]{12}$/);
  expect(new Uint8Array(await Bun.file(data.path).arrayBuffer())).toEqual(PNG);

  const file = join(daemon.dir, "copy.png");
  writeFileSync(file, PNG);
  const byPath = await client.putAsset({ path: file });
  expect(byPath.handle).toBe(data.handle);

  expect(await client.getAsset(data.handle)).toEqual(data);
  const missing = await client.getAsset("asset://img-ffffffffffff").catch((err: unknown) => err);
  expect(missing).toBeInstanceOf(SuiteError);
  expect(missing).toMatchObject({ code: "not_found" });
});

test("fileChanged reaches other clients (not the sender), onFileChanged can unsubscribe", async () => {
  const r = await start();
  cleanups.push(() => r.daemon.stop(200));
  cleanups.push(() => r.client.close());
  const other = await SuiteClient.connect({
    client: "editor",
    env: r.daemon.env,
    presence: { app: "editor", kind: "app", open: true },
  });
  cleanups.push(() => other.close());

  const seen: FileChanged[] = [];
  const own: FileChanged[] = [];
  const off = r.client.onFileChanged((change) => seen.push(change));
  other.onFileChanged((change) => own.push(change));

  other.fileChanged("/tmp/a.txt");
  await until(() => seen.length === 1, "file.changed");
  expect(seen[0]).toEqual({ path: "/tmp/a.txt", app: "editor" });

  off();
  other.fileChanged("/tmp/b.txt");
  await Bun.sleep(100);
  expect(seen).toHaveLength(1);
  expect(own).toHaveLength(0);
});

test("assetUrl builds the daemon's asset URL for handles only", () => {
  const address = { url: "http://127.0.0.1:4000/", token: "a b" };
  expect(assetUrl(address, "asset://img-0123456789ab")).toBe(
    "http://127.0.0.1:4000/assets/img-0123456789ab?token=a%20b",
  );
  expect(assetUrl({ ...address, url: "ws://127.0.0.1:4000" }, "asset://file-0123456789ab")).toBe(
    "http://127.0.0.1:4000/assets/file-0123456789ab?token=a%20b",
  );
  for (const bad of ["asset://../x", "asset://img-1", "https://x/y.png", "/tmp/a.png"]) {
    expect(assetUrl(address, bad)).toBeUndefined();
  }
});
