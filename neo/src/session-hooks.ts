// A neo session's hooks (#23): where in its turn each Claude Code event runs, and what the
// session does with the answer. The hooks themselves (loading, running, reading their answers)
// are hooks.ts.
//
//   SessionStart       before the session's first prompt; its context goes into that prompt
//   UserPromptSubmit   before each prompt; may add context, or drop the prompt
//   PreToolUse         before a tool call, before the permission question; allow and deny replace
//                      it, ask asks (PermissionAsk.hook), and the call's arguments can be rewritten
//   PostToolUse        after a tool call (PostToolUseFailure after a failed one); what it says is
//                      added to the tool's result, where the agent reads it
//   Stop               when the agent would end its turn; may make it go on (SubagentStop in a
//                      subagent)
//   SessionEnd         when the daemon stops
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import type { Message } from "@earendil-works/pi-ai";
import type { AgentUpdate } from "./agent.ts";
import type { HookEvent, HookResult, Hooks } from "./hooks.ts";
import type { AgentOptions } from "./loop/agent.ts";
import { toClaudeCall, toNeoArgs, toolResponse, type ClaudeCall } from "./tool-names.ts";
import { isRecord } from "./util.ts";

/** Where a session's transcript for the hooks is written. */
export function transcriptPathFor(sessionId: string): string {
  return join(tmpdir(), "neo-hooks", `${sessionId.replaceAll(/[^\w.-]/g, "_")}.jsonl`);
}

/** ": reason", or nothing without one. */
export function because(reason: string): string {
  if (reason === "") return "";
  return `: ${reason}`;
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => (isRecord(part) ? part.text : undefined))
    .filter((text): text is string => typeof text === "string" && text !== "")
    .join("\n");
}

/** The conversation as Claude Code writes its transcript: JSON lines, one message each. */
export function transcriptOf(messages: readonly Message[], sessionId: string): string {
  const lines = messages.flatMap((message): string[] => {
    const at = new Date(message.timestamp).toISOString();
    const common = { sessionId, timestamp: at };
    if (message.role === "user") {
      const body = { role: "user", content: textOf(message.content) };
      return [JSON.stringify({ type: "user", message: body, ...common })];
    }
    if (message.role === "toolResult") {
      const block = {
        type: "tool_result",
        tool_use_id: message.toolCallId,
        content: textOf(message.content),
        is_error: message.isError,
      };
      const body = { role: "user", content: [block] };
      return [JSON.stringify({ type: "user", message: body, ...common })];
    }
    if (message.role === "system") return []; // the prompt isn't part of a transcript
    const content = message.content.flatMap((part): unknown[] => {
      if (part.type === "text") return [{ type: "text", text: part.text }];
      if (part.type === "thinking") return [{ type: "thinking", thinking: part.thinking }];
      if (part.type === "toolCall") {
        return [{ type: "tool_use", id: part.id, name: part.name, input: part.arguments }];
      }
      return [];
    });
    const body = { role: "assistant", content, model: message.model };
    return [JSON.stringify({ type: "assistant", message: body, ...common })];
  });
  if (lines.length === 0) return "";
  return `${lines.join("\n")}\n`;
}

export interface SessionHooksOptions {
  hooks: Hooks;
  cwd: string;
  sessionId: string;
  /** In a subagent, Stop runs as SubagentStop. */
  subagent: boolean;
  /** The conversation so far, for the transcript. */
  messages: () => Message[];
  /** Tells the user (a notice in the chat), when a turn is running. */
  emit: () => ((update: AgentUpdate) => void) | undefined;
  /** Where to log what can't be shown (no turn is running). */
  log: (message: string) => void;
  /** A hook said `continue: false`: the turn ends once the current tool call is done. */
  halt: (reason: string) => void;
  /** The running turn's signal. */
  signal: () => AbortSignal | undefined;
}

/** What a UserPromptSubmit hook made of the prompt. */
export interface PromptVerdict {
  /** The prompt is dropped; this is why (for the user). */
  blocked?: string;
  /** For the agent, with the prompt. */
  context: string[];
}

export interface SessionHooks {
  /** Passed to the Agent. */
  beforeToolCall: NonNullable<AgentOptions["beforeToolCall"]>;
  afterToolCall: NonNullable<AgentOptions["afterToolCall"]>;
  /** What PreToolUse decided for a tool call that has yet to ask its question. */
  decision(toolCallId: string | undefined): "allow" | "ask" | undefined;
  sessionStart(source: string): Promise<string[]>;
  userPrompt(prompt: string): Promise<PromptVerdict>;
  /** Stop: what to tell the agent so it goes on, if a hook wants that. */
  stop(active: boolean, lastMessage: string): Promise<{ continueWith?: string; halted?: string }>;
  end(reason: string): Promise<void>;
}

