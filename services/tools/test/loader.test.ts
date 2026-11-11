import { afterAll, describe, expect, test } from "bun:test";
import {
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  unlinkSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fingerprint } from "../src/approval.ts";
import { ToolFiles } from "../src/loader.ts";
import { checkTool } from "../src/spec.ts";

const dirs: string[] = [];
const open: ToolFiles[] = [];
afterAll(() => {
  for (const files of open) files.close();
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "tools-test-")));
  dirs.push(dir);
  return dir;
}

function filesOf(
  dir: string,
  reserved: string[] = [],
  limits: { timeoutMs?: number; stuckMs?: number } = {},
): ToolFiles {
  const files = new ToolFiles(dir, { reserved: new Set(reserved), ...limits });
  open.push(files);
  return files;
}

const tool = (name: string, result = "ok"): string => `export default {
  name: "${name}",
  description: "Says ${result}",
  inputSchema: { type: "object", properties: {} },
  handler: async () => "${result}",
};`;

const byName = <T extends { name: string }>(list: T[], name: string): T => {
  const found = list.find((t) => t.name === name);
  if (!found) throw new Error(`no ${name}`);
  return found;
};

describe("loading a folder", () => {
  test("each file gives its tools, in file order; an array gives several", async () => {
    const dir = tempDir();
    writeFileSync(join(dir, "b.ts"), tool("beta"));
    writeFileSync(join(dir, "a.js"), tool("alpha"));
    writeFileSync(
      join(dir, "c.mjs"),
      `const t = (name) => ({ name, description: "d", inputSchema: {}, handler: async () => name });
       export default [t("one"), t("two")];`,
    );
    writeFileSync(join(dir, "notes.txt"), "not a tool file");
    const { tools, files } = await filesOf(dir).reload();
    expect(tools.map((t) => t.name)).toEqual(["alpha", "beta", "one", "two"]);
    expect(files).toEqual([
      { file: "a.js", tools: ["alpha"] },
      { file: "b.ts", tools: ["beta"] },
      { file: "c.mjs", tools: ["one", "two"] },
    ]);
    expect(await byName(tools, "alpha").handler({})).toBe("ok");
    expect(await byName(tools, "two").handler({})).toBe("two");
  });

  test("a broken file costs only its own tools", async () => {
    const dir = tempDir();
    writeFileSync(join(dir, "good.ts"), tool("good"));
    writeFileSync(join(dir, "syntax.ts"), "export default {{");
    writeFileSync(join(dir, "throws.ts"), 'throw new Error("boom");');
    writeFileSync(join(dir, "empty.ts"), "export const x = 1;");
    writeFileSync(join(dir, "bad-name.ts"), tool("not a name"));
    const { tools, files } = await filesOf(dir).reload();
    expect(tools.map((t) => t.name)).toEqual(["good"]);
    const byFile = Object.fromEntries(files.map((f) => [f.file, f]));
    expect(byFile["good.ts"]?.error).toBeUndefined();
    expect(byFile["syntax.ts"]?.tools).toEqual([]);
    expect(byFile["syntax.ts"]?.error).toContain("can't load");
    expect(byFile["throws.ts"]?.error).toContain("boom");
    expect(byFile["empty.ts"]?.error).toBe("has no default export");
    expect(byFile["bad-name.ts"]?.error).toContain("name must be");
  });

  test("a name that is taken, by another file or by the service, is left out", async () => {
    const dir = tempDir();
    writeFileSync(join(dir, "a.ts"), tool("same", "first"));
    writeFileSync(join(dir, "b.ts"), tool("same", "second"));
    writeFileSync(join(dir, "c.ts"), tool("status"));
    const { tools, files } = await filesOf(dir, ["status"]).reload();
    expect(tools.map((t) => t.description)).toEqual(["Says first"]);
    expect(files.find((f) => f.file === "b.ts")?.error).toBe("same: the name is taken");
    expect(files.find((f) => f.file === "c.ts")?.error).toBe("status: the name is taken");
  });

  test("a folder that is not there has no tools", async () => {
    expect(await filesOf(join(tempDir(), "missing")).reload()).toEqual({ tools: [], files: [] });
  });
});

