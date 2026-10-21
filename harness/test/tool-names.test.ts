import { expect, test } from "bun:test";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { lineHash } from "../src/tools/hashline.ts";
import { neoToolsFor, toClaudeCall, toNeoArgs, toolResponse } from "../src/tool-names.ts";
import { tempDir } from "./helpers.ts";

test("bash, read and write take Claude Code's names and arguments", async () => {
  const cwd = await tempDir();
  expect(
    await toClaudeCall("bash", { command: "ls", description: "List", timeout: 5 }, cwd),
  ).toEqual({
    name: "Bash",
    names: ["Bash"],
    input: { command: "ls", description: "List", timeout: 5000 },
  });
  expect(await toClaudeCall("read", { path: "a.ts", offset: 3 }, cwd)).toEqual({
    name: "Read",
    names: ["Read"],
    input: { file_path: "a.ts", offset: 3 },
  });
  expect(await toClaudeCall("write", { path: "a.ts", content: "x" }, cwd)).toEqual({
    name: "Write",
    names: ["Write"],
    input: { file_path: "a.ts", content: "x" },
  });
});

test("an edit becomes old_string and new_string, read from the file's lines", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "a.ts"), "one\ntwo\nthree\n");
  const tag = (n: number, text: string): string => `${n}:${lineHash(text)}`;
  const one = await toClaudeCall(
    "edit",
    { path: "a.ts", edits: [{ start: tag(2, "two"), newText: "2" }] },
    cwd,
  );
  expect(one).toEqual({
    name: "Edit",
    names: ["Edit"],
    input: { file_path: "a.ts", old_string: "two", new_string: "2", replace_all: false },
  });
  // Several edits are a MultiEdit, which a matcher for Edit accepts too.
  const many = await toClaudeCall(
    "edit",
    {
      path: "a.ts",
      edits: [
        { start: tag(1, "one"), end: tag(2, "two"), newText: "12" },
        { after: tag(3, "three"), newText: "four" },
      ],
    },
    cwd,
  );
  expect(many.name).toBe("MultiEdit");
  expect(many.names).toEqual(["MultiEdit", "Edit"]);
  expect(many.input).toMatchObject({
    file_path: "a.ts",
    edits: [
      { old_string: "one\ntwo", new_string: "12" },
      { old_string: "three", new_string: "three\nfour" },
    ],
    old_string: "one\ntwo\nthree",
    new_string: "12\nthree\nfour",
  });
});

test("an edit of a file that can't be read still shows the hook the new text", async () => {
  const cwd = await tempDir();
  const call = await toClaudeCall(
    "edit",
    { path: "nope.ts", edits: [{ start: "1:aa", newText: "x" }] },
    cwd,
  );
  expect(call.input).toMatchObject({ file_path: "nope.ts", old_string: "", new_string: "x" });
});

test("the other tools with a counterpart", async () => {
  const cwd = await tempDir();
  expect((await toClaudeCall("skill", { name: "git" }, cwd)).input).toEqual({ skill: "git" });
  expect((await toClaudeCall("web_fetch", { url: "https://x" }, cwd)).name).toBe("WebFetch");
  expect((await toClaudeCall("web_search", { query: "q" }, cwd)).name).toBe("WebSearch");
  expect((await toClaudeCall("todo_write", { todos: [] }, cwd)).name).toBe("TodoWrite");
  const task = await toClaudeCall("delegate", { agent: "reviewer", task: "Look.\nMore." }, cwd);
  expect(task).toMatchObject({
    name: "Task",
    input: { subagent_type: "reviewer", prompt: "Look.\nMore.", description: "Look." },
  });
  const mcp = await toClaudeCall("tools_call", { name: "browser.download", args: { u: 1 } }, cwd);
  expect(mcp).toEqual({
    name: "mcp__browser__download",
    names: ["mcp__browser__download"],
    input: { u: 1 },
  });
});

test("tools without a counterpart keep neo's name and arguments", async () => {
  const call = await toClaudeCall("tools_search", { query: "x" }, "/");
  expect(call).toEqual({ name: "tools_search", names: ["tools_search"], input: { query: "x" } });
});

test("a hook's updatedInput maps back to neo's arguments", () => {
  expect(
    toNeoArgs("bash", { command: "ls -la", timeout: 2000 }, { command: "ls", description: "d" }),
  ).toEqual({ command: "ls -la", description: "d", timeout: 2 });
  expect(toNeoArgs("read", { file_path: "b" }, { path: "a", limit: 4 })).toEqual({
    path: "b",
    limit: 4,
  });
  expect(toNeoArgs("write", { file_path: "b", content: "c" }, { path: "a", content: "" })).toEqual({
    path: "b",
    content: "c",
  });
  // An edit's tags can't be rebuilt from strings.
  expect(toNeoArgs("edit", { old_string: "a" }, {})).toBeUndefined();
});

test("tool_response by tool", () => {
  expect(toolResponse("bash", {}, "out", {})).toMatchObject({ stdout: "out", interrupted: false });
  expect(
    toolResponse("write", { path: "a", content: "c" }, "ok", { path: "/a", newText: "c" }),
  ).toEqual({
    type: "create",
    filePath: "/a",
    content: "c",
  });
  expect(toolResponse("web_search", {}, "r", {})).toEqual({ content: "r" });
});

test("a Claude Code tools list maps to neo's tools", () => {
  expect(neoToolsFor("Bash")).toEqual(["bash"]);
  expect(neoToolsFor(" Read ")).toEqual(["read"]);
  // edit works on read's tags
  expect(neoToolsFor("Edit")).toEqual(["edit", "read"]);
  expect(neoToolsFor("MultiEdit")).toEqual(["edit", "read"]);
  // searching is done in the shell
  expect(neoToolsFor("Grep")).toEqual(["bash"]);
  expect(neoToolsFor("Glob")).toEqual(["bash"]);
  expect(neoToolsFor("TodoWrite")).toEqual(["todo_write"]);
  expect(neoToolsFor("WebFetch")).toEqual(["web_fetch"]);
  expect(neoToolsFor("mcp__browser__download")).toEqual(["tools_search", "tools_call"]);
  // subagent permissions, and tools neo lacks
  expect(neoToolsFor("Agent(claude-security:explore)")).toEqual([]);
  expect(neoToolsFor("Task")).toEqual([]);
  expect(neoToolsFor("NotebookRead")).toEqual([]);
  expect(neoToolsFor("KillShell")).toEqual([]);
  // neo's own names work as they are
  expect(neoToolsFor("web_search")).toEqual(["web_search"]);
});
