import { expect, test } from "bun:test";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createEditTool } from "../src/tools/edit.ts";
import { lineHash, tagLines } from "../src/tools/hashline.ts";
import { errorMessage, resultText, tempDir } from "./helpers.ts";

async function setup(content: string): Promise<{ cwd: string; read: () => Promise<string> }> {
  const cwd = await tempDir();
  await writeFile(join(cwd, "f.txt"), content);
  return { cwd, read: () => readFile(join(cwd, "f.txt"), "utf8") };
}

/** The tag read shows for `text` on line `n`. */
const tag = (n: number, text: string): string => `${n}:${lineHash(text)}`;

type Edits = { start?: string; end?: string; after?: string; newText: string }[];

function run(cwd: string, edits: Edits): ReturnType<ReturnType<typeof createEditTool>["execute"]> {
  return createEditTool(cwd).execute("t", { path: "f.txt", edits });
}

test("replaces a line, a range, inserts after a line and at the top, all by the original tags", async () => {
  const { cwd, read } = await setup("a\nb\nc\nd\ne\n");
  const result = await run(cwd, [
    { start: tag(4, "d"), end: tag(5, "e"), newText: "D" },
    { start: tag(1, "a"), newText: "A" },
    { after: tag(2, "b"), newText: "b2\nb3" },
    { after: "0", newText: "top" },
  ]);
  expect(await read()).toBe("top\nA\nb\nb2\nb3\nc\nD\n");
  expect(result.details.oldText).toBe("a\nb\nc\nd\ne\n");
  // The changed lines come back with their new tags (nearby regions merged).
  expect(resultText(result)).toBe(
    `Applied 4 edit(s) to f.txt.\n\n${tagLines(["top", "A", "b", "b2", "b3", "c", "D"]).join("\n")}`,
  );
});

test("an empty newText deletes the lines; a final newline in newText is not an extra line", async () => {
  const { cwd, read } = await setup("a\nb\nc\n");
  await run(cwd, [
    { start: tag(2, "b"), newText: "" },
    { start: tag(3, "c"), newText: "C\n" },
  ]);
  expect(await read()).toBe("a\nC\n");
});

test("a line that changed since it was read fails with the lines there now", async () => {
  const { cwd, read } = await setup("one\ntwo\nthree\n");
  const message = await errorMessage(run(cwd, [{ start: tag(2, "TWO"), newText: "x" }]));
  expect(message).toBe(
    `edits[0].start: line 2 changed since it was read (${tag(2, "TWO")} not found near it). ` +
      `The lines there now:\n${tagLines(["one", "two", "three"]).join("\n")}`,
  );
  expect(await read()).toBe("one\ntwo\nthree\n");
});

test("a line that moved is followed when its hash is the only one near, and the result says so", async () => {
  const { cwd, read } = await setup("new\nfirst\nsecond\n");
  // Read before "new" was added: "second" was line 2.
  const result = await run(cwd, [{ start: tag(2, "second"), newText: "2nd" }]);
  expect(await read()).toBe("new\nfirst\n2nd\n");
  expect(resultText(result)).toContain("edits[0]: line 2 had moved to 3.");
});

test("a moved line is not guessed when several lines near have its hash", async () => {
  const { cwd, read } = await setup("}\na\nb\nc\n}\n");
  const message = await errorMessage(run(cwd, [{ start: `3:${lineHash("}")}`, newText: "x" }]));
  expect(message).toContain("2 lines near it have that hash");
  expect(await read()).toBe("}\na\nb\nc\n}\n");
});

test("a range whose ends moved apart is refused", async () => {
  const { cwd } = await setup("a\nx\nb\nc\n");
  // Read as a, b, c: "x" came in between.
  const message = await errorMessage(
    run(cwd, [{ start: tag(1, "a"), end: tag(2, "b"), newText: "z" }]),
  );
  expect(message).toContain(`lines ${tag(1, "a")}-${tag(2, "b")} changed since they were read`);
});

test("rejects overlapping edits, both start and after, bad tags, no edits and no change", async () => {
  const { cwd, read } = await setup("a\nb\nc\n");
  expect(
    await errorMessage(
      run(cwd, [
        { start: tag(1, "a"), end: tag(2, "b"), newText: "1" },
        { start: tag(2, "b"), newText: "2" },
      ]),
    ),
  ).toContain("overlap");
  expect(
    await errorMessage(run(cwd, [{ start: tag(1, "a"), after: tag(1, "a"), newText: "x" }])),
  ).toContain("either start");
  expect(await errorMessage(run(cwd, [{ start: "a", newText: "x" }]))).toContain(
    'is not a line tag like "12:k3"',
  );
  expect(await errorMessage(run(cwd, []))).toContain("empty");
  expect(await errorMessage(run(cwd, [{ start: tag(2, "b"), newText: "b" }]))).toContain(
    "no change",
  );
  expect(await read()).toBe("a\nb\nc\n");
});

test("a tag copied with its line text still works", async () => {
  const { cwd, read } = await setup("a\nb\n");
  await run(cwd, [{ start: `${tag(2, "b")}|b`, newText: "B" }]);
  expect(await read()).toBe("a\nB\n");
});

test("preserves CRLF line endings, a UTF-8 BOM and a missing final newline", async () => {
  const crlf = await setup("a\r\nb\r\nc\r\n");
  await run(crlf.cwd, [{ start: tag(1, "a"), end: tag(2, "b"), newText: "x\ny\nz" }]);
  expect(await crlf.read()).toBe("x\r\ny\r\nz\r\nc\r\n");

  const bom = await setup("﻿hello\n");
  await run(bom.cwd, [{ start: tag(1, "hello"), newText: "bye" }]);
  expect(await bom.read()).toBe("﻿bye\n");

  const open = await setup("a\nb");
  await run(open.cwd, [{ start: tag(2, "b"), newText: "c" }]);
  expect(await open.read()).toBe("a\nc");
});

test("a hash without its line number works when only one line has it", async () => {
  const { cwd, read } = await setup("a\n}\nb\n}\n");
  await run(cwd, [{ start: lineHash("b"), newText: "B" }]);
  expect(await read()).toBe("a\n}\nB\n}\n");
  const message = await errorMessage(run(cwd, [{ start: lineHash("}"), newText: "x" }]));
  expect(message).toContain(`2 lines have the hash ${lineHash("}")}; give the line number too`);
  expect(message).toContain(`4:${lineHash("}")}|}`);
});
