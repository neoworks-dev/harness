import { expect, test } from "bun:test";
import { assetHandleOf } from "../ui/lib/asset-path.ts";

test("a path into the asset store names its handle", () => {
  expect(assetHandleOf("/home/u/.local/share/neo/assets/img-0006dd1d7373.jpg")).toBe(
    "asset://img-0006dd1d7373",
  );
});

test("any other source stays as it is", () => {
  expect(assetHandleOf("asset://img-0006dd1d7373")).toBe("asset://img-0006dd1d7373");
  expect(assetHandleOf("/home/u/pictures/cat.png")).toBe("/home/u/pictures/cat.png");
  expect(assetHandleOf("/home/u/assets/logo.png")).toBe("/home/u/assets/logo.png");
  expect(assetHandleOf("https://example.com/assets/img-0006dd1d7373.jpg")).toBe(
    "https://example.com/assets/img-0006dd1d7373.jpg",
  );
});