describe("reloading", () => {
  const counter = (label: string): string => `let n = 0;
export default {
  name: "count",
  description: "${label}",
  inputSchema: {},
  handler: async () => String(++n),
};`;

  test("a changed file is loaded again, one that did not change keeps its state", async () => {
    const dir = tempDir();
    const path = join(dir, "count.ts");
    writeFileSync(path, counter("before"));
    utimesSync(path, 1000, 1000);
    writeFileSync(join(dir, "other.ts"), tool("other"));
    const files = filesOf(dir);

    let { tools } = await files.reload();
    expect(await byName(tools, "count").handler({})).toBe("1");
    expect(await byName(tools, "count").handler({})).toBe("2");

    // Nothing changed: the worker, and its counter, stay.
    ({ tools } = await files.reload());
    expect(await byName(tools, "count").handler({})).toBe("3");

    writeFileSync(path, counter("after"));
    utimesSync(path, 2000, 2000);
    ({ tools } = await files.reload());
    expect(byName(tools, "count").description).toBe("after");
    expect(await byName(tools, "count").handler({})).toBe("1");
  });

  test("a file that is removed takes its tools with it", async () => {
    const dir = tempDir();
    writeFileSync(join(dir, "a.ts"), tool("a"));
    writeFileSync(join(dir, "b.ts"), tool("b"));
    const files = filesOf(dir);
    expect((await files.reload()).tools.map((t) => t.name)).toEqual(["a", "b"]);
    unlinkSync(join(dir, "a.ts"));
    expect((await files.reload()).tools.map((t) => t.name)).toEqual(["b"]);
  });
});

describe("a tool that misbehaves", () => {
  test("a slow call fails after the limit, and its worker goes on with what it holds", async () => {
    const dir = tempDir();
    writeFileSync(
      join(dir, "watch.ts"),
      `let ticks = 0;
setInterval(() => ticks++, 10);
export default [
  { name: "slow", description: "d", inputSchema: {}, handler: () => new Promise((r) => setTimeout(() => r("late"), 1000)) },
  { name: "ticks", description: "d", inputSchema: {}, handler: async () => String(ticks) },
];`,
    );
    const { tools } = await filesOf(dir, [], { timeoutMs: 200 }).reload();
    const before = Number(await byName(tools, "ticks").handler({}));
    const err = await byName(tools, "slow")
      .handler({})
      .catch((e: unknown) => e);
    expect(String(err)).toContain("took longer than 0.2s");
    // The same worker: what it started at load still runs.
    expect(Number(await byName(tools, "ticks").handler({}))).toBeGreaterThan(before);
  });

  test("one that spins stops answering, is ended, and the others carry on", async () => {
    const dir = tempDir();
    writeFileSync(
      join(dir, "hang.ts"),
      `export default { name: "hang", description: "d", inputSchema: {}, handler: () => { while (true) {} } };`,
    );
    writeFileSync(join(dir, "fine.ts"), tool("fine"));
    const { tools } = await filesOf(dir, [], { stuckMs: 300 }).reload();
    const started = Date.now();
    const err = await byName(tools, "hang")
      .handler({})
      .catch((e: unknown) => e);
    expect(String(err)).toContain("stopped answering");
    expect(Date.now() - started).toBeLessThan(3000);
    // The service was never blocked.
    expect(await byName(tools, "fine").handler({})).toBe("ok");
  });

  test("one that crashes its worker fails that call, and starts fresh on the next", async () => {
    const dir = tempDir();
    writeFileSync(
      join(dir, "crash.ts"),
      `let calls = 0;
export default {
  name: "crash",
  description: "d",
  inputSchema: {},
  handler: (args) => {
    calls++;
    if (args.die) { setTimeout(() => { throw new Error("late failure"); }, 5); return new Promise(() => {}); }
    return Promise.resolve("call " + calls);
  },
};`,
    );
    const { tools } = await filesOf(dir).reload();
    const crash = byName(tools, "crash");
    expect(await crash.handler({})).toBe("call 1");
    const err = await crash.handler({ die: true }).catch((e: unknown) => e);
    expect(String(err)).toContain("crashed");
    // A new worker: the file's state starts over.
    expect(await crash.handler({})).toBe("call 1");
  });

  test("a thrown error is the error of the call", async () => {
    const dir = tempDir();
    writeFileSync(
      join(dir, "t.ts"),
      `export default { name: "t", description: "d", inputSchema: {}, handler: () => { throw new Error("no"); } };`,
    );
    const { tools } = await filesOf(dir).reload();
    const err = await byName(tools, "t")
      .handler({})
      .catch((e: unknown) => e);
    expect(String(err)).toContain("no");
  });
});

