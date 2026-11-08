// The agents a session can run on (harnesses.list), for apps that let the user pick one: which
// are installed, which can sign in, and how much each takes of the context before the first
// message, with its own tools and, where it can, with neo's. That number is measured on this
// machine: the first usage of an agent in a conversation that has one user message so far is
// its system prompt, its tool definitions and that message; the smallest one seen is kept.
// Until there is one, the numbers measured when this version was made stand in.
import { findEnvKeys, getProviders } from "@earendil-works/pi-ai/compat";
import type { HarnessInfo, HarnessTools, ToolsChoice } from "@neoworks-dev/neo-protocol/types";
import { accountProviders } from "@neoworks-dev/neo/auth";
import type { GuestManager } from "./guests/guests.ts";
import type { Settings } from "./settings.ts";
import type { SessionStore } from "./store.ts";

/** Measured with this version: Claude Code 2.x with its own tools, neo with its own. */
const MEASURED: Record<string, number> = {
  "claude:own": 17_700,
  "neo:own": 2_600,
};

export interface HarnessesParts {
  store: SessionStore;
  settings: Settings;
  guests: GuestManager;
  /** neo's own agent's name ("neo"); undefined: the daemon runs another (echo, tests). */
  own?: string;
  /** Where neo's accounts are (/login's and pi's). */
  authPaths: string[];
}

function key(harness: string, tools: ToolsChoice): string {
  return `${harness}:${tools}`;
}

export class Harnesses {
  /** The agents already measured per session, so only a conversation's first usage counts. */
  private seen = new Set<string>();

  constructor(private readonly parts: HarnessesParts) {
    parts.store.onAppend(({ session, event }) => {
      if (event.type !== "usage" || event.contextUsed === undefined) return;
      const harness = event.agent ?? parts.settings.harness(session);
      const at = `${session}\0${harness}`;
      if (this.seen.has(at)) return;
      this.seen.add(at);
      // A conversation further along has more in its context than the start.
      if (parts.store.readTypes(session, ["user_message"]).length !== 1) return;
      this.measured(harness, this.toolsOf(harness), event.contextUsed);
    });
  }

  /** The tools `harness` works with now. */
  private toolsOf(harness: string): ToolsChoice {
    const { guests, settings } = this.parts;
    return guests.swapsFileTools(harness) && settings.guestTools(harness) === "neo" ? "neo" : "own";
  }

  private measured(harness: string, tools: ToolsChoice, tokens: number): void {
    const name = `start_tokens:${key(harness, tools)}`;
    const before = Number(this.parts.store.getMeta(name));
    if (Number.isFinite(before) && before > 0 && before <= tokens) return;
    this.parts.store.setMeta(name, String(tokens));
  }

  private tools(harness: string, tools: ToolsChoice): HarnessTools {
    const out: HarnessTools = { id: tools, current: this.toolsOf(harness) === tools };
    const here = Number(this.parts.store.getMeta(`start_tokens:${key(harness, tools)}`));
    if (Number.isFinite(here) && here > 0) {
      out.startTokens = here;
      out.measured = true;
    } else {
      const then = MEASURED[key(harness, tools)];
      if (then !== undefined) out.startTokens = then;
    }
    return out;
  }

  /** neo has an account or an API key for some provider. */
  private ownSignedIn(): boolean {
    if (this.parts.authPaths.some((path) => accountProviders(path).length > 0)) return true;
    return getProviders().some((provider) => (findEnvKeys(provider) ?? []).length > 0);
  }

  list(): HarnessInfo[] {
    const { guests, own } = this.parts;
    const out: HarnessInfo[] = [];
    if (own !== undefined) {
      out.push({
        name: own,
        title: "Neo",
        own: true,
        installed: true,
        signedIn: this.ownSignedIn(),
        tools: [this.tools(own, "own")],
      });
    }
    for (const name of guests.names()) {
      const config = guests.config(name);
      const program = config?.program;
      const info: HarnessInfo = {
        name,
        title: guests.title(name),
        own: false,
        installed: program === undefined || Bun.which(program) !== null,
        tools: [this.tools(name, "own")],
      };
      if (guests.swapsFileTools(name)) info.tools.push(this.tools(name, "neo"));
      const signedIn = config?.signedIn?.();
      if (signedIn !== undefined) info.signedIn = signedIn;
      out.push(info);
    }
    return out;
  }
}
