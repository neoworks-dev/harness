// /login, /logout and /accounts: the suite's accounts (API keys and OAuth logins), kept in the
// suite's own auth file in pi's format (harness/src/auth.ts). A key or a pasted code is asked with
// askSecret, so it never reaches the session log; what the user sees in the chat is a notice.
//
// A login can take minutes (a browser, a device code), so the command returns once it has
// started; what happens next is told with notices and cards in the session.
import type { OAuthProviderInterface } from "@mariozechner/pi-ai/oauth";
import { type Accounts, readAccounts, removeAccount, setAccount } from "@neoworks-dev/neo/auth";
import type { Asker } from "./asker.ts";
import { BadRequestError } from "./focus.ts";
import type { CommandContext } from "./hub.ts";
import type { Logger } from "./log.ts";

/** How long one login may take before it is given up (it also ends the open prompts). */
const LOGIN_TIMEOUT_MS = 10 * 60_000;

/** What the user knows the logins by; the rest use the provider's own name. */
const OAUTH_NAMES: Record<string, string> = {
  anthropic: "Claude Pro/Max",
  "openai-codex": "ChatGPT",
  "github-copilot": "GitHub Copilot",
};

/**
 * Keys for services rather than models, which pi-ai doesn't know: what they are for (shown when
 * picking) and the variable that outranks the stored key.
 */
const SERVICES: Record<string, { description: string; env: string }> = {
  tinyfish: { description: "web search for neo", env: "TINYFISH_API_KEY" },
};

/** The `/login` argument that stands for an API key (the next word is the provider). */
const KEY = "key";

export interface LoginParts {
  asker: Asker;
  log: Logger;
  /** A line in the session's chat (tone danger for a failure). Never pass a secret. */
  notice: (session: string, text: string, tone?: "muted" | "danger") => void;
  /** The suite's accounts file: written by /login and /logout. */
  authPath: string;
  /** pi's accounts file: read only, only to tell where an account comes from. */
  piAuthPath: string;
  /** Providers that log in with an account, e.g. pi-ai's getOAuthProviders. */
  oauthProviders: () => OAuthProviderInterface[];
  /** Providers an API key works for, e.g. pi-ai's getProviders. */
  apiProviders: () => string[];
  /** The environment variables set with an API key for a provider (pi-ai's findEnvKeys). */
  envKeys: (provider: string) => string[];
  now?: () => number;
}

function oauthName(provider: OAuthProviderInterface): string {
  return OAUTH_NAMES[provider.id] ?? provider.name;
}