describe("files the user allowed", () => {
  /** A folder with a tool that uses a helper in lib/, and the allowed set the test controls. */
  function setup(): {
    dir: string;
    allowed: Record<string, string>;
    files: ToolFiles;
    write: (helper: string) => void;
  } {
    const dir = tempDir();
    mkdirSync(join(dir, "lib"));
    const write = (helper: string): void => {
      writeFileSync(join(dir, "lib", "say.ts"), `export const say = () => "${helper}";`);
    };
    write("one");
    writeFileSync(
      join(dir, "talk.ts"),
      `import { say } from "./lib/say.ts";
export default { name: "talk", description: "d", inputSchema: {}, handler: async () => say() };`,
    );
    const allowed: Record<string, string> = {};
    const files = new ToolFiles(dir, { approved: () => allowed });
    open.push(files);
    return { dir, allowed, files, write };
  }

  test("a file nobody allowed does not run, and says so", async () => {
    const { files } = setup();
    const { tools, files: status } = await files.reload();
    expect(tools).toEqual([]);
    expect(status).toEqual([
      { file: "talk.ts", tools: [], error: expect.stringContaining("not allowed yet") },
    ]);
  });

  test("an allowed file runs; changed, it stops until it is allowed as it is", async () => {
    const { dir, allowed, files } = setup();
    allowed["talk.ts"] = fingerprint(dir)["talk.ts"] ?? "";
    let { tools } = await files.reload();
    expect(await byName(tools, "talk").handler({})).toBe("one");

    writeFileSync(join(dir, "talk.ts"), tool("talk", "changed"));
    const after = await files.reload();
    expect(after.tools).toEqual([]);
    expect(after.files[0]?.error).toContain("changed since the user approved it");

    allowed["talk.ts"] = fingerprint(dir)["talk.ts"] ?? "";
    ({ tools } = await files.reload());
    expect(await byName(tools, "talk").handler({})).toBe("changed");
  });

  test("a helper the file imports is part of what was allowed", async () => {
    const { dir, allowed, files, write } = setup();
    allowed["talk.ts"] = fingerprint(dir)["talk.ts"] ?? "";
    expect((await files.reload()).tools).toHaveLength(1);

    write("something else");
    const after = await files.reload();
    expect(after.tools).toEqual([]);
    expect(after.files[0]?.error).toContain("changed since the user approved it");
  });

  test("without the set every file runs, and a changed helper starts its tool over", async () => {
    const dir = tempDir();
    mkdirSync(join(dir, "lib"));
    writeFileSync(join(dir, "lib", "say.ts"), `export const say = () => "one";`);
    writeFileSync(
      join(dir, "talk.ts"),
      `import { say } from "./lib/say.ts";
export default { name: "talk", description: "d", inputSchema: {}, handler: async () => say() };`,
    );
    const files = filesOf(dir);
    expect(await byName((await files.reload()).tools, "talk").handler({})).toBe("one");
    writeFileSync(join(dir, "lib", "say.ts"), `export const say = () => "two";`);
    expect(await byName((await files.reload()).tools, "talk").handler({})).toBe("two");
  });
});

