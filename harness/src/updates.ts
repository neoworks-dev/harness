import type { AgentEvent } from "./loop/index.ts";
import type { ToolCallContent, ToolKind } from "@suite/protocol/types";
import type { AgentUpdate } from "./agent.ts";
import { HARNESS_NAME } from "./restore.ts";
import { parseTodos } from "./tools/todo.ts";
import { tailBytes } from "./tools/truncate.ts";
import type { UsageTracker } from "./usage.ts";
import { isRecord } from "./util.ts";

// Live output shown while a tool runs; the final result carries what the model got (for web
// tools with a shorter first line, see shownText).
const LIVE_OUTPUT_BYTES = 8 * 1024;
const TITLE_COMMAND_CHARS = 80;

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function stringArg(args: unknown, key: string): string {
  return (isRecord(args) ? text(args[key]) : undefined) ?? "";
}

function describeToolCall(name: string, args: unknown): { title: string; kind: ToolKind } {
  switch (name) {
    case "bash": {
      const firstLine = stringArg(args, "command").split("\n")[0] ?? "";
      return { title: `bash: ${firstLine.slice(0, TITLE_COMMAND_CHARS)}`, kind: "execute" };
    }
    case "read":
      return { title: `read ${stringArg(args, "path")}`, kind: "read" };
    case "write":
    case "edit":
      return { title: `${name} ${stringArg(args, "path")}`, kind: "edit" };
    case "tools_search": {
      const app = stringArg(args, "app");
      let title = `search tools: ${stringArg(args, "query")}`;
      if (app) title += ` in ${app}`;
      return { title, kind: "search" };
    }
    case "tools_call":
      return { title: stringArg(args, "name"), kind: "other" };
    case "apps_launch":
      return { title: `launch ${stringArg(args, "app")}`, kind: "other" };
    case "apps_focus":
      return { title: `focus ${stringArg(args, "app")}`, kind: "other" };
    case "web_search": {
      const kind = stringArg(args, "kind");
      const what = kind === "news" || kind === "images" ? kind : "the web";
      return { title: `search ${what}: ${stringArg(args, "query")}`, kind: "search" };
    }
    case "web_fetch":
      return { title: `fetch ${stringArg(args, "url")}`, kind: "fetch" };
    case "models": {
      const agent = stringArg(args, "agent");
      return { title: agent ? `models of ${agent}` : "models of the harnesses", kind: "search" };
    }
    case "send_message": {
      const first = stringArg(args, "text").split("\n")[0] ?? "";
      return {
        title: `message ${stringArg(args, "to")}: ${first.slice(0, TITLE_COMMAND_CHARS)}`,
        kind: "other",
      };
    }
    case "wait":
      return { title: "wait for messages", kind: "other" };
    case "todo_write":
      return { title: "update todos", kind: "think" };
    case "delegate": {
      const task = stringArg(args, "task").split("\n")[0] ?? "";
      return {
        title: `${stringArg(args, "agent")} · ${task.slice(0, TITLE_COMMAND_CHARS)}`,
        kind: "think",
      };
    }
    default:
      return { title: name, kind: "other" };
  }
}

/** Text of tool result content; images are mentioned unless they are shown as image content. */
function contentText(content: unknown, imagesShown = false): string {
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const part of content) {
    if (!isRecord(part)) continue;
    if (part.type === "text" && typeof part.text === "string") parts.push(part.text);
    else if (part.type === "image" && !imagesShown) parts.push("[image]");
  }
  return parts.join("\n");
}

/** An image result as a picture for the view: the file it was read from (read's details.path). */
function imagesOf(content: unknown, details: unknown): ToolCallContent[] {
  if (!Array.isArray(content) || !isRecord(details) || typeof details.path !== "string") return [];
  const image = content.find((part) => isRecord(part) && part.type === "image");
  if (!isRecord(image)) return [];
  const out: ToolCallContent = { type: "image", src: details.path };
  if (typeof image.mimeType === "string") out.mimeType = image.mimeType;
  return [out];
}

/**
 * What the chat shows of a result whose first line says little at a glance: a fetch's `Saved
 * <url> as markdown: <path> (N lines).` (the header has the URL already) becomes `N lines ·
 * title`, a search gets its count on top. The rest stays, for the expanded view.
 */
