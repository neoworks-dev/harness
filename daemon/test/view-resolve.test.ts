import { describe, expect, test } from "bun:test";
import { diffLines } from "../src/view/diff.ts";
import { parseMarkdown } from "../src/view/markdown.ts";
import { resolveTemplate, TemplateError } from "../src/view/template.ts";

describe("templates", () => {
  test("whole-string placeholders keep the raw value; partial ones stringify", () => {
    expect(
      resolveTemplate(
        { type: "progress", value: "{{data.p}}", label: "{{data.done}} of {{data.all}}" },
        { p: 0.25, done: 1, all: 4 },
      ),
    ).toEqual({ type: "progress", value: 0.25, label: "1 of 4" });
  });

  test("if drops nodes, each repeats them with a name, a field that is only a missing value is dropped", () => {
    const template = {
      type: "stack",
      children: [
        { type: "text", if: "data.none", markdown: "never" },
        { type: "file", each: "data.files", as: "f", path: "{{f.path}}", name: "{{f.missing}}" },
      ],
    };
    expect(resolveTemplate(template, { files: [{ path: "/a" }, { path: "/b" }] })).toEqual({
      type: "stack",
      children: [
        { type: "file", path: "/a" },
        { type: "file", path: "/b" },
      ],
    });
  });

  test("kv takes an object or a list; table rows may be objects read in column order", () => {
    expect(resolveTemplate({ type: "kv", items: "{{data}}" }, { a: 1, b: true })).toEqual({
      type: "kv",
      items: [
        { key: "a", value: "1" },
        { key: "b", value: "true" },
      ],
    });
    expect(
      resolveTemplate({ type: "table", columns: ["n", "v"], rows: "{{data}}" }, [{ v: 2, n: "x" }]),
    ).toEqual({ type: "table", columns: ["n", "v"], rows: [["x", "2"]] });
  });

  test("a template that doesn't resolve to valid components throws TemplateError", () => {
    expect(() => resolveTemplate({ type: "carousel" }, {})).toThrow(TemplateError);
    expect(() => resolveTemplate({ type: "card", children: [{ text: "no type" }] }, {})).toThrow(
      TemplateError,
    );
    expect(() => resolveTemplate({ type: "button", label: "x" }, {})).toThrow(TemplateError);
  });

  test("several roots (a top-level each) are wrapped in a stack", () => {
    expect(resolveTemplate({ type: "link", each: "data", url: "{{item}}" }, ["u1", "u2"])).toEqual({
      type: "stack",
      children: [
        { type: "link", url: "u1" },
        { type: "link", url: "u2" },
      ],
    });
  });
});

describe("markdown", () => {
  test("adjacent spans with the same style merge; entities are decoded", () => {
    expect(parseMarkdown("a **b** c & `<d>`")).toEqual([
      {
        type: "paragraph",
        spans: [
          { text: "a " },
          { text: "b", bold: true },
          { text: " c & " },
          { text: "<d>", code: true },
        ],
      },
    ]);
  });

  test("streaming prefixes parse to a growing block list", () => {
    const full = "# T\n\npara one\n\n- a\n- b";
    const partial = parseMarkdown(full.slice(0, 12));
    const complete = parseMarkdown(full);
    expect(complete.slice(0, partial.length - 1)).toEqual(partial.slice(0, -1));
  });
});

describe("diff", () => {
  test("new files are all additions", () => {
    expect(diffLines(undefined, "a\nb\n")).toEqual([
      { kind: "hunk", text: "@@ -1,0 +1,2 @@" },
      { kind: "add", text: "a", new: 1 },
      { kind: "add", text: "b", new: 2 },
    ]);
  });

  test("distant changes get separate hunks", () => {
    const old = Array.from({ length: 20 }, (_, i) => `l${i}`).join("\n");
    const changed = old.replace("l1\n", "L1\n").replace("l18", "L18");
    expect(diffLines(old, changed).filter((l) => l.kind === "hunk")).toHaveLength(2);
  });
});
