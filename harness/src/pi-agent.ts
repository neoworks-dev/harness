import { Agent, type StreamFn } from "@mariozechner/pi-agent-core";
import type { AssistantMessage } from "@mariozechner/pi-ai";
import type {
  Agent as HarnessAgent,
  AgentSession,
  LoadSessionParams,
  McpServer,
  NewSessionParams,
  PromptParams,
  PromptResult,
} from "./agent.ts";
import { formatPrompt } from "./context-events.ts";
import { missingApiKeyMessage, resolveApiKey, type HarnessConfig } from "./config.ts";
import type { Logger } from "./logger.ts";
import { resolveModel } from "./model.ts";
import { createUpdateMapper } from "./pi-updates.ts";
import { HARNESS_NAME, restoreMessages } from "./restore.ts";
import { SYSTEM_PROMPT } from "./system-prompt.ts";
import { createCoreTools } from "./tools/core.ts";
import { connectGateway, type GatewayConnection } from "./tools/gateway.ts";
import { loadImages } from "./tools/read.ts";

export interface PiAgentOptions {
  config: HarnessConfig;
  logger?: Logger;
  /** Replaces the model call; for tests. */
  streamFn?: StreamFn;
}

function toResult(final: AssistantMessage | undefined, cancelled: boolean): PromptResult {
  if (!final) return { stopReason: "error", error: "The agent ended without a response." };
  if (final.stopReason === "error") {
    return { stopReason: "error", error: final.errorMessage || "The model request failed." };
  }
  if (cancelled || final.stopReason === "aborted") return { stopReason: "cancelled" };
  if (final.stopReason === "length") return { stopReason: "max_tokens" };
  return { stopReason: "end_turn" };
}

/** The daemon's gateway, if it handed us one (it's named "suite"). */
function gatewayOf(servers: McpServer[]): GatewayConnection | undefined {
  const server = servers.find((s) => s.name === "suite") ?? servers[0];
  return server && connectGateway(server);
}

/** The harness built on pi-agent-core: one pi Agent per session, the session log owns history. */
export function createPiAgent(options: PiAgentOptions): HarnessAgent {
  const { config, logger, streamFn } = options;
  const model = resolveModel(config.provider, config.modelId, logger);

  function createSession(
    params: NewSessionParams,
    history: LoadSessionParams["history"],
  ): AgentSession {
    const messages = restoreMessages(history, logger);
    // Told once per session, in its first prompt (older logs: the first prompt after loading).
    let cwdTold = JSON.stringify(messages).includes("<session cwd=");
    const agent = new Agent({
      initialState: {
        systemPrompt: SYSTEM_PROMPT,
        model,
        tools: createCoreTools(params.cwd, gatewayOf(params.mcpServers), params.resolveAsset),
        messages,
        thinkingLevel: config.thinkingLevel,
      },
      getApiKey: () => resolveApiKey(config),
      sessionId: params.sessionId,
      toolExecution: "sequential",
      streamFn,
    });
    let running = false;
    let cancelRequested = false;

    async function prompt({
      text,
      images,
      pendingEvents,
      onUpdate,
    }: PromptParams): Promise<PromptResult> {
      if (running) return { stopReason: "error", error: "A turn is already running." };
      if (!resolveApiKey(config)) {
        return { stopReason: "error", error: missingApiKeyMessage(config) };
      }
      running = true;
      cancelRequested = false;
      const mapEvent = createUpdateMapper(onUpdate);
      let final: AssistantMessage | undefined;
      const unsubscribe = agent.subscribe((event) => {
        mapEvent(event);
        if (event.type !== "agent_end") return;
        for (const message of event.messages) if (message.role === "assistant") final = message;
      });
      try {
        const cwd = cwdTold ? undefined : params.cwd;
        cwdTold = true;
        await agent.prompt(formatPrompt(text, pendingEvents, cwd), await loadImages(images ?? []));
        return toResult(final, cancelRequested);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        logger?.error(`pi agent failed: ${message}`);
        return { stopReason: "error", error: message };
      } finally {
        unsubscribe();
        running = false;
      }
    }

    return {
      prompt,
      cancel() {
        if (!running) return;
        cancelRequested = true;
        agent.abort();
      },
    };
  }

  return {
    name: HARNESS_NAME,
    newSession: (params) => Promise.resolve(createSession(params, [])),
    loadSession: (params) => Promise.resolve(createSession(params, params.history)),
  };
}
