import type { StreamFn } from "../src/loop/index.ts";
import {
  createAssistantMessageEventStream,
  fauxAssistantMessage,
  type AssistantMessage,
  type AssistantMessageEventStream,
  type Context,
  type Message,
} from "@mariozechner/pi-ai";
import type { HarnessConfig } from "../src/config.ts";

export const TEST_CONFIG: HarnessConfig = {
  provider: "deepseek",
  modelId: "deepseek-v4-flash",
  thinkingLevel: "off",
  apiKey: "test-key-not-real",
  authPath: "/nonexistent/suite-auth.json",
  piAuthPath: "/nonexistent/auth.json",
};

export type Step =
  AssistantMessage | ((context: Context, signal?: AbortSignal) => AssistantMessage);

export function reply(
  content: Parameters<typeof fauxAssistantMessage>[0],
  stopReason: AssistantMessage["stopReason"] = "stop",
  errorMessage?: string,
): AssistantMessage {
  return fauxAssistantMessage(content, { stopReason, errorMessage });
}

/** A reply as a provider reports it: with token counts and the cost of the request. */
export function withUsage(
  message: AssistantMessage,
  tokens: { input: number; output: number; cacheRead?: number; cacheWrite?: number },
  cost: number,
): AssistantMessage {
  const { input, output, cacheRead = 0, cacheWrite = 0 } = tokens;
  const usage = {
    input,
    output,
    cacheRead,
    cacheWrite,
    totalTokens: input + output + cacheRead + cacheWrite,
    cost: { input: cost, output: 0, cacheRead: 0, cacheWrite: 0, total: cost },
  };
  return { ...message, usage };
}

function streamMessage(message: AssistantMessage): AssistantMessageEventStream {
  const stream = createAssistantMessageEventStream();
  const empty: AssistantMessage = { ...message, content: [] };
  queueMicrotask(() => {
    stream.push({ type: "start", partial: empty });
    message.content.forEach((block, contentIndex) => {
      const partial = { ...message, content: message.content.slice(0, contentIndex + 1) };
      if (block.type === "text") {
        stream.push({ type: "text_start", contentIndex, partial });
        const half = Math.ceil(block.text.length / 2);
        stream.push({
          type: "text_delta",
          contentIndex,
          delta: block.text.slice(0, half),
          partial,
        });
        stream.push({ type: "text_delta", contentIndex, delta: block.text.slice(half), partial });
        stream.push({ type: "text_end", contentIndex, content: block.text, partial });
      } else if (block.type === "thinking") {
        stream.push({ type: "thinking_start", contentIndex, partial });
        stream.push({ type: "thinking_delta", contentIndex, delta: block.thinking, partial });
        stream.push({ type: "thinking_end", contentIndex, content: block.thinking, partial });
      } else {
        stream.push({ type: "toolcall_start", contentIndex, partial });
        const delta = JSON.stringify(block.arguments);
        stream.push({ type: "toolcall_delta", contentIndex, delta, partial });
        stream.push({ type: "toolcall_end", contentIndex, toolCall: block, partial });
      }
    });
    if (message.stopReason === "error" || message.stopReason === "aborted") {
      stream.push({ type: "error", reason: message.stopReason, error: message });
    } else {
      stream.push({ type: "done", reason: message.stopReason, message });
    }
  });
  return stream;
}

export interface FakeModel {
  streamFn: StreamFn;
  /** The messages sent to the model on each request. */
  requests: Message[][];
  /** The model each request was made with. */
  models: { provider: string; id: string }[];
  /** The thinking level (reasoning option) of each request. */
  reasoning: (string | undefined)[];
}

/** A model that answers request N with step N. */
export function fakeModel(steps: Step[]): FakeModel {
  const requests: Message[][] = [];
  const models: FakeModel["models"] = [];
  const reasoning: FakeModel["reasoning"] = [];
  const streamFn: StreamFn = (model, context, options) => {
    requests.push(structuredClone(context.messages));
    models.push({ provider: model.provider, id: model.id });
    reasoning.push(options?.reasoning);
    const step = steps[requests.length - 1];
    if (!step) throw new Error(`unexpected model request #${requests.length}`);
    const message = typeof step === "function" ? step(context, options?.signal) : step;
    return streamMessage(message);
  };
  return { streamFn, requests, models, reasoning };
}

/** A model that never answers until the request is aborted. */
export function hangingModel(): FakeModel {
  const requests: Message[][] = [];
  const streamFn: StreamFn = (_model, context, options) => {
    requests.push(structuredClone(context.messages));
    const stream = createAssistantMessageEventStream();
    options?.signal?.addEventListener("abort", () => {
      const message = reply("", "aborted", "Request was aborted");
      stream.push({ type: "error", reason: "aborted", error: message });
    });
    return stream;
  };
  return { streamFn, requests, models: [], reasoning: [] };
}
