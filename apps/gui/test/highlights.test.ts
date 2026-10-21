import { expect, test } from "bun:test";
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";
import { forDiff, forFile } from "../ui/plugins/viewer/highlights.ts";
import { wrappable } from "../ui/plugins/viewer/look.ts";

const text = (tokens: { text: string }[] | undefined): string =>
  (tokens ?? []).map((token) => token.text).join("");

test("a diff's old and new lines are each read as the code they were", () => {
  // The old text opens a comment that the new one does not, so the lines after it differ.
  const lines: DiffLine[] = [
    { kind: "context", text: "const a = 1;", old: 1, new: 1 },
    { kind: "remove", text: "/* gone", old: 2 },
    { kind: "remove", text: "still gone */", old: 3 },
    { kind: "add", text: "const b = 2;", new: 2 },
    { kind: "context", text: "const c = 3;", old: 4, new: 3 },
  ];
  const light = forDiff("a.ts", lines);
  expect(light?.tokens(1)).toEqual([{ text: "/* gone", kind: "comment" }]);
  expect(light?.tokens(2)).toEqual([{ text: "still gone */", kind: "comment" }]);
  expect(light?.tokens(3)?.[0]).toEqual({ text: "const", kind: "keyword" });
  expect(light?.tokens(4)?.[0]).toEqual({ text: "const", kind: "keyword" });
  expect(text(light?.tokens(4))).toBe("const c = 3;");
});

test("a diff that is no code the highlighter knows has no runs, and is asked once", () => {
  const lines: DiffLine[] = [{ kind: "add", text: "hello", new: 1 }];
  expect(forDiff("notes.xyz", lines)).toBeUndefined();
  expect(forDiff("notes.xyz", lines)).toBeUndefined();
  expect(forDiff("a.ts", lines)).toBe(forDiff("a.ts", lines));
});

test("the lines of a file are read in order, and kept for the same lines", () => {
  const lines = ["// one", "let x = `a", "b`;", "let y = 1;"];
  const light = forFile("x.ts", lines);
  expect(light?.tokens(2)).toEqual([{ text: "b`", kind: "string" }, { text: ";" }]);
  expect(forFile("x.ts", lines)).toBe(light);
  expect(forFile("x.ts", [...lines])).not.toBe(light);
});

test("wrapping keeps the indentation and the spaces inside, and lets a run in the middle break", () => {
  const nbsp = "\u00a0";
  expect(wrappable("  a  b")).toBe(`${nbsp}${nbsp}a${nbsp} b`);
  expect(wrappable("\tx")).toBe(`${nbsp.repeat(4)}x`);
  expect(wrappable(" = x", false)).toBe(" = x");
});
