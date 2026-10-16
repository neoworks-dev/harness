import { Agent, type StreamFn, type ThinkingLevel } from "@mariozechner/pi-agent-core";
import {
  getModels,
  getProviders,
  type Api,
  type AssistantMessage,
  type KnownProvider,
  type Model,
} from "@mariozechner/pi-ai";
import type {
  Agent as HarnessAgent,
  AgentSession,
  ConfigCategory,
  ConfigOption,
  LoadSessionParams,
  McpServer,
  NewSessionParams,
  PromptParams,
  PromptResult,
} from "./agent.ts";
import { formatPrompt } from "./context-events.ts";
import {
  missingApiKeyMessage,
  resolveApiKey,
  THINKING_LEVELS,
  type HarnessConfig,
} from "./config.ts";
import type { Logger } from "./logger.ts";
import { resolveModel } from "./model.ts";
import { createUpdateMapper } from "./pi-updates.ts";
import { HARNESS_NAME, loggedMessages, restoreMessages } from "./restore.ts";
import { SYSTEM_PROMPT } from "./system-prompt.ts";
import { createCoreTools } from "./tools/core.ts";
import { connectGateway, type GatewayConnection } from "./tools/gateway.ts";
import { loadImages } from "./tools/read.ts";
import { createUsageTracker } from "./usage.ts";

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

function parseThinkingLevel(value: string): ThinkingLevel {
  const level = THINKING_LEVELS.find((candidate) => candidate === value);
  if (level) return level;
  throw new Error(`Unknown thinking level "${value}"; use one of ${THINKING_LEVELS.join(", ")}.`);
}

const modelValue = (model: Model<Api>): string => `${model.provider}/${model.id}`;

/** The daemon's gateway, if it handed us one (it's named "suite"). */
function gatewayOf(servers: McpServer[]): GatewayConnection | undefined {
  const server = servers.find((s) => s.name === "suite") ?? servers[0];
  return server && connectGateway(server);
}

/** The harness built on pi-agent-core: one pi Agent per session, the session log owns history. */
export function createPiAgent(options: PiAgentOptions): HarnessAgent {
  const { config, logger, streamFn } = options;
  const defaultModel = resolveModel(config.provider, config.modelId, logger, config.images);

  /**
   * "provider/id", or a bare id on the configured provider. Throws a readable Error for an
   * unknown provider; an unknown id on a known provider is fine (the registry lags).
   */
  function modelFromSetting(value: string): Model<Api> {
    const providers: readonly string[] = getProviders();
    const slash = value.indexOf("/");
    let provider = config.provider;
    let id = value;
    if (slash >= 0) {
      const head = value.slice(0, slash);
      const isConfiguredModel = getModels(config.provider as KnownProvider).some(
        (candidate) => candidate.id === value,
      );
      // An id that itself contains a slash (openrouter) on the configured provider.
      if (!isConfiguredModel) {
        provider = head;
        id = value.slice(slash + 1);
      }
    }
    if (!provider || !id) throw new Error(`Model must look like "provider/id", got "${value}".`);
    if (!providers.includes(provider)) throw new Error(`Unknown model provider "${provider}".`);
    return resolveModel(provider, id, logger, config.images);
  }

  /** The model and thinking level a new session starts with: its settings, else the defaults. */
  function initialSettings(params: NewSessionParams): {
    model: Model<Api>;
    thinkingLevel: ThinkingLevel;
  } {
    let model = defaultModel;
    let thinkingLevel = config.thinkingLevel;
    const { model: modelSetting, thought_level: levelSetting } = params.settings ?? {};
    if (modelSetting !== undefined) {
      try {
        model = modelFromSetting(modelSetting);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        logger?.warn(`Ignoring stored model setting "${modelSetting}": ${reason}`);
      }
    }
    if (levelSetting !== undefined) {
      try {
        thinkingLevel = parseThinkingLevel(levelSetting);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        logger?.warn(`Ignoring stored thinking level setting "${levelSetting}": ${reason}`);
      }
    }
    return { model, thinkingLevel };
  }

  function createSession(
    params: NewSessionParams,
    history: LoadSessionParams["history"],
  ): AgentSession {
    const messages = restoreMessages(history, logger);
    const initial = initialSettings(params);
    // Told once per session, in its first prompt (older logs: the first prompt after loading).
    let cwdTold = JSON.stringify(messages).includes("<session cwd=");
    const agent = new Agent({
      initialState: {
        systemPrompt: SYSTEM_PROMPT,
        model: initial.model,
        tools: createCoreTools(
          params.cwd,
          gatewayOf(params.mcpServers),
          params.resolveAsset,
          params.requestPermission,
        ),
        messages,
        thinkingLevel: initial.thinkingLevel,
      },
      getApiKey: (provider) => resolveApiKey(config, provider),
      sessionId: params.sessionId,
      toolExecution: "sequential",
      streamFn,
    });
    // Counts every message ever logged, including failed ones the context no longer holds.
    const usage = createUsageTracker(
      loggedMessages(history, logger),
      () => agent.state.model.contextWindow,
    );
    let running = false;
    let cancelRequested = false;

    async function prompt({
      text,
      images,
      pendingEvents,
      summary,
      onUpdate,
    }: PromptParams): Promise<PromptResult> {
      if (running) return { stopReason: "error", error: "A turn is already running." };
      const provider = agent.state.model.provider;
      if (!resolveApiKey(config, provider)) {
        return { stopReason: "error", error: missingApiKeyMessage(config, provider) };
      }
      running = true;
      cancelRequested = false;
      const mapEvent = createUpdateMapper(onUpdate, usage);
      let final: AssistantMessage | undefined;
      const unsubscribe = agent.subscribe((event) => {
        mapEvent(event);
        if (event.type !== "agent_end") return;
        for (const message of event.messages) if (message.role === "assistant") final = message;
      });
      try {
        const cwd = cwdTold ? undefined : params.cwd;
        cwdTold = true;
        await agent.prompt(
          formatPrompt(text, pendingEvents, cwd, summary),
          await loadImages(images ?? []),
        );
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

    function configOptions(): ConfigOption[] {
      const current = agent.state.model;
      const currentValue = modelValue(current);
      const models: Model<Api>[] = getModels(current.provider as KnownProvider);
      const values = models.map((model) => ({ value: modelValue(model), name: model.name }));
      if (!values.some((entry) => entry.value === currentValue)) {
        values.unshift({ value: currentValue, name: current.name });
      }
      return [
        { category: "model", current: currentValue, values },
        {
          category: "thought_level",
          current: agent.state.thinkingLevel,
          values: THINKING_LEVELS.map((level) => ({ value: level, name: level })),
        },
      ];
    }

    function setConfigOption(category: ConfigCategory, value: string): Promise<void> {
      try {
        if (category === "model") agent.state.model = modelFromSetting(value);
        else if (category === "thought_level")
          agent.state.thinkingLevel = parseThinkingLevel(value);
        else throw new Error("pi has no mode setting.");
        return Promise.resolve();
      } catch (error) {
        return Promise.reject(error instanceof Error ? error : new Error(String(error)));
      }
    }

    return {
      prompt,
      cancel() {
        if (!running) return;
        cancelRequested = true;
        agent.abort();
      },
      configOptions,
      setConfigOption,
    };
  }

  return {
    name: HARNESS_NAME,
    newSession: (params) => Promise.resolve(createSession(params, [])),
    loadSession: (params) => Promise.resolve(createSession(params, params.history)),
  };
}
