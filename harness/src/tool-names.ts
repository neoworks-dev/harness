// Claude Code's tool names and arguments, for the things written against them: hooks (matchers
// and the JSON a hook reads) and agent definitions (their `tools:` list). Neo's tools have names
// and arguments of their own; this maps what has a counterpart and leaves the rest as it is.
//
//   neo tool      Claude Code        arguments
//   bash          Bash               command, description, timeout (neo: seconds, Claude: ms)
//   read          Read               path -> file_path, offset, limit
//   write         Write              path -> file_path, content
//   edit          Edit               one edit: file_path, old_string, new_string, replace_all
//   edit          MultiEdit          several: file_path, edits[{old_string, new_string, replace_all}]
//   todo_write    TodoWrite          todos
//   skill         Skill              name -> skill
//   web_fetch     WebFetch           url
//   web_search    WebSearch          query
//   delegate      Task               agent -> subagent_type, task -> prompt (+ description)
//   tools_call    mcp__<app>__<tool> the connector tool's own arguments
//   the rest (tools_search, apps_launch, models, send_message, wait, …) keep neo's names.
//
// Neo's edit names lines by tags from read, so old_string and new_string are what those lines hold
// before and after (read from the file as it is now).
import { readFile } from "node:fs/promises";
import { fileLines } from "./tools/hashline.ts";
import { resolveWritablePath } from "./tools/paths.ts";
import { spliceOf, type Edit } from "./tools/edit.ts";
import { isRecord } from "./util.ts";

/** A neo tool call as Claude Code would have made it. */
export interface ClaudeCall {
  /** The hook's `tool_name`. */
  name: string;
  /** The names a matcher is tested against: `name`, and Edit too for a MultiEdit. */
  names: string[];
  /** The arguments, under Claude Code's names: a hook's `tool_input`. */
  input: Record<string, unknown>;
}

const str = (value: unknown): string | undefined => (typeof value === "string" ? value : undefined);
function list(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  return [];
}

function record(value: unknown): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (isRecord(value)) Object.assign(out, value);
  return out;
}

/** What an `edit` call changes, as Claude Code's old_string/new_string pairs. */
async function editPairs(
  cwd: string,
  path: string,
  edits: Edit[],
): Promise<{ old_string: string; new_string: string; replace_all: boolean }[]> {
  let lines: string[] | undefined;
  try {
    const raw = await readFile(resolveWritablePath(cwd, path), "utf8");
    lines = fileLines(raw.replace(/^﻿/, "").replaceAll("\r\n", "\n"));
  } catch {
    // Unreadable now: the tool itself will say so; the hook still sees the new text.
  }
  return edits.map((edit, i) => {
    const text = edit.newText;
    if (!lines) return { old_string: "", new_string: text, replace_all: false };
    try {
      const splice = spliceOf(edit, i, lines, []);
      if (splice.count > 0) {
        const old = lines.slice(splice.index, splice.index + splice.count).join("\n");
        return { old_string: old, new_string: splice.lines.join("\n"), replace_all: false };
      }
      // An insert: the line it goes after, with the new lines under it.
      let before = "";
      if (splice.index > 0) before = lines[splice.index - 1] ?? "";
      const added = splice.lines.join("\n");
      return {
        old_string: before,
        new_string: before === "" ? added : `${before}\n${added}`,
        replace_all: false,
      };
    } catch {
      return { old_string: "", new_string: text, replace_all: false };
    }
  });
}

/** A neo tool call as the Claude Code tool call a hook is written for. */
export async function toClaudeCall(tool: string, args: unknown, cwd: string): Promise<ClaudeCall> {
  const a = record(args);
  switch (tool) {
    case "bash": {
      const input: Record<string, unknown> = { command: a.command, description: a.description };
      if (typeof a.timeout === "number") input.timeout = Math.round(a.timeout * 1000);
      return { name: "Bash", names: ["Bash"], input };
    }
    case "read": {
      const input: Record<string, unknown> = { file_path: a.path };
      if (a.offset !== undefined) input.offset = a.offset;
      if (a.limit !== undefined) input.limit = a.limit;
      return { name: "Read", names: ["Read"], input };
    }
    case "write":
      return { name: "Write", names: ["Write"], input: { file_path: a.path, content: a.content } };
    case "edit": {
      const path = str(a.path) ?? "";
      const edits = list(a.edits).filter(isRecord);
      const pairs = await editPairs(cwd, path, edits as unknown as Edit[]);
      const [only] = pairs;
      if (pairs.length === 1 && only) {
        return { name: "Edit", names: ["Edit"], input: { file_path: path, ...only } };
      }
      // A hook written for Edit reads old_string and new_string, so they are there too, joined.
      return {
        name: "MultiEdit",
        names: ["MultiEdit", "Edit"],
        input: {
          file_path: path,
          edits: pairs,
          old_string: pairs.map((pair) => pair.old_string).join("\n"),
          new_string: pairs.map((pair) => pair.new_string).join("\n"),
        },
      };
    }
    case "todo_write": {
      return { name: "TodoWrite", names: ["TodoWrite"], input: { todos: a.todos } };
    }
    case "skill":
      return { name: "Skill", names: ["Skill"], input: { skill: a.name } };
    case "web_fetch":
      return { name: "WebFetch", names: ["WebFetch"], input: { url: a.url } };
    case "web_search":
      return {
        name: "WebSearch",
        names: ["WebSearch"],
        input: { query: a.query, ...rest(a, ["query"]) },
      };
    case "delegate": {
      const task = str(a.task) ?? "";
      return {
        name: "Task",
        names: ["Task"],
        input: {
          description: task.split("\n")[0]?.slice(0, 80) ?? "",
          prompt: task,
          subagent_type: a.agent,
        },
      };
    }
    case "tools_call": {
      const name = str(a.name) ?? "";
      const dot = name.indexOf(".");
      const mcp = dot > 0 ? `mcp__${name.slice(0, dot)}__${name.slice(dot + 1)}` : `mcp__${name}`;
      return { name: mcp, names: [mcp], input: record(a.args) };
    }
    default:
      return { name: tool, names: [tool], input: a };
  }
}

