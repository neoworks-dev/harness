// OpenRouter's public catalog: the live model list (pi-ai's bundled one lags) and the hosts
// ("endpoints") a model runs on. Neither endpoint needs a key.
import { getModels, type Api, type Model } from "@mariozechner/pi-ai";
import type { Logger } from "./logger.ts";
import { isRecord } from "./util.ts";

const BASE_URL = "https://openrouter.ai/api/v1";
const PROVIDER = "openrouter";
/** How long a fetched list is used before the next background refresh. */
const TTL_MS = 60 * 60 * 1000;
/** After a failed fetch the next try waits this long (the old list, or pi-ai's, stays). */
const RETRY_MS = 60 * 1000;
const TIMEOUT_MS = 10_000;

export type FetchFn = (url: string, init?: { signal?: AbortSignal }) => Promise<Response>;

export interface OpenRouterEndpoint {
  /** "DeepInfra". */
  provider: string;
  /** What `provider.only` takes: "deepinfra/fp8", "deepseek". */
  tag: string;
  /** "fp8"; absent when OpenRouter says "unknown". */
  quantization?: string;
  /** "flex", "priority"; absent for the plain endpoint. */
  tier?: string;
  contextLength?: number;
  /** USD per million tokens. */
  promptPerM?: number;
  completionPerM?: number;
  /** OpenRouter's status code: 0 is fine, below that the host is having trouble. */
  status?: number;
}

const str = (value: unknown): string | undefined =>
  typeof value === "string" && value !== "" ? value : undefined;

const num = (value: unknown): number | undefined =>
  typeof value === "number" && Number.isFinite(value) && value > 0 ? value : undefined;

function record(value: unknown): Record<string, unknown> {
  if (isRecord(value)) return value;
  return {};
}

function list(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  return [];
}

/** pi-ai's own openrouter models. */
export function bundledOpenRouterModels(): Model<Api>[] {
  const models: Model<Api>[] = getModels(PROVIDER);
  return models;
}

/** "0.00000015" (USD per token) → 0.15 (per million). Missing, junk and negative (-1) → undefined. */
function perMillion(value: unknown): number | undefined {
  const perToken = typeof value === "string" ? Number(value) : value;
  if (typeof perToken !== "number" || !Number.isFinite(perToken) || perToken < 0) return undefined;
  return Number((perToken * 1e6).toPrecision(10));
}

/** One entry of GET /models as a pi-ai Model, built from a template for what the list lacks. */
function liveModel(entry: Record<string, unknown>, template: Model<Api>): Model<Api> | undefined {
  const id = str(entry.id);
  if (id === undefined) return undefined;
  const architecture = record(entry.architecture);
  const inputs = list(architecture.input_modalities);
  const outputs = list(architecture.output_modalities);
  // Image and audio generators can't be chatted with.
  if (outputs.length > 0 && !outputs.includes("text")) return undefined;
  const pricing = record(entry.pricing);
  const top = record(entry.top_provider);
  const supported = list(entry.supported_parameters);
  const contextWindow = num(entry.context_length);
  if (contextWindow === undefined) return undefined;
  const maxTokens = Math.min(num(top.max_completion_tokens) ?? template.maxTokens, contextWindow);
  return {
    ...template,
    id,
    name: str(entry.name) ?? id,
    reasoning: supported.includes("reasoning"),
    input: inputs.includes("image") ? ["text", "image"] : ["text"],
    cost: {
      input: perMillion(pricing.prompt) ?? 0,
      output: perMillion(pricing.completion) ?? 0,
      cacheRead: perMillion(pricing.input_cache_read) ?? 0,
      cacheWrite: perMillion(pricing.input_cache_write) ?? 0,
    },
    contextWindow,
    maxTokens,
  };
}

/**
 * The live list, in OpenRouter's order. pi-ai's entry wins where it has one (its metadata is
 * curated: thinking levels, compat); the rest are built from a plain pi-ai openrouter model.
 */
export function mergeLiveModels(body: unknown, bundled: Model<Api>[]): Model<Api>[] {
  const data = isRecord(body) ? body.data : undefined;
  if (!Array.isArray(data)) throw new Error("OpenRouter's model list has no data array");
  const template = bundled.find((m) => !m.compat && !m.thinkingLevelMap) ?? bundled[0];
  if (!template) throw new Error("pi-ai has no openrouter template");
  const known = new Map(bundled.map((model) => [model.id, model]));
  const merged: Model<Api>[] = [];
  const seen = new Set<string>();
  for (const entry of data) {
    if (!isRecord(entry)) continue;
    const id = str(entry.id);
    if (id === undefined || seen.has(id)) continue;
    const bundledModel = known.get(id);
    // (pi-ai prices the auto router at -1 per token; the live entry says 0 for that.)
    const usable = bundledModel && bundledModel.cost.input >= 0 && bundledModel.cost.output >= 0;
    const model = usable ? bundledModel : liveModel(entry, template);
    if (!model) continue;
    seen.add(id);
    merged.push(model);
  }
  if (merged.length === 0) throw new Error("OpenRouter's model list is empty");
  return merged;
}