function when(ms: number): string {
  return new Date(ms).toISOString().slice(0, 16).replace("T", " ");
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Secrets one login has seen, to cut them out of anything that is shown or logged. */
class Secrets {
  private readonly seen = new Set<string>();

  add(value: string): void {
    // Short values would blank out ordinary words.
    if (value.length >= 6) this.seen.add(value);
  }

  scrub(text: string): string {
    let out = text;
    for (const secret of this.seen) out = out.replaceAll(secret, "***");
    return out;
  }
}

class Cancelled extends Error {
  constructor() {
    super("cancelled");
  }
}

export class Login {
  /** Logins in progress, by provider: two would fight over the same callback port. */
  private readonly running = new Set<string>();

  constructor(private readonly parts: LoginParts) {}

  /** /login [provider | key [provider]] */
  login(session: string, args: string[], context: CommandContext): Promise<void> {
    const [first, second, ...rest] = args;
    if (rest.length > 0 || (second !== undefined && first !== KEY)) {
      return Promise.reject(new BadRequestError("usage: /login [provider | key [provider]]"));
    }
    if (first === undefined) {
      this.background(session, "Login", () => this.pickMethod(session, context));
      return Promise.resolve();
    }
    if (first === KEY) {
      return this.keyLogin(session, second, context);
    }
    const oauth = this.parts.oauthProviders().find((p) => p.id === first);
    if (oauth) return this.startOAuth(session, oauth, context);
    return this.keyLogin(session, first, context);
  }

  private keyLogin(
    session: string,
    provider: string | undefined,
    context: CommandContext,
  ): Promise<void> {
    if (provider === undefined) {
      this.background(session, "Login", () => this.pickProvider(session, context));
      return Promise.resolve();
    }
    if (!this.keyProviders().includes(provider)) {
      // The argument is not echoed: it may be a key pasted by mistake.
      return Promise.reject(
        new BadRequestError("no such provider; /login key lists the ones that take an API key"),
      );
    }
    this.background(session, "Login", () => this.apiKey(session, provider, context));
    return Promise.resolve();
  }

  /** Runs `work` after the command has returned; a failure becomes a notice. */
  private background(session: string, label: string, work: () => Promise<void>): void {
    work().catch((error: unknown) => {
      if (error instanceof Cancelled) {
        this.parts.notice(session, `${label} cancelled.`);
        return;
      }
      this.parts.log.warn(`${label}: ${message(error)}`);
      this.parts.notice(session, `${label} failed: ${message(error)}`, "danger");
    });
  }

  /** What takes an API key: the model providers and the services. */
  private keyProviders(): string[] {
    return [...this.parts.apiProviders(), ...Object.keys(SERVICES)];
  }

  /** The variables that are set with a key for a provider or a service. */
  private envKeys(id: string): string[] {
    const service = SERVICES[id];
    if (!service) return this.parts.envKeys(id);
    const set: string[] = [];
    if (process.env[service.env]) set.push(service.env);
    return set;
  }

  private async pickMethod(session: string, context: CommandContext): Promise<void> {
    const options = [
      {
        id: KEY,
        name: "API key",
        description: "Any provider, or TinyFish for web search; you paste the key",
      },
      ...this.parts.oauthProviders().map((p) => ({ id: p.id, name: oauthName(p) })),
    ];
    const picked = await this.parts.asker.ask(session, {
      type: "choice",
      title: "Log in with",
      options,
    });
    if (picked === KEY) await this.pickProvider(session, context);
    else if (picked !== undefined) {
      const provider = this.parts.oauthProviders().find((p) => p.id === picked);
      if (provider) await this.startOAuth(session, provider, context);
    }
  }

  private async pickProvider(session: string, context: CommandContext): Promise<void> {
    const providers = this.keyProviders().sort();
    const picked = await this.parts.asker.ask(session, {
      type: "choice",
      title: "API key for",
      options: providers.map((id) => {
        const description = SERVICES[id]?.description;
        return description === undefined ? { id, name: id } : { id, name: id, description };
      }),
    });
    if (picked !== undefined) await this.apiKey(session, picked, context);
  }

  private async apiKey(session: string, provider: string, context: CommandContext): Promise<void> {
    const value = await context.askSecret(`${provider} API key`, "paste the key here");
    const key = value?.trim();
    if (key === undefined || key === "") {
      this.parts.notice(session, `Login to ${provider} cancelled.`);
      return;
    }
    await setAccount(this.parts.authPath, provider, { type: "api_key", key });
    this.parts.notice(session, `Logged in to ${provider} (API key).${this.envNote(provider)}`);
  }

  /** A variable that is set outranks the stored login; say so. */
  private envNote(provider: string): string {
    const [first] = this.envKeys(provider);
    if (first === undefined) return "";
    return ` ${first} is set and is used first.`;
  }

  private startOAuth(
    session: string,
    provider: OAuthProviderInterface,
    context: CommandContext,
  ): Promise<void> {
    if (this.running.has(provider.id)) {
      return Promise.reject(
        new BadRequestError(`a login to ${oauthName(provider)} is already in progress`),
      );
    }
    this.running.add(provider.id);
    this.background(session, "Login", () =>
      this.oauth(session, provider, context).finally(() => this.running.delete(provider.id)),
    );
    return Promise.resolve();
  }

  private async oauth(
    session: string,
    provider: OAuthProviderInterface,
    context: CommandContext,
  ): Promise<void> {
    const name = oauthName(provider);
    const { asker, notice } = this.parts;
    const secrets = new Secrets();
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), LOGIN_TIMEOUT_MS);
    let finished = false;
    /** A prompt the user answers privately. Cancelling it cancels the login. */
    const secret = async (
      title: string,
      placeholder?: string,
      allowEmpty = false,
    ): Promise<string> => {
      const value = await context.askSecret(title, placeholder, controller.signal);
      if (finished) return new Promise<never>(() => {}); // the login ended while this was open
      if (value === undefined || (!allowEmpty && value.trim() === "")) {
        controller.abort();
        throw new Cancelled();
      }
      secrets.add(value.trim());
      return value;
    };
    try {
      const credentials = await provider.login({
        onAuth: ({ url, instructions }) =>
          notice(
            session,
            [`Open this address to log in to ${name}:`, url, instructions]
              .filter((line) => line !== undefined && line !== "")
              .join("\n"),
          ),
        onPrompt: (prompt) => secret(prompt.message, prompt.placeholder, prompt.allowEmpty),
        onManualCodeInput: () =>
          secret("Paste the code or the address you were sent to (or finish in the browser)"),
        onSelect: (prompt) =>
          asker.ask(
            session,
            {
              type: "choice",
              title: prompt.message,
              options: prompt.options.map((o) => ({ id: o.id, name: o.label })),
            },
            controller.signal,
          ),
        onProgress: (text) => notice(session, text),
        signal: controller.signal,
      });
      finished = true;
      secrets.add(credentials.access);
      secrets.add(credentials.refresh);
      await setAccount(this.parts.authPath, provider.id, { type: "oauth", ...credentials });
      let text = `Logged in to ${name}.`;
      if (provider.id === "anthropic") text += " It is used by neo only.";
      notice(session, text + this.envNote(provider.id));
    } catch (error) {
      // What the library says may quote a code or token from the exchange.
      if (error instanceof Cancelled) throw error;
      throw new Error(secrets.scrub(message(error)));
    } finally {
      finished = true;
      clearTimeout(timer);
      controller.abort(); // closes a code prompt still open (the browser finished first)
    }
  }

  /** /logout [provider] */
  async logout(session: string, args: string[]): Promise<void> {
    if (args.length > 1) throw new BadRequestError("usage: /logout [provider]");
    const [provider] = args;
    if (provider !== undefined) {
      await this.removeOne(session, provider);
      return;
    }
    const ours = await this.read(this.parts.authPath);
    const theirs = await this.read(this.parts.piAuthPath);
    const options = Object.keys(ours)
      .sort()
      .map((id) => ({ id, name: this.label(id, ours[id]) }));
    for (const id of Object.keys(theirs).sort()) {
      if (id in ours) continue;
      options.push({ id, name: `${this.label(id, theirs[id])} (pi's file)` });
    }
    if (options.length === 0) {
      this.parts.notice(session, "No logins stored. Environment variables are not touched.");
      return;
    }
    void this.parts.asker
      .ask(session, { type: "choice", title: "Log out of", options })
      .then(async (picked) => {
        if (picked !== undefined) await this.removeOne(session, picked);
      })
      .catch((error: unknown) => {
        this.parts.log.warn(`/logout: ${message(error)}`);
        this.parts.notice(session, `Logout failed: ${message(error)}`, "danger");
      });
  }

  private async removeOne(session: string, provider: string): Promise<void> {
    const { notice, authPath, piAuthPath } = this.parts;
    const removed = await removeAccount(authPath, provider);
    const piHas = provider in (await this.read(piAuthPath));
    const vars = this.envKeys(provider);
    const lines: string[] = [];
    if (removed) lines.push(`Logged out of ${provider}.`);
    else lines.push(`No login for ${provider} in the suite's auth file.`);
    if (piHas) {
      lines.push(`${piAuthPath} has one too; that file is pi's own, so it stays. Log out in pi.`);
    }
    if (vars.length > 0) lines.push(`${vars.join(", ")} is set in the environment and stays.`);
    notice(session, lines.join(" "));
  }

  /** /accounts */
  async accounts(session: string): Promise<void> {
    const { authPath, piAuthPath } = this.parts;
    const ours = await this.read(authPath);
    const theirs = await this.read(piAuthPath);
    const providers = new Set<string>([...Object.keys(ours), ...Object.keys(theirs)]);
    const known = [...this.keyProviders(), ...this.parts.oauthProviders().map((p) => p.id)];
    for (const id of known) if (this.envKeys(id).length > 0) providers.add(id);
    // The first source is the one in use: variable, then the suite's file, then pi's.
    const lines: string[] = [];
    for (const id of [...providers].sort()) {
      const sources = this.envKeys(id);
      if (id in ours) sources.push("the suite's file");
      if (id in theirs) sources.push("pi's file");
      const account = ours[id] ?? theirs[id];
      let line = `${id}: ${this.label(id, account)} from ${sources.join(", ")}`;
      if (account?.type === "oauth") line += this.expiry(account.expires);
      lines.push(line);
    }
    if (process.env.SUITE_API_KEY)
      lines.push("SUITE_API_KEY is set: it is the key for any provider.");
    if (lines.length === 0) {
      this.parts.notice(session, "No logins. /login adds one.");
      return;
    }
    this.parts.notice(session, ["Logins (the first source is used):", ...lines].join("\n"));
  }

  private expiry(expires: number): string {
    if (expires > (this.parts.now ?? Date.now)()) return `, token until ${when(expires)}`;
    return ", token expired (renewed when used)";
  }

  /** "API key", or the account's name. */
  private label(id: string, account: Accounts[string] | undefined): string {
    if (account?.type !== "oauth") return "API key";
    const provider = this.parts.oauthProviders().find((p) => p.id === id);
    return provider ? oauthName(provider) : "account";
  }

  /** The file's logins; none when it is missing or broken (the broken case is told once). */
  private async read(path: string): Promise<Accounts> {
    try {
      return await readAccounts(path);
    } catch (error) {
      this.parts.log.warn(`accounts: ${message(error)}`);
      return {};
    }
  }
}