function rest(from: Record<string, unknown>, skip: string[]): Record<string, unknown> {
  return Object.fromEntries(Object.entries(from).filter(([key]) => !skip.includes(key)));
}

/**
 * The neo arguments for what a hook returned as `updatedInput` (Claude Code's names), merged into
 * the call's own. undefined when the tool has no mapping back (edit's tags can't be rebuilt from
 * strings, the rest never needed one); the caller says so.
 */
export function toNeoArgs(
  tool: string,
  input: Record<string, unknown>,
  current: Record<string, unknown>,
): Record<string, unknown> | undefined {
  const out = { ...current };
  switch (tool) {
    case "bash":
      if (input.command !== undefined) out.command = input.command;
      if (input.description !== undefined) out.description = input.description;
      if (typeof input.timeout === "number") out.timeout = input.timeout / 1000;
      return out;
    case "read":
      if (input.file_path !== undefined) out.path = input.file_path;
      if (input.offset !== undefined) out.offset = input.offset;
      if (input.limit !== undefined) out.limit = input.limit;
      return out;
    case "write":
      if (input.file_path !== undefined) out.path = input.file_path;
      if (input.content !== undefined) out.content = input.content;
      return out;
    case "web_fetch":
      if (input.url !== undefined) out.url = input.url;
      return out;
    case "web_search":
      if (input.query !== undefined) out.query = input.query;
      return out;
    default:
      return undefined;
  }
}

/** What a finished tool call looks like to a PostToolUse hook (`tool_response`). */
export function toolResponse(
  tool: string,
  args: unknown,
  text: string,
  details: unknown,
): Record<string, unknown> {
  const a = record(args);
  switch (tool) {
    case "bash":
      // Neo gives stdout and stderr as one stream.
      return { stdout: text, stderr: "", output: text, interrupted: false, isImage: false };
    case "read":
      return { type: "text", file: { filePath: record(details).path ?? a.path, content: text } };
    case "write":
      return {
        type: record(details).oldText === undefined ? "create" : "update",
        filePath: record(details).path ?? a.path,
        content: a.content,
      };
    case "edit":
      return { filePath: record(details).path ?? a.path, message: text };
    default:
      return { content: text };
  }
}

/** The neo tools a Claude Code tool name in an agent's `tools:` list stands for. */
const TOOLS_FOR: Record<string, string[]> = {
  bash: ["bash"],
  read: ["read"],
  write: ["write"],
  // edit works on read's tags, so it is useless without read.
  edit: ["edit", "read"],
  multiedit: ["edit", "read"],
  // Neo searches through the shell.
  grep: ["bash"],
  glob: ["bash"],
  ls: ["bash"],
  todowrite: ["todo_write"],
  skill: ["skill"],
  webfetch: ["web_fetch"],
  websearch: ["web_search"],
};

/** Names that mean "bash" only to look around, not to change anything. */
export const SEARCH_ONLY = new Set(["grep", "glob", "ls"]);

/**
 * The neo tool names a Claude Code `tools:` entry grants. `Agent(x, y)`/`Task(...)` entries are
 * subagent permissions, which neo's subagents never use (they can't delegate): none. Connector
 * tools (`mcp__server__tool`) are reached through tools_search and tools_call. A neo tool name
 * works as it is. Unknown names grant nothing.
 */
export function neoToolsFor(entry: string): string[] {
  const name = entry.trim();
  const bare = name.replace(/\(.*$/s, "").toLowerCase();
  if (bare === "agent" || bare === "task") return [];
  if (bare.startsWith("mcp__")) return ["tools_search", "tools_call"];
  const mapped = TOOLS_FOR[bare];
  if (mapped) return mapped;
  const neoNames = [...new Set(Object.values(TOOLS_FOR).flat()), "tools_search", "tools_call"];
  if (!neoNames.includes(bare)) return [];
  return [bare];
}
