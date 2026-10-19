import { tmpdir } from "node:os";
import { join } from "node:path";
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
  FollowUp,
  LoadSessionParams,
  McpServer,
  NewSessionParams,
  PromptParams,
  PromptResult,
} from "./agent.ts";
import { resolveAccount } from "./auth.ts";
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
import {
  bundledOpenRouterModels,
  endpointLabel,
  hostOf,
  OpenRouterCatalog,
  pinnedTo,
  type FetchFn,
  type OpenRouterEndpoint,
} from "./openrouter.ts";
import { createUpdateMapper } from "./updates.ts";
import { HARNESS_NAME, loggedMessages, restoreMessages } from "./restore.ts";
import { SYSTEM_PROMPT } from "./system-prompt.ts";
import { createCoreTools } from "./tools/core.ts";
import type { WebToolsOptions } from "./tools/web.ts";
import { connectGateway, type GatewayConnection } from "./tools/gateway.ts";
import { loadImages } from "./tools/read.ts";
import { createUsageTracker } from "./usage.ts";

export interface NeoAgentOptions {
  config: HarnessConfig;
  logger?: Logger;
  /** Replaces the model call; for tests. */
  streamFn?: StreamFn;
  /** Replaces the network for OpenRouter's model and host lists; for tests. */
  fetch?: FetchFn;
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

const baseValue = (model: Model<Api>): string => `${model.provider}/${model.id}`;

/** "provider/id", plus "@host" when an OpenRouter model is pinned to one. */
function modelValue(model: Model<Api>): string {
  const host = hostOf(model);
  return host === undefined ? baseValue(model) : `${baseValue(model)}@${host}`;
}

const OPENROUTER = "openrouter";

/** One host of the card that follows /model: its value is the model with "@tag". */
function hostOption(
  plain: string,
  endpoint: OpenRouterEndpoint,
): { value: string; name: string; description?: string } {
  const option: { value: string; name: string; description?: string } = {
    value: `${plain}@${endpoint.tag}`,
    name: endpointLabel(endpoint),
  };
  if (endpoint.status !== undefined && endpoint.status < 0) {
    option.description = "Having trouble right now";
  }
  return option;
}

/** A model setting taken apart: "openrouter/deepseek/x@deepinfra/fp8". */
interface ModelSetting {
  provider: string;
  id: string;
  /** OpenRouter's endpoint tag after the last "@". */
  host?: string;
}

/** The daemon's gateway, if it handed us one (it's named "suite"). */
function gatewayOf(servers: McpServer[]): GatewayConnection | undefined {
  const server = servers.find((s) => s.name === "suite") ?? servers[0];
  return server && connectGateway(server);
}

/** The harness built on our loop: one Agent per session, the session log owns history. */
export function createNeoAgent(options: NeoAgentOptions): HarnessAgent {
  const { config, logger, streamFn } = options;
  const catalog = new OpenRouterCatalog({ logger, fetch: options.fetch });
  /** OpenRouter's live models once fetched (never waits for them), else pi-ai's. */
  const openRouterModels = (): Model<Api>[] => catalog.cached() ?? bundledOpenRouterModels();
  const defaultModel = resolveModel(config.provider, config.modelId, logger, config.images);

  /**
   * The web tools' settings for a session: safe search is the user's choice in the daemon
   * (/safesearch), else config.json's. Fetched pages go to a temp directory of its own,
   * which the system clears; TinyFish is searched too when the user has a key (its env var, or a
   * "tinyfish" API key in the suite's accounts file).
   */
  function webToolsFor(params: NewSessionParams): WebToolsOptions {
    const web: WebToolsOptions = {
      config: config.web,
      dir: join(tmpdir(), "neo-web", params.sessionId),
      tinyfishKey: async () =>
        process.env.TINYFISH_API_KEY || (await resolveAccount(config.authPath, "tinyfish"))?.apiKey,
    };
    if (params.safeSearch) web.safeSearch = params.safeSearch;
    return web;
  }

  /** A provider's models: OpenRouter's live list once there is one, else pi-ai's. */
  function modelsOf(provider: string): Model<Api>[] {
    if (provider === OPENROUTER) return openRouterModels();
    const models: Model<Api>[] = getModels(provider as KnownProvider);
    return models;
  }

  /** Whether the provider has a model with this id (OpenRouter: also in its live list). */
  function hasModel(provider: string, id: string): boolean {
    return modelsOf(provider).some((candidate) => candidate.id === id);
  }

  /**
   * "provider/id", or a bare id on the configured provider. The id may contain a slash itself
   * (openrouter's): "openrouter/anthropic/claude-haiku-4.5". "head/rest" means provider `head`
   * when it has a model `rest` (what /model lists always has this form); else a bare id with a
   * slash on the configured provider; else an id the registry lacks, on provider `head`. Throws a
   * readable Error for an unknown provider; an unknown id on a known provider is fine (the
   * registry lags).
   */
  function locate(value: string): { provider: string; id: string } {
    const providers: readonly string[] = getProviders();
    const slash = value.indexOf("/");
    let provider = config.provider;
    let id = value;
    if (slash >= 0) {
      const head = value.slice(0, slash);
      const rest = value.slice(slash + 1);
      const headHasIt = providers.includes(head) && hasModel(head, rest);
      if (headHasIt || !hasModel(config.provider, value)) {
        provider = head;
        id = rest;
      }
    }
    if (!provider || !id) throw new Error(`Model must look like "provider/id", got "${value}".`);
    if (!providers.includes(provider)) throw new Error(`Unknown model provider "${provider}".`);
    return { provider, id };
  }

  /**
   * A setting may end in "@<OpenRouter endpoint tag>" ("…@deepinfra/fp8"; tags have slashes, so
   * the last "@" splits). Not when the whole value is a model that has an "@" in its id.
   */
  function parseSetting(value: string): ModelSetting {
    const at = value.lastIndexOf("@");
    if (at > 0 && at < value.length - 1) {
      const whole = locate(value);
      if (!hasModel(whole.provider, whole.id)) {
        const base = locate(value.slice(0, at));
        const host = value.slice(at + 1);
        if (base.provider !== OPENROUTER) {
          throw new Error(
            `"@${host}" picks an OpenRouter host, but ${base.provider} is not OpenRouter.`,
          );
        }
        return { ...base, host };
      }
      return whole;
    }
    return locate(value);
  }

  function modelFromSetting(value: string): Model<Api> {
    const { provider, id, host } = parseSetting(value);
    const live = provider === OPENROUTER ? openRouterModels() : undefined;
    const model = resolveModel(provider, id, logger, config.images, live);
    return host === undefined ? model : pinnedTo(model, host);
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
          webToolsFor(params),
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
     * one's provider first). With several providers a name says whose model it is. OpenRouter's
     * list is the live one once it has arrived.
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
        modelsOf(provider).map((model) => ({
          value: baseValue(model),
          name: providers.length > 1 ? `${model.name} · ${provider}` : model.name,
        })),
      );
      const host = hostOf(current);
      const at = values.findIndex((entry) => entry.value === baseValue(current));
      // A pinned model keeps its plain entry too: picking that one asks for a host again.
      let known = values[at];
      if (at >= 0 && host === undefined) known = values.splice(at, 1)[0];
      const entry = known ?? { value: baseValue(current), name: current.name };
      values.unshift(
        host === undefined ? entry : { value: currentValue, name: `${entry.name} @ ${host}` },
      );
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

    /** After /model picks an OpenRouter model without a host: which host should run it. */
    async function followUp(
      category: ConfigCategory,
      value: string,
    ): Promise<FollowUp | undefined> {
      if (category !== "model") return undefined;
      let setting: ModelSetting;
      let name: string;
      try {
        setting = parseSetting(value);
        name = modelFromSetting(value).name;
      } catch {
        return undefined;
      }
      if (setting.provider !== OPENROUTER || setting.host !== undefined) return undefined;
      const plain = `${OPENROUTER}/${setting.id}`;
      let endpoints;
      try {
        endpoints = await catalog.endpoints(setting.id);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        return { notice: `Could not list the hosts for ${name} (${reason}); OpenRouter chooses.` };
      }
      if (endpoints.length === 0) {
        return { notice: `OpenRouter lists no hosts for ${name}; it chooses.` };
      }
      return {
        choice: {
          title: `Host for ${name}`,
          options: [
            { value: plain, name: "Any (OpenRouter chooses)" },
            ...endpoints.map((endpoint) => hostOption(plain, endpoint)),
          ],
        },
      };
    }

    // The live model list is fetched in the background; start it before /model asks.
    if (
      initial.model.provider === OPENROUTER ||
      providersWithCredential(config).some((credential) => credential.provider === OPENROUTER)
    ) {
      void catalog.refresh();
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
      followUp,
    };
  }

  return {
    name: HARNESS_NAME,
    newSession: (params) => Promise.resolve(createSession(params, [])),
    loadSession: (params) => Promise.resolve(createSession(params, params.history)),
  };
}
