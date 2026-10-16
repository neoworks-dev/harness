import { expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { PermissionAsk, RequestPermission } from "../src/agent.ts";
import { createBashTool } from "../src/tools/bash.ts";
import { createCoreTools } from "../src/tools/core.ts";
import { createEditTool } from "../src/tools/edit.ts";
import { createReadTool } from "../src/tools/read.ts";
import { createWriteTool } from "../src/tools/write.ts";
import { errorMessage, tempDir } from "./helpers.ts";

/** A hook that answers `answer` and records what it was asked. */
function hook(answer: boolean): { ask: RequestPermission; asked: PermissionAsk[] } {
  const asked: PermissionAsk[] = [];
  return {
    asked,
    ask: (request) => {
      asked.push(request);
      return Promise.resolve(answer);
    },
  };
}

test("write asks with the diff before writing; allowed writes", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "f.txt"), "old\n");
  const { ask, asked } = hook(true);
  let seenOnDisk = "";
  const spy: RequestPermission = async (request, signal) => {
    seenOnDisk = await readFile(join(cwd, "f.txt"), "utf8");
    return ask(request, signal);
  };
  await createWriteTool(cwd, spy).execute("t1", { path: "f.txt", content: "new\n" });
  expect(seenOnDisk).toBe("old\n");
  expect(asked).toEqual([
    {
      title: "write f.txt",
      kind: "edit",
      toolCallId: "t1",
      content: [{ type: "diff", path: join(cwd, "f.txt"), oldText: "old\n", newText: "new\n" }],
    },
  ]);
  expect(await readFile(join(cwd, "f.txt"), "utf8")).toBe("new\n");
});

test("a refused write fails with a message for the model and writes nothing", async () => {
  const cwd = await tempDir();
  const { ask, asked } = hook(false);
  const message = await errorMessage(
    createWriteTool(cwd, ask).execute("t", { path: "new.txt", content: "x" }),
  );
  expect(message).toBe("The user declined this edit.");
  expect(existsSync(join(cwd, "new.txt"))).toBe(false);
  // A new file has no oldText in the preview.
  expect(asked[0]?.content).toEqual([
    { type: "diff", path: join(cwd, "new.txt"), oldText: undefined, newText: "x" },
  ]);
});

test("edit asks with the computed result; refused leaves the file alone", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "f.txt"), "alpha\nbeta\n");
  const edits = [{ oldText: "beta", newText: "BETA" }];
  const refuse = hook(false);
  const message = await errorMessage(
    createEditTool(cwd, refuse.ask).execute("t", { path: "f.txt", edits }),
  );
  expect(message).toBe("The user declined this edit.");
  expect(await readFile(join(cwd, "f.txt"), "utf8")).toBe("alpha\nbeta\n");
  expect(refuse.asked[0]).toMatchObject({
    title: "edit f.txt",
    kind: "edit",
    content: [{ type: "diff", oldText: "alpha\nbeta\n", newText: "alpha\nBETA\n" }],
  });

  const allow = hook(true);
  await createEditTool(cwd, allow.ask).execute("t", { path: "f.txt", edits });
  expect(await readFile(join(cwd, "f.txt"), "utf8")).toBe("alpha\nBETA\n");
});

test("an edit that fails to match never asks", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "f.txt"), "alpha\n");
  const { ask, asked } = hook(true);
  await errorMessage(
    createEditTool(cwd, ask).execute("t", {
      path: "f.txt",
      edits: [{ oldText: "nope", newText: "x" }],
    }),
  );
  expect(asked).toEqual([]);
});

test("bash asks with the command as the title and no content", async () => {
  const cwd = await tempDir();
  const allow = hook(true);
  await createBashTool(cwd, undefined, allow.ask).execute("t9", { command: "touch made" });
  expect(allow.asked).toEqual([{ title: "touch made", kind: "execute", toolCallId: "t9" }]);
  expect(existsSync(join(cwd, "made"))).toBe(true);

  const refuse = hook(false);
  const message = await errorMessage(
    createBashTool(cwd, undefined, refuse.ask).execute("t", { command: "touch nope" }),
  );
  expect(message).toBe("The user declined to run this command.");
  expect(existsSync(join(cwd, "nope"))).toBe(false);
});

test("the signal reaches the hook, and a cancelled turn does not run the tool", async () => {
  const cwd = await tempDir();
  const controller = new AbortController();
  let received: AbortSignal | undefined;
  const ask: RequestPermission = (_request, signal) => {
    received = signal;
    controller.abort();
    return Promise.resolve(true);
  };
  const message = await errorMessage(
    createBashTool(cwd, undefined, ask).execute("t", { command: "touch late" }, controller.signal),
  );
  expect(received).toBe(controller.signal);
  expect(message).toContain("Cancelled");
  expect(existsSync(join(cwd, "late"))).toBe(false);
});

test("without a hook nothing asks", async () => {
  const cwd = await tempDir();
  await createWriteTool(cwd).execute("t", { path: "a.txt", content: "a" });
  await createBashTool(cwd).execute("t", { command: "touch b" });
  expect(await readFile(join(cwd, "a.txt"), "utf8")).toBe("a");
  expect(existsSync(join(cwd, "b"))).toBe(true);
});

test("read, todo_write and the gateway tools never ask", async () => {
  const cwd = await tempDir();
  await writeFile(join(cwd, "r.txt"), "x");
  const { ask, asked } = hook(false);
  await createReadTool(cwd).execute("t", { path: "r.txt" });
  const tools = createCoreTools(cwd, undefined, undefined, ask);
  const todo = tools.find((tool) => tool.name === "todo_write");
  await todo?.execute("t", { todos: [{ content: "x", status: "pending" }] });
  expect(asked).toEqual([]);
  expect(tools.map((tool) => tool.name)).toContain("bash");
});
