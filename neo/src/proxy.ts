// A model proxy for neo's requests (#1): `proxy: { "openrouter": "http://localhost:8787/v1" }`
// in config.json puts that URL in place of the base URL of every model of the provider, so the
// request goes to the proxy (headroom, LiteLLM…), which forwards it to the one upstream it is set
// up for. The setting is keyed by provider because a proxy has a single upstream.
//
// pi-ai decides some things from the base URL it is given, and a proxy's address says nothing of
// the provider behind it. So a proxied model carries what the real URL would have made pi-ai do:
//
//   - the completions API's compatibility flags (no `store`, no developer role for DeepSeek, the
//     reasoning format, …) are written into `model.compat`, worked out from the real URL;
//   - the OpenAI prompt cache key, which pi-ai only puts in a request that goes to api.openai.com,
//     is put into the request body by `restoreBody`.
//
// `detectCompat` copies the URL-dependent part of pi-ai's (1.5.0, pinned in package.json);
// proxy.test.ts compares the request bodies of every model pi-ai knows with and without a proxy,
// so a pi-ai that changes its detection fails there.
import type { Api, Model, SimpleStreamOptions } from "@earendil-works/pi-ai";
import { clampOpenAIPromptCacheKey } from "@earendil-works/pi-ai/api/openai-prompt-cache";
import { isRecord } from "./util.ts";

/** Provider name → the base URL its models use instead of their own. */
export type ProxyConfig = Record<string, string>;

type Completions = NonNullable<Model<"openai-completions">["compat"]>;

function isCompletions(model: Model<Api>): model is Model<"openai-completions"> {
  return model.api === "openai-completions";
}

/**
 * pi-ai's `detectCompat`, the flags that depend on the base URL: what the provider and the URL
 * make of a completions model.
 */
export function detectCompat(
  model: Model<"openai-completions">,
): Required<
  Pick<
    Completions,
    | "supportsStore"
    | "supportsDeveloperRole"
    | "supportsReasoningEffort"
    | "maxTokensField"
    | "requiresReasoningContentOnAssistantMessages"
    | "thinkingFormat"
    | "sendSessionAffinityHeaders"
    | "sessionAffinityFormat"
    | "supportsLongCacheRetention"
  >
> &
  Pick<Completions, "cacheControlFormat"> {
  const { provider, baseUrl } = model;
  const isZai =
    provider === "zai" ||
    provider === "zai-coding-cn" ||
    baseUrl.includes("api.z.ai") ||
    baseUrl.includes("open.bigmodel.cn");
  const isTogether =
    provider === "together" ||
    baseUrl.includes("api.together.ai") ||
    baseUrl.includes("api.together.xyz");
  const isMoonshot =
    provider === "moonshotai" || provider === "moonshotai-cn" || baseUrl.includes("api.moonshot.");
  const isOpenRouter = provider === "openrouter" || baseUrl.includes("openrouter.ai");
  const isCloudflareWorkersAI =
    provider === "cloudflare-workers-ai" || baseUrl.includes("api.cloudflare.com");
  const isCloudflareAiGateway =
    provider === "cloudflare-ai-gateway" || baseUrl.includes("gateway.ai.cloudflare.com");
  const isNvidia = provider === "nvidia" || baseUrl.includes("integrate.api.nvidia.com");
  const isAntLing = provider === "ant-ling" || baseUrl.includes("api.ant-ling.com");
  const isCerebras = provider === "cerebras" || baseUrl.includes("cerebras.ai");
  const isDeepSeek = provider === "deepseek" || baseUrl.toLowerCase().includes("deepseek.com");
  const isGrok = provider === "xai" || baseUrl.includes("api.x.ai");
  const isNonStandard =
    isNvidia ||
    isCerebras ||
    isGrok ||
    isTogether ||
    baseUrl.includes("chutes.ai") ||
    isDeepSeek ||
    isZai ||
    isMoonshot ||
    provider === "opencode" ||
    baseUrl.includes("opencode.ai") ||
    isCloudflareWorkersAI ||
    isCloudflareAiGateway ||
    isAntLing;
  const useMaxTokens =
    baseUrl.includes("chutes.ai") ||
    isDeepSeek ||
    isMoonshot ||
    isCloudflareAiGateway ||
    isTogether ||
    isNvidia ||
    isAntLing ||
    isZai;
  const isOpenRouterDeveloperRoleModel =
    isOpenRouter && (model.id.startsWith("anthropic/") || model.id.startsWith("openai/"));
  let thinkingFormat: Completions["thinkingFormat"] = "openai";
  if (isDeepSeek) thinkingFormat = "deepseek";
  else if (isZai) thinkingFormat = "zai";
  else if (isTogether) thinkingFormat = "together";
  else if (isAntLing) thinkingFormat = "ant-ling";
  else if (isOpenRouter) thinkingFormat = "openrouter";
  const detected = {
    supportsStore: !isNonStandard,
    supportsDeveloperRole: isOpenRouterDeveloperRoleModel || (!isNonStandard && !isOpenRouter),
    supportsReasoningEffort:
      !isGrok &&
      !isZai &&
      !isMoonshot &&
      !isTogether &&
      !isCloudflareAiGateway &&
      !isNvidia &&
      !isAntLing,
    maxTokensField: useMaxTokens ? ("max_tokens" as const) : ("max_completion_tokens" as const),
    requiresReasoningContentOnAssistantMessages: isDeepSeek,
    thinkingFormat,
    sendSessionAffinityHeaders: isOpenRouter,
    sessionAffinityFormat: isOpenRouter ? ("openrouter" as const) : ("openai" as const),
    supportsLongCacheRetention: !(
      isTogether ||
      isCloudflareWorkersAI ||
      isCloudflareAiGateway ||
      isNvidia ||
      isAntLing
    ),
  };
  if (provider === "openrouter" && model.id.startsWith("anthropic/")) {
    return { ...detected, cacheControlFormat: "anthropic" };
  }
  return detected;
}