function shownText(toolName: string, output: string, details: unknown): string {
  if (!isRecord(details)) return output;
  if (toolName === "web_fetch" && typeof details.lines === "number") {
    const parts = [`${details.lines} lines`];
    const title = text(details.title);
    const file = text(details.path)?.split("/").pop();
    if (title) parts.push(title);
    else if (file) parts.push(file);
    const lines = [parts.join(" · "), ...output.split("\n").slice(1)];
    return lines.join("\n").trimEnd();
  }
  if (toolName === "web_search" && Array.isArray(details.results) && details.results.length > 0) {
    const n = details.results.length;
    const noun = details.kind === "images" ? "image" : "result";
    return `${n} ${n === 1 ? noun : `${noun}s`}\n\n${output}`;
  }
  return output;
}

function diffOf(toolName: string, details: unknown): ToolCallContent[] {
  if (toolName !== "write" && toolName !== "edit") return [];
  if (!isRecord(details) || typeof details.path !== "string") return [];
  if (typeof details.newText !== "string") return [];
  const oldText = text(details.oldText);
  return [{ type: "diff", path: details.path, oldText, newText: details.newText }];
}

/**
 * Translates neo events into AgentUpdates for one prompt. Stateful because deltas need the id of
 * the assistant message they belong to. With a usage tracker, each assistant message is followed
 * by a usage update.
 */
export function createUpdateMapper(
  emit: (update: AgentUpdate) => void,
  usage?: UsageTracker,
): (event: AgentEvent) => void {
  let messageId = "";

  return (event) => {
    switch (event.type) {
      case "message_start":
        if (event.message.role === "assistant") messageId = crypto.randomUUID();
        break;
      case "message_update": {
        const ev = event.assistantMessageEvent;
        if (ev.type === "text_delta" && ev.delta) {
          emit({ type: "agent_message_chunk", messageId, text: ev.delta });
        } else if (ev.type === "thinking_delta" && ev.delta) {
          emit({ type: "agent_thought_chunk", messageId, text: ev.delta });
        } else if (ev.type === "toolcall_end") {
          const { id, name, arguments: rawInput } = ev.toolCall;
          const { title, kind } = describeToolCall(name, rawInput);
          // tools_call results render with the connector tool's template, so name that tool.
          const tool = name === "tools_call" ? stringArg(rawInput, "name") || name : name;
          emit({
            type: "tool_call",
            toolCallId: id,
            tool,
            title,
            kind,
            status: "pending",
            rawInput,
          });
          // The plan shows as soon as the model has written it, not when the tool has run.
          const todos = name === "todo_write" ? parseTodos(rawInput) : undefined;
          if (todos) emit({ type: "plan", entries: todos });
        }
        break;
      }
      case "message_end":
        emit({ type: "harness_message", harness: HARNESS_NAME, message: event.message });
        if (event.message.role === "assistant") {
          const update = usage?.add(event.message);
          if (update) emit(update);
        }
        break;
      case "tool_execution_start":
        emit({ type: "tool_call_update", toolCallId: event.toolCallId, status: "in_progress" });
        break;
      case "tool_execution_update": {
        const output = tailBytes(contentText(event.partialResult?.content), LIVE_OUTPUT_BYTES);
        emit({
          type: "tool_call_update",
          toolCallId: event.toolCallId,
          content: [{ type: "text", text: output }],
        });
        break;
      }
      case "tool_execution_end": {
        const images: ToolCallContent[] = [];
        if (!event.isError) images.push(...imagesOf(event.result?.content, event.result?.details));
        let output = contentText(event.result?.content, images.length > 0);
        if (!event.isError) output = shownText(event.toolName, output, event.result?.details);
        const content: ToolCallContent[] = [{ type: "text", text: output }];
        if (!event.isError) content.push(...diffOf(event.toolName, event.result?.details));
        content.push(...images);
        emit({
          type: "tool_call_update",
          toolCallId: event.toolCallId,
          status: event.isError ? "failed" : "completed",
          content,
          rawOutput: event.result?.details,
        });
        break;
      }
      default:
        break;
    }
  };
}
