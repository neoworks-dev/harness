import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { resolveTemplate } from "../../../daemon/src/view/template.ts";
import { findRoot, ServerPool } from "../src/servers.ts";
import { applyEdits, createTools, editsOf, position, templates } from "../src/tools.ts";

const dirs: string[] = [];
afterAll(() => {
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "lsp-test-")));
  dirs.push(dir);
  return dir;
}

describe("positions and edits", () => {
  const text = "package main\n\nfunc greet(name string) string {\n\treturn greet2(name)\n}\n";

  test("a symbol on a line beats counting columns", () => {
    expect(position(text, { line: 4, symbol: "name" })).toEqual({ line: 3, character: 15 });
    expect(position(text, { line: 3, symbol: "greet" })).toEqual({ line: 2, character: 5 });
    expect(position(text, { line: 4, column: 2 })).toEqual({ line: 3, character: 1 });
    expect(position(text, { line: 4 })).toEqual({ line: 3, character: 1 });
    expect(() => position(text, { line: 4, symbol: "nope" })).toThrow("isn't on line 4");
    expect(() => position(text, { line: 99 })).toThrow("no line 99");
  });

  test("workspace edits apply back to front", () => {
    const edit = {
      changes: {
        "file:///w/a.go": [
          {
            range: { start: { line: 2, character: 5 }, end: { line: 2, character: 10 } },
            newText: "hello",
          },
        ],
      },
      documentChanges: [
        {
          textDocument: { uri: "file:///w/a.go", version: 1 },
          edits: [
            {
              range: { start: { line: 3, character: 8 }, end: { line: 3, character: 14 } },
              newText: "hi",
            },
          ],
        },
      ],
    };
    const edits = editsOf(edit);
    expect([...edits.keys()]).toEqual(["/w/a.go"]);
    expect(applyEdits(text, edits.get("/w/a.go") ?? [])).toBe(
      "package main\n\nfunc hello(name string) string {\n\treturn hi(name)\n}\n",
    );
  });

  test("go.work wins over the module below it", () => {
    const root = tempDir();
    mkdirSync(join(root, "mod", "pkg"), { recursive: true });
    writeFileSync(join(root, "mod", "go.mod"), "module x\n");
    expect(findRoot(join(root, "mod", "pkg", "a.go"), ["go.work", "go.mod"])).toBe(
      join(root, "mod"),
    );
    writeFileSync(join(root, "go.work"), "go 1.22\n");
    expect(findRoot(join(root, "mod", "pkg", "a.go"), ["go.work", "go.mod"])).toBe(root);
  });

  test("templates resolve to valid components", () => {
    const loc = { path: "/w/a.go", line: 3, column: 6, text: "func greet() {}" };
    expect(() =>
      resolveTemplate(templates.diagnostics, {
        path: "/w/a.go",
        errors: 1,
        warnings: 0,
        diagnostics: [
          { line: 4, column: 2, severity: "error", message: "undefined: x", source: "go" },
        ],
      }),
    ).not.toThrow();
    expect(() =>
      resolveTemplate(templates.diagnostics, {
        path: "/w/a.go",
        errors: 0,
        warnings: 0,
        diagnostics: [],
      }),
    ).not.toThrow();
    expect(() => resolveTemplate(templates.definition, { locations: [loc] })).not.toThrow();
    expect(() =>
      resolveTemplate(templates.references, { count: 1, locations: [loc] }),
    ).not.toThrow();
    expect(() => resolveTemplate(templates.hover, { text: "func greet()" })).not.toThrow();
    expect(() =>
      resolveTemplate(templates.rename, { newName: "hi", files: [{ path: "/w/a.go", edits: 2 }] }),
    ).not.toThrow();
  });
});

describe.skipIf(Bun.which("gopls") === null)("with gopls", () => {
  const pool = new ServerPool();
  afterAll(() => pool.shutdown());
  const tools = new Map(createTools(pool).map((t) => [t.name, t]));
  const call = async (
    name: string,
    args: Record<string, unknown>,
  ): Promise<Record<string, unknown>> => {
    const result = await tools.get(name)?.handler(args);
    if (typeof result !== "object" || result.structuredContent === undefined) {
      throw new Error(`${name} returned no structured content`);
    }
    return result.structuredContent;
  };

  const root = tempDir();
  writeFileSync(join(root, "go.mod"), "module example.com/demo\n\ngo 1.22\n");
  const main = join(root, "main.go");
  const util = join(root, "util.go");
  writeFileSync(
    main,
    'package main\n\nimport "fmt"\n\nfunc main() {\n\tfmt.Println(greet("you"))\n\tfmt.Println(missing)\n}\n',
  );
  writeFileSync(
    util,
    'package main\n\n// greet says hello.\nfunc greet(name string) string {\n\treturn "hi " + name\n}\n',
  );

  test("diagnostics, definition, references, hover and rename", async () => {
    const diag = await call("diagnostics", { path: main });
    expect(diag.errors).toBe(1);
    expect(JSON.stringify(diag.diagnostics)).toContain("missing");

    // Fixing the file on disk is seen on the next call.
    writeFileSync(main, readFileSync(main, "utf8").replace("\tfmt.Println(missing)\n", ""));
    expect((await call("diagnostics", { path: main })).errors).toBe(0);

    const def = await call("definition", { path: main, line: 6, symbol: "greet" });
    expect(def.locations).toEqual([
      { path: util, line: 4, column: 6, text: "func greet(name string) string {" },
    ]);
    expect((await call("references", { path: util, line: 4, symbol: "greet" })).count).toBe(2);
    expect(String((await call("hover", { path: main, line: 6, symbol: "greet" })).text)).toContain(
      "greet says hello",
    );

    const renamed = await call("rename", {
      path: util,
      line: 4,
      symbol: "greet",
      newName: "welcome",
    });
    expect(renamed.files).toHaveLength(2);
    expect(readFileSync(main, "utf8")).toContain('welcome("you")');
    expect(readFileSync(util, "utf8")).toContain("func welcome(name string)");
  }, 60_000);
});