/** The entries that are set: spreading an explicit `undefined` would overwrite a detected value. */
function definedOnly(value: Completions | undefined): Completions {
  const out: Record<string, unknown> = {};
  for (const [key, entry] of Object.entries(value ?? {})) {
    if (entry !== undefined) out[key] = entry;
  }
  return out;
}

/**
 * The model with the proxy's base URL for its provider; the same model when there is none. A
 * completions model keeps what its real URL made of it (see the top of the file).
 */
export function proxiedModel(model: Model<Api>, proxy: ProxyConfig | undefined): Model<Api> {
  const url = proxy?.[model.provider];
  if (url === undefined || url === model.baseUrl) return model;
  // The setting is the provider's `…/v1` base; Anthropic's SDK adds `/v1/messages` itself.
  if (model.api === "anthropic-messages") return { ...model, baseUrl: url.replace(/\/v1\/?$/, "") };
  if (!isCompletions(model)) return { ...model, baseUrl: url };
  const proxied: Model<"openai-completions"> = {
    ...model,
    baseUrl: url,
    compat: { ...detectCompat(model), ...definedOnly(model.compat) },
  };
  return proxied;
}

/**
 * What pi-ai put in the request body because of the real URL, put back into a request that a
 * proxy's URL made it leave out. Mutates `payload` (the body about to be sent) and returns it.
 */
export function restoreBody(
  payload: unknown,
  original: Model<Api>,
  options: Pick<SimpleStreamOptions, "cacheRetention" | "sessionId"> | undefined,
): unknown {
  if (!isCompletions(original) || !isRecord(payload)) return payload;
  const retention =
    options?.cacheRetention ?? (process.env.PI_CACHE_RETENTION === "long" ? "long" : "short");
  if (original.baseUrl.includes("api.openai.com") && retention !== "none") {
    payload.prompt_cache_key = clampOpenAIPromptCacheKey(options?.sessionId);
  }
  return payload;
}

/** The request's options with `restoreBody` chained after whatever payload hook they had. */
export function restoringOptions(
  original: Model<Api>,
  options: SimpleStreamOptions | undefined,
): SimpleStreamOptions {
  const before = options?.onPayload;
  return {
    ...options,
    onPayload: async (payload, model) => {
      const changed = before ? ((await before(payload, model)) ?? payload) : payload;
      return restoreBody(changed, original, options);
    },
  };
}

/**
 * The model and options to make a request with: the proxy's, when the provider has one. Anything
 * not a URL in the setting is a mistake the config loader has already refused.
 */
export function throughProxy(
  model: Model<Api>,
  options: SimpleStreamOptions | undefined,
  proxy: ProxyConfig | undefined,
): [Model<Api>, SimpleStreamOptions | undefined] {
  const proxied = proxiedModel(model, proxy);
  if (proxied === model) return [model, options];
  return [proxied, restoringOptions(model, options)];
}
