import { Agent, type StreamFn, type ThinkingLevel } from "./loop/index.ts";
import {
  getModels,
  getProviders,
  streamSimple,
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
  providersWithCredential,
  resolveAuth,
  THINKING_LEVELS,
  type HarnessConfig,
} from "./config.ts";
import type { Logger } from "./logger.ts";
import { resolveModel } from "./model.ts";
import { createUpdateMapper } from "./updates.ts";
import { HARNESS_NAME, loggedMessages, restoreMessages } from "./restore.ts";
import { SYSTEM_PROMPT } from "./system-prompt.ts";
import { createCoreTools } from "./tools/core.ts";
import { connectGateway, type GatewayConnection } from "./tools/gateway.ts";
import { loadImages } from "./tools/read.ts";
import { createUsageTracker } from "./usage.ts";

export interface NeoAgentOptions {
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

/** The harness built on our loop: one Agent per session, the session log owns history. */
export function createNeoAgent(options: NeoAgentOptions): HarnessAgent {
  const { config, logger, streamFn } = options;
  const defaultModel = resolveModel(config.provider, config.modelId, logger, config.images);

  /**
   * "provider/id", or a bare id on the configured provider. The id may contain a slash itself
   * (openrouter's): "openrouter/anthropic/claude-haiku-4.5". "head/rest" means provider `head`
   * when it has a model `rest` (what /model lists always has this form); else a bare id with a
   * slash on the configured provider; else an id the registry lacks, on provider `head`. Throws a
   * readable Error for an unknown provider; an unknown id on a known provider is fine (the
   * registry lags).
   */
  function modelFromSetting(value: string): Model<Api> {
    const providers: readonly string[] = getProviders();
    const slash = value.indexOf("/");
    let provider = config.provider;
    let id = value;
    if (slash >= 0) {
      const head = value.slice(0, slash);
      const rest = value.slice(slash + 1);
      const headHasIt =
        providers.includes(head) &&
        getModels(head as KnownProvider).some((candidate) => candidate.id === rest);
      const configuredHasIt = getModels(config.provider as KnownProvider).some(
        (candidate) => candidate.id === value,
      );
      if (headHasIt || !configuredHasIt) {
        provider = head;
        id = rest;
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
    // How the account a request just resolved changes the model (Copilot's base URL).
    const adjustModels = new Map<string, ((model: Model<Api>) => Model<Api>) | undefined>();
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
      getApiKey: async (provider) => {
        const account = await resolveAuth(config, provider);
        adjustModels.set(provider, account?.adjustModel);
        return account?.apiKey;
      },
      sessionId: params.sessionId,
      toolExecution: "sequential",
      streamFn: (model, context, options) =>
        (streamFn ?? streamSimple)(
          adjustModels.get(model.provider)?.(model) ?? model,
          context,
          options,
        ),
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
      try {
        if (!(await resolveAuth(config, provider))) {
          return { stopReason: "error", error: missingApiKeyMessage(config, provider) };
        }
      } catch (error) {
        return {
          stopReason: "error",
          error: error instanceof Error ? error.message : String(error),
        };
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
        logger?.error(`neo agent failed: ${message}`);
        return { stopReason: "error", error: message };
      } finally {
        unsubscribe();
        running = false;
      }
    }

    /**
     * The current model first, then the models of every provider with a credential (the current
     * one's provider first). With several providers a name says whose model it is.
     */
    function modelValues(): { value: string; name: string }[] {
      const current = agent.state.model;
      const currentValue = modelValue(current);
      const providers = [
        current.provider,
        ...providersWithCredential(config)
          .map((credential) => credential.provider)
          .filter((provider) => provider !== current.provider),
      ];
      const values = providers.flatMap((provider) =>
        getModels(provider as KnownProvider).map((model) => ({
          value: modelValue(model),
          name: providers.length > 1 ? `${model.name} · ${provider}` : model.name,
        })),
      );
      const at = values.findIndex((entry) => entry.value === currentValue);
      const known = at >= 0 ? values.splice(at, 1)[0] : undefined;
      values.unshift(known ?? { value: currentValue, name: current.name });
      return values;
    }

    function configOptions(): ConfigOption[] {
      const currentValue = modelValue(agent.state.model);
      return [
        { category: "model", current: currentValue, values: modelValues() },
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
        else throw new Error("neo has no mode setting.");
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
