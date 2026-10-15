import { expect, test } from "bun:test";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createEditTool } from "../src/tools/edit.ts";
import { errorMessage, tempDir } from "./helpers.ts";

async function setup(content: string): Promise<{ cwd: string; read: () => Promise<string> }> {
  const cwd = await tempDir();
  await writeFile(join(cwd, "f.txt"), content);
  return { cwd, read: () => readFile(join(cwd, "f.txt"), "utf8") };
}

test("applies several edits matched against the original file", async () => {
  const { cwd, read } = await setup("alpha\nbeta\ngamma\n");
  const result = await createEditTool(cwd).execute("t", {
    path: "f.txt",
    edits: [
      { oldText: "gamma", newText: "GAMMA" },
      { oldText: "alpha", newText: "ALPHA\nextra" },
    ],
  });
  expect(await read()).toBe("ALPHA\nextra\nbeta\nGAMMA\n");
  expect(result.details.oldText).toBe("alpha\nbeta\ngamma\n");
  expect(result.details.newText).toBe("ALPHA\nextra\nbeta\nGAMMA\n");
});

test("an oldText that only exists after an earlier edit is not found", async () => {
  const { cwd, read } = await setup("a b");
  const message = await errorMessage(
    createEditTool(cwd).execute("t", {
      path: "f.txt",
      edits: [
        { oldText: "a", newText: "x" },
        { oldText: "x", newText: "y" },
      ],
    }),
  );
  expect(message).toContain("edits[1]");
  expect(message).toContain("not found");
  expect(await read()).toBe("a b");
});

test("reports how many places a non-unique oldText matches", async () => {
  const { cwd, read } = await setup("foo foo foo");
  const message = await errorMessage(
    createEditTool(cwd).execute("t", {
      path: "f.txt",
      edits: [{ oldText: "foo", newText: "bar" }],
    }),
  );
  expect(message).toContain("matches 3 places");
  expect(await read()).toBe("foo foo foo");
});

test("rejects overlapping edits without touching the file", async () => {
  const { cwd, read } = await setup("one two three");
  const message = await errorMessage(
    createEditTool(cwd).execute("t", {
      path: "f.txt",
      edits: [
        { oldText: "one two", newText: "1" },
        { oldText: "two three", newText: "2" },
      ],
    }),
  );
  expect(message).toContain("overlap");
  expect(await read()).toBe("one two three");
});

test("rejects empty oldText, no edits and no-op edits", async () => {
  const { cwd } = await setup("abc");
  const edit = createEditTool(cwd);
  expect(await errorMessage(edit.execute("t", { path: "f.txt", edits: [] }))).toContain("empty");
  expect(
    await errorMessage(
      edit.execute("t", { path: "f.txt", edits: [{ oldText: "", newText: "x" }] }),
    ),
  ).toContain("oldText is empty");
  expect(
    await errorMessage(
      edit.execute("t", { path: "f.txt", edits: [{ oldText: "b", newText: "b" }] }),
    ),
  ).toContain("no change");
});

test("preserves CRLF line endings, including in multi-line edits", async () => {
  const { cwd, read } = await setup("a\r\nb\r\nc\r\n");
  await createEditTool(cwd).execute("t", {
    path: "f.txt",
    edits: [{ oldText: "a\nb", newText: "x\ny\nz" }],
  });
  expect(await read()).toBe("x\r\ny\r\nz\r\nc\r\n");
});

test("preserves a UTF-8 BOM", async () => {
  const { cwd, read } = await setup("﻿hello\n");
  await createEditTool(cwd).execute("t", {
    path: "f.txt",
    edits: [{ oldText: "hello", newText: "bye" }],
  });
  expect(await read()).toBe("﻿bye\n");
});

test("leaves LF files and files without a BOM alone", async () => {
  const { cwd, read } = await setup("a\nb\n");
  await createEditTool(cwd).execute("t", {
    path: "f.txt",
    edits: [{ oldText: "b", newText: "c" }],
  });
  expect(await read()).toBe("a\nc\n");
});
