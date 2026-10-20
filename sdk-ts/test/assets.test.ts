import { expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { SuiteError } from "../src/index.ts";
import { cleanupAfterEach } from "./support/cleanup.ts";
import { startRig } from "./support/real-daemon.ts";

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

const later = cleanupAfterEach();

test("putAsset takes data or a path, and getAsset finds it again", async () => {
  const { client, daemon } = await startRig(later);

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
});

test("getAsset rejects with SuiteError not_found for an unknown handle", async () => {
  const { client } = await startRig(later);
  const missing = await client
    .getAsset("asset://img-ffffffffffff")
    .catch((thrown: unknown) => thrown);
  expect(missing).toBeInstanceOf(SuiteError);
  expect(missing).toMatchObject({ code: "not_found" });
});

test("fetchAsset rejects with bad_request for a URL that is not a public http(s) image", async () => {
  const { client } = await startRig(later);
  for (const url of ["http://127.0.0.1:1/a.png", "ftp://example.com/a.png"]) {
    const refused = await client.fetchAsset(url).catch((thrown: unknown) => thrown);
    expect(refused).toBeInstanceOf(SuiteError);
    expect(refused).toMatchObject({ code: "bad_request" });
  }
});