describe("what a tool file may import", () => {
  test("built-ins and the files of its folder, nothing outside it", async () => {
    const root = tempDir();
    const dir = join(root, "tools");
    mkdirSync(join(dir, "lib"), { recursive: true });
    writeFileSync(join(root, "outside.ts"), `export const v = "outside";`);
    writeFileSync(join(dir, "lib", "in.ts"), `export const v = "in";`);
    writeFileSync(
      join(dir, "ok.ts"),
      `import { v } from "./lib/in.ts";
import { join } from "node:path";
export default { name: "ok", description: "d", inputSchema: {}, handler: async () => v + join("a", "b") };`,
    );
    // Approving ok.ts says nothing of ../outside.ts, which could change at any time.
    writeFileSync(
      join(dir, "out.ts"),
      `import { v } from "../outside.ts";
export default { name: "out", description: "d", inputSchema: {}, handler: async () => v };`,
    );
    const { tools, files } = await filesOf(dir).reload();
    expect(await byName(tools, "ok").handler({})).toBe("ina/b");
    expect(tools.map((t) => t.name)).toEqual(["ok"]);
    expect(files.find((f) => f.file === "out.ts")?.error).toContain("outside the tools folder");
  });

  test("the packages of its own node_modules, not those of a folder above it", async () => {
    const root = tempDir();
    const dir = join(root, "tools");
    const pkg = (base: string, name: string, value: string): void => {
      const at = join(base, "node_modules", name);
      mkdirSync(at, { recursive: true });
      writeFileSync(join(at, "package.json"), JSON.stringify({ name, main: "index.js" }));
      writeFileSync(join(at, "index.js"), `module.exports = { v: ${JSON.stringify(value)} };`);
    };
    pkg(dir, "inside", "in");
    pkg(root, "above", "above");
    writeFileSync(
      join(dir, "ok.ts"),
      `import { v } from "inside";
export default { name: "ok", description: "d", inputSchema: {}, handler: async () => v };`,
    );
    writeFileSync(
      join(dir, "up.ts"),
      `import { v } from "above";
export default { name: "up", description: "d", inputSchema: {}, handler: async () => v };`,
    );
    const { tools, files } = await filesOf(dir).reload();
    expect(await byName(tools, "ok").handler({})).toBe("in");
    expect(files.find((f) => f.file === "up.ts")?.error).toContain("outside the tools folder");
  });
});

describe("checking a tool", () => {
  const spec = {
    name: "t",
    description: "d",
    inputSchema: { type: "object" },
    handler: () => Promise.resolve("x"),
  };

  test("asks for what the gateway needs", () => {
    expect(typeof checkTool(spec)).toBe("object");
    expect(checkTool(null)).toContain("is an object");
    expect(checkTool({ ...spec, description: " " })).toContain("description");
    expect(checkTool({ ...spec, inputSchema: [] })).toContain("inputSchema");
    expect(checkTool({ ...spec, handler: "nope" })).toContain("handler");
  });

  test("a schema is an object's: one with no type gets it, another type is refused", () => {
    // MCP drops the whole tool list for one schema that is not an object's.
    const bare = checkTool({ ...spec, inputSchema: { properties: { a: { type: "string" } } } });
    expect(typeof bare === "object" && bare.inputSchema).toEqual({
      type: "object",
      properties: { a: { type: "string" } },
    });
    expect(typeof checkTool({ ...spec, inputSchema: {} })).toBe("object");
    expect(checkTool({ ...spec, inputSchema: { type: "string" } })).toContain(
      "inputSchema describes an object",
    );
    expect(checkTool({ ...spec, outputSchema: { type: "array" } })).toContain(
      "outputSchema describes an object",
    );
    expect(checkTool({ ...spec, outputSchema: "x" })).toContain("outputSchema");
    const out = checkTool({ ...spec, outputSchema: { properties: {} } });
    expect(typeof out === "object" && out.outputSchema).toEqual({ type: "object", properties: {} });
  });
});
