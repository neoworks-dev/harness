import type { AgentEvent } from "@mariozechner/pi-agent-core";
import type { ToolCallContent, ToolKind } from "@suite/protocol/types";
import type { AgentUpdate } from "./agent.ts";
import { HARNESS_NAME } from "./restore.ts";
import { tailBytes } from "./tools/truncate.ts";
import { isRecord } from "./util.ts";

// Live output shown while a tool runs; the final result carries what the model got.
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
    default:
      return { title: name, kind: "other" };
  }
}

/** Text of tool result content; images are only mentioned. */
function contentText(content: unknown): string {
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const part of content) {
    if (!isRecord(part)) continue;
    if (part.type === "text" && typeof part.text === "string") parts.push(part.text);
    else if (part.type === "image") parts.push("[image]");
  }
  return parts.join("\n");
}

function diffOf(toolName: string, details: unknown): ToolCallContent[] {
  if (toolName !== "write" && toolName !== "edit") return [];
  if (!isRecord(details) || typeof details.path !== "string") return [];
  if (typeof details.newText !== "string") return [];
  const oldText = text(details.oldText);
  return [{ type: "diff", path: details.path, oldText, newText: details.newText }];
}

/**
 * Translates pi events into AgentUpdates for one prompt. Stateful because deltas need the id of
 * the assistant message they belong to.
 */
export function createUpdateMapper(
  emit: (update: AgentUpdate) => void,
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
          emit({ type: "tool_call", toolCallId: id, title, kind, status: "pending", rawInput });
        }
        break;
      }
      case "message_end":
        emit({ type: "harness_message", harness: HARNESS_NAME, message: event.message });
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
        const output = contentText(event.result?.content);
        const content: ToolCallContent[] = [{ type: "text", text: output }];
        if (!event.isError) content.push(...diffOf(event.toolName, event.result?.details));
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
