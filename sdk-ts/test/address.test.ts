import { expect, test } from "bun:test";
import { assetUrl, busUrl, fixedAddress } from "../src/address.ts";

const address = { url: "http://127.0.0.1:4000/", token: "a b" };

test("busUrl makes the websocket URL from http, https, ws or wss, and escapes the token", () => {
  expect(busUrl(address)).toBe("ws://127.0.0.1:4000/bus?token=a%20b");
  expect(busUrl({ url: "https://host:1", token: "t" })).toBe("wss://host:1/bus?token=t");
  expect(busUrl({ url: "ws://host:2//", token: "t" })).toBe("ws://host:2/bus?token=t");
});

test("fixedAddress always answers with the same address", async () => {
  const source = fixedAddress(address);
  expect(await source()).toEqual(address);
  expect(await source()).toEqual(address);
});

test("assetUrl builds the daemon's asset URL for asset handles only", () => {
  expect(assetUrl(address, "asset://img-0123456789ab")).toBe(
    "http://127.0.0.1:4000/assets/img-0123456789ab?token=a%20b",
  );
  expect(assetUrl({ ...address, url: "ws://127.0.0.1:4000" }, "asset://file-0123456789ab")).toBe(
    "http://127.0.0.1:4000/assets/file-0123456789ab?token=a%20b",
  );
  for (const notAHandle of ["asset://../x", "asset://img-1", "https://x/y.png", "/tmp/a.png"]) {
    expect(assetUrl(address, notAHandle)).toBeUndefined();
  }
});