/** GET /models/<id>/endpoints → the hosts, in OpenRouter's order. */
export function parseEndpoints(body: unknown): OpenRouterEndpoint[] {
  const data = isRecord(body) ? body.data : undefined;
  const list = isRecord(data) ? data.endpoints : undefined;
  if (!Array.isArray(list)) throw new Error("OpenRouter's endpoint list has no endpoints array");
  const out: OpenRouterEndpoint[] = [];
  for (const entry of list) {
    if (!isRecord(entry)) continue;
    const tag = str(entry.tag);
    if (tag === undefined) continue;
    const pricing = record(entry.pricing);
    const endpoint: OpenRouterEndpoint = { provider: str(entry.provider_name) ?? tag, tag };
    const quantization = str(entry.quantization);
    if (quantization !== undefined && quantization !== "unknown") {
      endpoint.quantization = quantization;
    }
    const tier = str(entry.service_tier);
    if (tier !== undefined) endpoint.tier = tier;
    const contextLength = num(entry.context_length);
    if (contextLength !== undefined) endpoint.contextLength = contextLength;
    const prompt = perMillion(pricing.prompt);
    if (prompt !== undefined) endpoint.promptPerM = prompt;
    const completion = perMillion(pricing.completion);
    if (completion !== undefined) endpoint.completionPerM = completion;
    if (typeof entry.status === "number") endpoint.status = entry.status;
    out.push(endpoint);
  }
  return out;
}

/** 0.15 → "0.15", 0.098 → "0.098", 3 → "3.00". */
function price(perM: number): string {
  return perM >= 1 ? perM.toFixed(2) : perM.toPrecision(2);
}

/** 1048576 → "1M", 131072 → "131K". */
function context(tokens: number): string {
  if (tokens >= 1e6) return `${(tokens / 1e6).toFixed(1).replace(/\.0$/, "")}M`;
  return `${Math.round(tokens / 1000)}K`;
}

/** "DeepInfra · fp8 · $0.15 / $0.60 per M · 1M ctx". */
export function endpointLabel(endpoint: OpenRouterEndpoint): string {
  const parts = [endpoint.tier ? `${endpoint.provider} ${endpoint.tier}` : endpoint.provider];
  if (endpoint.quantization) parts.push(endpoint.quantization);
  if (endpoint.promptPerM !== undefined && endpoint.completionPerM !== undefined) {
    parts.push(`$${price(endpoint.promptPerM)} / $${price(endpoint.completionPerM)} per M`);
  }
  if (endpoint.contextLength !== undefined) parts.push(`${context(endpoint.contextLength)} ctx`);
  return parts.join(" · ");
}

/** The host a model value pins, if any: "openrouter/a/b@deepinfra/fp8" → "deepinfra/fp8". */
export function hostOf(model: Model<Api>): string | undefined {
  if (model.provider !== PROVIDER) return undefined;
  const only = (model.compat as { openRouterRouting?: { only?: string[] } } | undefined)
    ?.openRouterRouting?.only;
  return only?.length === 1 ? only[0] : undefined;
}

/** The model that sends its requests to this host only (a copy; registry objects are shared). */
export function pinnedTo(model: Model<Api>, tag: string): Model<Api> {
  return {
    ...model,
    compat: { ...model.compat, openRouterRouting: { only: [tag], allow_fallbacks: false } },
  } as Model<Api>;
}

export interface OpenRouterCatalogOptions {
  /** Replaces the network; for tests. */
  fetch?: FetchFn;
  logger?: Logger;
  now?: () => number;
  /** pi-ai's own openrouter list; for tests. */
  bundled?: () => Model<Api>[];
}

/** The live model list, cached in memory, and the per-model host lists. */
export class OpenRouterCatalog {
  private models: Model<Api>[] | undefined;
  private expires = 0;
  private loading: Promise<void> | undefined;
  private readonly fetchFn: FetchFn;
  private readonly now: () => number;

  constructor(private readonly options: OpenRouterCatalogOptions = {}) {
    this.fetchFn = options.fetch ?? ((url, init) => fetch(url, init));
    this.now = options.now ?? Date.now;
  }

  private bundled(): Model<Api>[] {
    return (this.options.bundled ?? bundledOpenRouterModels)();
  }

  /**
   * The live list once it has arrived, else undefined (use pi-ai's). Never waits: a missing or
   * old list starts a refresh in the background.
   */
  cached(): Model<Api>[] | undefined {
    if (this.now() >= this.expires) void this.refresh();
    return this.models;
  }

  /** Fetches the list now (one fetch at a time); a failure is logged and keeps the old list. */
  refresh(): Promise<void> {
    this.loading ??= this.load().finally(() => {
      this.loading = undefined;
    });
    return this.loading;
  }

  private async load(): Promise<void> {
    try {
      const body = await this.get(`${BASE_URL}/models`);
      this.models = mergeLiveModels(body, this.bundled());
      this.expires = this.now() + TTL_MS;
    } catch (error) {
      this.expires = this.now() + RETRY_MS;
      const reason = error instanceof Error ? error.message : String(error);
      this.options.logger?.warn(`OpenRouter's model list could not be fetched: ${reason}`);
    }
  }

  /** The hosts of a model. Rejects with a readable message when the fetch fails. */
  async endpoints(modelId: string): Promise<OpenRouterEndpoint[]> {
    const path = modelId
      .split("/")
      .map((part) => encodeURIComponent(part).replaceAll("%3A", ":"))
      .join("/");
    return parseEndpoints(await this.get(`${BASE_URL}/models/${path}/endpoints`));
  }

  private async get(url: string): Promise<unknown> {
    const response = await this.fetchFn(url, { signal: AbortSignal.timeout(TIMEOUT_MS) });
    if (!response.ok) throw new Error(`${url} answered ${response.status}`);
    return response.json();
  }
}
