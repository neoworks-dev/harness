import { expect, test } from "bun:test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createReadTool } from "../src/tools/read.ts";
import { createWriteTool } from "../src/tools/write.ts";
import { errorMessage, resultText, tempDir } from "./helpers.ts";

const lines = (n: number): string =>
  Array.from({ length: n }, (_, i) => `line ${i + 1}`).join("\n");

test("read returns the file text and resolves relative paths against the cwd", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "a.txt"), "one\ntwo\n");
  const read = createReadTool(cwd);
  expect(resultText(await read.execute("t", { path: "a.txt" }))).toBe("one\ntwo");
  expect(resultText(await read.execute("t", { path: join(cwd, "a.txt") }))).toBe("one\ntwo");
});

test("read honours offset and limit and says how to continue", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "f"), lines(10));
  const read = createReadTool(cwd);
  const text = resultText(await read.execute("t", { path: "f", offset: 3, limit: 2 }));
  expect(text).toStartWith("line 3\nline 4\n");
  expect(text).toContain("Showing lines 3-4 of 10");
  expect(text).toContain("offset=5");
  const rest = resultText(await read.execute("t", { path: "f", offset: 9 }));
  expect(rest).toBe("line 9\nline 10");
  expect(await errorMessage(read.execute("t", { path: "f", offset: 11 }))).toContain("beyond");
});

test("read truncates at 2000 lines", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "big"), lines(2500));
  const text = resultText(await createReadTool(cwd).execute("t", { path: "big" }));
  expect(text).toContain("line 2000\n");
  expect(text).not.toContain("line 2001\n");
  expect(text).toContain("Use offset=2001 to continue");
});

test("read truncates at 50KB", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "wide"), `${"x".repeat(1000)}\n`.repeat(100));
  const read = createReadTool(cwd);
  const text = resultText(await read.execute("t", { path: "wide" }));
  expect(text.length).toBeLessThan(52 * 1024);
  expect(text).toMatch(/Use offset=\d+ to continue/);
});

test("read strips a BOM and CRLF", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "w"), "﻿a\r\nb\r\n");
  expect(resultText(await createReadTool(cwd).execute("t", { path: "w" }))).toBe("a\nb");
});

test("read returns images as base64 and rejects other binary files", async () => {
  const cwd = await tempDir();
  const png = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 1, 2, 3]);
  await writeFile(join(cwd, "p.png"), png);
  await writeFile(join(cwd, "blob.bin"), Buffer.from([1, 2, 0, 3]));
  const read = createReadTool(cwd);
  const result = await read.execute("t", { path: "p.png" });
  const image = result.content.find((part) => part.type === "image");
  expect(image).toEqual({ type: "image", data: png.toString("base64"), mimeType: "image/png" });
  expect(await errorMessage(read.execute("t", { path: "blob.bin" }))).toContain("binary");
});

test("read expands ~ and fails clearly on a missing file", async () => {
  const read = createReadTool(await tempDir());
  expect(await errorMessage(read.execute("t", { path: "~/.suite-no-such-file" }))).toContain(
    join(homedir(), ".suite-no-such-file"),
  );
});

test("write creates parent directories and reports the new content", async () => {
  const cwd = await tempDir();
  const result = await createWriteTool(cwd).execute("t", { path: "a/b/c.txt", content: "hi" });
  expect(await readFile(join(cwd, "a/b/c.txt"), "utf8")).toBe("hi");
  expect(result.details).toEqual({
    path: join(cwd, "a/b/c.txt"),
    oldText: undefined,
    newText: "hi",
  });
  expect(resultText(result)).toContain("Created");
});

test("write overwrites and keeps the old content in the details", async () => {
  const cwd = await tempDir();
  await mkdir(join(cwd, "d"));
  await writeFile(join(cwd, "d/f"), "old");
  const result = await createWriteTool(cwd).execute("t", { path: "d/f", content: "new" });
  expect(await readFile(join(cwd, "d/f"), "utf8")).toBe("new");
  expect(result.details.oldText).toBe("old");
  expect(result.details.newText).toBe("new");
  expect(resultText(result)).toContain("Overwrote");
});