export function createSessionHooks(options: SessionHooksOptions): SessionHooks {
  const { hooks, cwd } = options;
  /** The Claude Code view of a call, made before it ran (an edit can't be mapped once written). */
  const calls = new Map<string, ClaudeCall>();
  const decisions = new Map<string, "allow" | "ask">();

  function notice(text: string, tone?: "danger"): void {
    const emit = options.emit();
    if (!emit) {
      options.log(text);
      return;
    }
    emit(tone ? { type: "notice", text, tone } : { type: "notice", text });
  }

  /** What the user should see of an answer: hooks that failed, and their own messages. */
  function show(result: HookResult): void {
    for (const error of result.errors) notice(error, "danger");
    for (const message of result.messages) notice(message);
  }

  const wanted = (...events: HookEvent[]): boolean => events.some((event) => hooks.has(event));

  return {
    decision: (id) => (id === undefined ? undefined : decisions.get(id)),

    async beforeToolCall({ toolCall, args }, signal) {
      if (!wanted("PreToolUse", "PostToolUse", "PostToolUseFailure")) return undefined;
      const call = await toClaudeCall(toolCall.name, args, cwd);
      calls.set(toolCall.id, call);
      if (!hooks.has("PreToolUse")) return undefined;
      const result = await hooks.run(
        "PreToolUse",
        { tool_name: call.name, tool_input: call.input, tool_use_id: toolCall.id },
        { matchValue: call.names, tool: call, signal },
      );
      show(result);
      if (result.halt !== undefined) {
        options.halt(result.halt);
        const stopped = `A hook stopped the turn${because(result.halt)}`;
        notice(stopped, "danger");
        return { block: true, reason: stopped };
      }
      if (result.blocked !== undefined || result.decision === "deny") {
        let why = result.blocked ?? result.decisionReason;
        if (why === undefined) why = result.messages.join("\n") || "no reason given";
        return { block: true, reason: `Blocked by a hook: ${why}` };
      }
      if (result.updatedInput) {
        const next = isRecord(args)
          ? toNeoArgs(toolCall.name, result.updatedInput, args)
          : undefined;
        if (next && isRecord(args)) Object.assign(args, next);
        else notice(`A hook rewrote the arguments of ${toolCall.name}, which can't be applied.`);
      }
      if (result.decision === "allow" || result.decision === "ask") {
        decisions.set(toolCall.id, result.decision);
        if (result.decisionReason) {
          const what = result.decision === "allow" ? "allowed" : "asks about";
          notice(`A hook ${what} ${toolCall.name}: ${result.decisionReason}`);
        }
      }
      return undefined;
    },

    async afterToolCall({ toolCall, args, result, isError }, signal) {
      const call = calls.get(toolCall.id);
      calls.delete(toolCall.id);
      decisions.delete(toolCall.id);
      const event: HookEvent = isError ? "PostToolUseFailure" : "PostToolUse";
      if (!call || !hooks.has(event)) return undefined;
      const text = textOf(result.content);
      const fields: Record<string, unknown> = {
        tool_name: call.name,
        tool_input: call.input,
        tool_use_id: toolCall.id,
      };
      if (isError) fields.error = text;
      else fields.tool_response = toolResponse(toolCall.name, args, text, result.details);
      const answer = await hooks.run(event, fields, {
        matchValue: call.names,
        tool: call,
        signal,
      });
      show(answer);
      if (answer.halt !== undefined) {
        options.halt(answer.halt);
        notice(`A hook stopped the turn${because(answer.halt)}`, "danger");
      }
      // The agent reads this with the result.
      const extra: string[] = [];
      if (answer.blocked !== undefined) extra.push(`Hook feedback: ${answer.blocked}`);
      for (const context of answer.context) extra.push(`Hook context: ${context}`);
      if (extra.length === 0) return undefined;
      return {
        content: [...result.content, { type: "text", text: `\n\n${extra.join("\n\n")}` }],
      };
    },

    async sessionStart(source) {
      if (!hooks.has("SessionStart")) return [];
      const result = await hooks.run(
        "SessionStart",
        { source },
        { matchValue: source, signal: options.signal() },
      );
      show(result);
      return result.context;
    },

    async userPrompt(prompt) {
      if (!hooks.has("UserPromptSubmit")) return { context: [] };
      const result = await hooks.run("UserPromptSubmit", { prompt }, { signal: options.signal() });
      show(result);
      const verdict: PromptVerdict = { context: result.context };
      if (result.halt !== undefined) verdict.blocked = result.halt || "A hook stopped this prompt.";
      else if (result.blocked !== undefined) verdict.blocked = result.blocked;
      return verdict;
    },

    async stop(active, lastMessage) {
      const event: HookEvent = options.subagent ? "SubagentStop" : "Stop";
      if (!hooks.has(event)) return {};
      const result = await hooks.run(
        event,
        { stop_hook_active: active, last_assistant_message: lastMessage },
        { signal: options.signal() },
      );
      show(result);
      if (result.halt !== undefined) return { halted: result.halt };
      if (result.blocked !== undefined) return { continueWith: result.blocked };
      return {};
    },

    async end(reason) {
      if (!hooks.has("SessionEnd")) return;
      const result = await hooks.run("SessionEnd", { reason }, { matchValue: reason });
      for (const error of result.errors) options.log(error);
    },
  };
}

/** Writes the transcript hooks read; before every event but the tool ones, which are many. */
export function writeTranscript(path: string, text: string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
}
