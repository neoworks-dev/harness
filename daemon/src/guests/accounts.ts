// The suite's API keys for the guests (design §9, "Accounts for guests"). A guest normally uses
// its own login (Claude Code's, Codex's). When the user picks a model as `provider/model`
// (`/model codex openrouter/openai/gpt-5-mini`) and the suite has an API key for that provider,
// the guest starts on that key instead: Codex through the adapter's custom gateway sign-in,
// (Claude Code follows.) Nothing is set for a guest the user didn't point at
// an account.
//
// Only API keys are handed out, never an account: a Claude Pro/Max login may be used by neo only,
// and Codex keeps its own ChatGPT login.
import { findEnvKeys, getModels, type KnownProvider } from "@mariozechner/pi-ai";
import { type Accounts, readAccounts } from "@suite/harness/auth";
import { BadRequestError } from "../focus.ts";

/** Which guest the route is for. */
export type RouteKind = "codex";

/** How a guest process is started on one provider's key and model. */
export interface Route {
  /** `provider/model`, as the user picked it. */
  id: string;
  provider: string;
  model: string;
  /** Added to the adapter's environment; undefined removes an inherited variable. */
  env: Record<string, string | undefined>;
  /** Codex: sent with the adapter's `gateway` sign-in. */
  gateway?: { baseUrl: string; headers: Record<string, string>; providerName: string };
  /** Cut from anything shown or logged about this route. */
  secrets: string[];
}

export interface RouteModel {
  value: string;
  name: string;
  description?: string;
}

export interface GuestAccounts {
  /** The models the guest can run on the suite's keys: the providers that have one. */
  models(kind: RouteKind): Promise<RouteModel[]>;
  /**
   * The route for a model value, or undefined when the value is not `provider/model` of a
   * provider this guest can use (it is then the adapter's own). Rejects with BadRequestError
   * when the provider is known but the suite has no key for it.
   */
  route(kind: RouteKind, value: string): Promise<Route | undefined>;
}

/**
 * OpenAI-style providers that serve the Responses API, the only one Codex still speaks
 * (`wire_api = "chat"` is gone). Docs checked: OpenAI, OpenRouter
 * (https://openrouter.ai/docs/api/reference/responses/overview), Groq
 * (https://console.groq.com/docs/responses-api), xAI (https://docs.x.ai/docs/guides/responses-api),
 * Fireworks (https://docs.fireworks.ai/guides/response-api), Vercel AI Gateway
 * (https://vercel.com/docs/ai-gateway/sdks-and-apis/responses). DeepSeek, Mistral, Cerebras,
 * Z.ai, Moonshot, MiniMax and the rest only have chat completions or say nothing: #476.
 */
const CODEX_PROVIDERS: Record<string, { name: string; baseUrl: string }> = {
  openai: { name: "OpenAI", baseUrl: "https://api.openai.com/v1" },
  openrouter: { name: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1" },
  groq: { name: "Groq", baseUrl: "https://api.groq.com/openai/v1" },
  xai: { name: "xAI", baseUrl: "https://api.x.ai/v1" },
  fireworks: { name: "Fireworks", baseUrl: "https://api.fireworks.ai/inference/v1" },
  "vercel-ai-gateway": { name: "Vercel AI Gateway", baseUrl: "https://ai-gateway.vercel.sh/v1" },
};

/** How many of a provider's models /model lists; any other can still be typed. */
const LISTED_MODELS = 8;

function table(_kind: RouteKind): Record<string, { name: string; baseUrl?: string }> {
  return CODEX_PROVIDERS;
}

function modelsOf(provider: string): { id: string; reasoning: boolean }[] {
  // pi-ai types the providers; ours are all known ones.
  return getModels(provider as KnownProvider).map((m) => ({ id: m.id, reasoning: m.reasoning }));
}

function codexConfig(model: string): string {
  // Whatever the user has in CODEX_CONFIG stays; only the model is ours.
  let config: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(process.env.CODEX_CONFIG ?? "{}");
    if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
      config = { ...parsed };
    }
  } catch {
    // Not JSON: the adapter would refuse it, and so we replace it.
  }
  return JSON.stringify({ ...config, model });
}

export function codexRoute(provider: string, model: string, key: string): Route {
  const known = CODEX_PROVIDERS[provider];
  if (!known?.baseUrl) throw new BadRequestError(`Codex can't use ${provider}`);
  return {
    id: `${provider}/${model}`,
    provider,
    model,
    // The adapter takes any model id through its config, not through the options it lists.
    env: { CODEX_CONFIG: codexConfig(model) },
    gateway: {
      baseUrl: known.baseUrl,
      headers: { Authorization: `Bearer ${key}` },
      providerName: known.name,
    },
    secrets: [key],
  };
}

export interface AccountFiles {
  /** The suite's accounts file. */
  authPath: string;
  /** pi's, read only. */
  piAuthPath: string;
}

async function readOr(path: string): Promise<Accounts> {
  try {
    return await readAccounts(path);
  } catch {
    return {};
  }
}

/**
 * The API key for a provider: its environment variable, the suite's file, then pi's. Never an
 * OAuth login (so never a Claude subscription) and never SUITE_API_KEY, which is neo's own.
 */
async function apiKey(files: AccountFiles, provider: string): Promise<string | undefined> {
  for (const name of findEnvKeys(provider) ?? []) {
    if (name.includes("OAUTH")) continue;
    const value = process.env[name];
    if (value) return value;
  }
  for (const path of [files.authPath, files.piAuthPath]) {
    const account = (await readOr(path))[provider];
    if (account?.type === "api_key") return account.key;
  }
  return undefined;
}

export function createGuestAccounts(files: AccountFiles): GuestAccounts {
  return {
    async models(kind) {
      const out: RouteModel[] = [];
      for (const [provider, info] of Object.entries(table(kind))) {
        if ((await apiKey(files, provider)) === undefined) continue;
        const all = modelsOf(provider);
        // Models that reason first: they are the ones that can drive an agent.
        const listed = [...all.filter((m) => m.reasoning), ...all.filter((m) => !m.reasoning)];
        for (const model of listed.slice(0, LISTED_MODELS)) {
          out.push({
            value: `${provider}/${model.id}`,
            name: `${info.name}: ${model.id}`,
            description: `your ${info.name} API key`,
          });
        }
      }
      return out;
    },
    async route(kind, value) {
      const slash = value.indexOf("/");
      if (slash <= 0 || slash === value.length - 1) return undefined;
      const provider = value.slice(0, slash);
      if (!Object.hasOwn(table(kind), provider)) return undefined;
      const model = value.slice(slash + 1);
      const key = await apiKey(files, provider);
      if (key === undefined) {
        throw new BadRequestError(`No API key for ${provider}. /login ${provider} adds one.`);
      }
      return codexRoute(provider, model, key);
    },
  };
}
