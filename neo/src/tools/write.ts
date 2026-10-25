import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { AgentTool } from "../loop/index.ts";
import { Type } from "typebox";
import type { PermissionAsk, RequestPermission } from "../agent.ts";
import { resolveWritablePath } from "./paths.ts";
import { askToProceed, reviewEdit, withNote } from "./permission.ts";

const parameters = Type.Object({
  path: Type.String({
    description: "File to write: absolute, relative to the working directory, or starting with ~.",
  }),
  content: Type.String({ description: "Complete new content of the file." }),
});

/** Details of a changed file; the harness turns them into a diff for views. */
export interface FileChangeDetails {
  path: string;
  /** Absent when the file did not exist. */
  oldText?: string;
  newText: string;
}

async function readIfExists(path: string): Promise<string | undefined> {
  try {
    return await readFile(path, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
}

export function createWriteTool(
  cwd: string,
  ask?: RequestPermission,
): AgentTool<typeof parameters, FileChangeDetails> {
  return {
    name: "write",
    label: "write",
    description:
      "Write a file, replacing its content if it exists and creating missing parent " +
      "directories. Use it for new files or complete rewrites; use edit for targeted changes.",
    parameters,
    async execute(toolCallId, { path, content }, signal) {
      const absolute = resolveWritablePath(cwd, path);
      const oldText = await readIfExists(absolute);
      const request: PermissionAsk = {
        title: `write ${path}`,
        kind: "edit",
        toolCallId,
        content: [{ type: "diff", path: absolute, oldText, newText: content }],
      };
      const proceed = await askToProceed(ask, request, "The user declined this edit.", signal);
      await mkdir(dirname(absolute), { recursive: true });
      await writeFile(absolute, content);
      const note = await reviewEdit(ask, proceed, request, signal);
      const verb = oldText === undefined ? "Created" : "Overwrote";
      const done = `${verb} ${path} (${Buffer.byteLength(content)} bytes).`;
      return {
        content: [{ type: "text", text: withNote(done, note) }],
        details: { path: absolute, oldText, newText: content },
      };
    },
  };
}
