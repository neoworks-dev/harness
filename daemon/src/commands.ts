// Commands typed in any pane that the daemon answers itself (not the agent):
//   /model, /effort, /mode [harness] [value]  set it for this session; without a value, pick
//                                             from what the harness offers
//   /resume [guest]                           pick a stored guest conversation to continue
//   /approvals [ask|review|edits|auto]        how much the agents ask first; no value: next mode
//   /harness [name]                           the harness the session runs on; no name: pick one
//   /title                                    write the session's title again, with a model
//   /title-model [harness] [model|default]    the model that writes titles (for every session)
//   /safesearch [strict|moderate|off]         what neo's web search may return, for every session
//   /login [provider | key [provider]]        add an API key or an account (login.ts)
//   /logout [provider], /accounts             remove a login; list what is logged in
//   /plugin …                                 marketplaces and plugins (plugins.ts)
// The harness defaults to the one the session runs on. Nothing is chosen for the user.
import type { ConfigOption, SafeSearch } from "@suite/harness/agent";
import type { HarnessSettings } from "@suite/protocol/types";

import type { Asker, Choice } from "./asker.ts";
import { BadRequestError } from "./focus.ts";
import type { GuestManager, StoredConversation } from "./guests/guests.ts";
import type { CommandContext } from "./hub.ts";
import type { Logger } from "./log.ts";
import type { Login } from "./login.ts";
import type { PluginCommands } from "./plugins.ts";
import type { SessionManager } from "./sessions.ts";
import {
  APPROVALS,
  CATEGORIES,
  type HarnessSettingName,
  isSettingName,
  nextApprovals,
  parseApprovals,
  parseSafeSearch,
  SAFE_SEARCH,
  type Settings,
  type TitleModel,
  titleModelText,
} from "./settings.ts";
import type { Titles } from "./titles.ts";

/**
 * The commands the daemon answers itself (and /clear, which the hub answers): a harness's or a
 * plugin's command of the same name is reached some other way, see plugin-commands.ts.
 */
export const DAEMON_COMMANDS: readonly string[] = [
  ...Object.keys(CATEGORIES),
  "clear",
  "resume",
  "approvals",
  "safesearch",
  "harness",
  "title",
  "title-model",
  "login",
  "logout",
  "accounts",
  "plugin",
];

/** How long a harness gets to tell its options before the picker shows it without them. */
export const SETTINGS_TIMEOUT_MS = 5000;

/** How many stored conversations per guest /resume offers. */
const RESUME_LIMIT = 8;

export interface CommandParts {
  sessions: SessionManager;
  guests: GuestManager;
  settings: Settings;
  titles: Titles;
  asker: Asker;
  login: Login;
  /** /plugin; absent: the command doesn't exist. */
  plugins?: PluginCommands | undefined;
  log: Logger;
  /** Tells the user something in the session's log. */
  notice: (session: string, text: string, tone?: "muted" | "danger") => void;
  /** The daemon's own agent's harness name, e.g. neo. */
  main: string;
  /** The harness the session runs on now. */
  harness: (session: string) => string;
  /** Every harness a session can run on (the daemon's agent and the configured guests). */
  harnesses: () => string[];
  /** Per harness, how long `settings` waits for its options. Defaults to SETTINGS_TIMEOUT_MS. */
  settingsTimeoutMs?: number | undefined;
}

function when(iso: string | undefined): string | undefined {
  if (iso === undefined) return undefined;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return undefined;
  return date.toISOString().slice(0, 16).replace("T", " ");
}

export class Commands {
  constructor(private readonly parts: CommandParts) {}

  /** Runs a command; undefined when the text isn't one (it goes to the agent then). */
  run(session: string, text: string, context?: CommandContext): Promise<void> | undefined {
    const [head, ...args] = text.trim().split(/\s+/);
    if (head?.startsWith("/") !== true) return undefined;
    const name = head.slice(1);
    if (isSettingName(name)) return this.setting(session, name, args);
    if (name === "resume") return this.resume(session, args);
    if (name === "approvals") return this.approvals(session, args);
    if (name === "safesearch") return this.safeSearch(session, args);
    if (name === "harness") return this.harness(session, args);
    if (name === "title") return this.titleCommand(session, args);
    if (name === "title-model") return this.titleModel(session, args);
    if (name === "login")
      return this.needsClient(context, (c) => this.parts.login.login(session, args, c));
    if (name === "logout") return this.parts.login.logout(session, args);
    if (name === "accounts") return this.parts.login.accounts(session);
    if (name === "plugin" && this.parts.plugins) return this.parts.plugins.run(session, args);
    return undefined;
  }

  /** Commands that ask the user something privately need the client that typed them. */
  private needsClient(
    context: CommandContext | undefined,
    run: (context: CommandContext) => Promise<void>,
  ): Promise<void> {
    if (!context) return Promise.reject(new BadRequestError("this needs a client to ask"));
    return run(context);
  }

  /**
   * Every harness the session can switch to with its options, for a client's picker (the data
   * /model, /effort, /mode and /harness offer as choices). Asks the harnesses in parallel; one that
   * fails or is too slow is listed without options and with the reason. Fetching a guest's options
   * starts its adapter for this session if it isn't running (it stays up, so asking again is quick).
   * With `known`, nothing is started: a harness whose options aren't known yet is listed without.
   */
  settings(session: string, known = false): Promise<HarnessSettings[]> {
    const current = this.parts.harness(session);
    return Promise.all(
      this.parts.harnesses().map(async (name): Promise<HarnessSettings> => {
        const entry: HarnessSettings = {
          name,
          title: this.title(name),
          current: name === current,
          options: [],
        };
        try {
          entry.options = known
            ? await this.knownOptions(session, name)
            : await this.withTimeout(this.options(session, name), name);
        } catch (err) {
          entry.error = err instanceof Error ? err.message : String(err);
          this.parts.log.warn(`settings: ${name}: ${entry.error}`);
        }
        return entry;
      }),
    );
  }

  private withTimeout<T>(work: Promise<T>, harness: string): Promise<T> {
    const ms = this.parts.settingsTimeoutMs ?? SETTINGS_TIMEOUT_MS;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const late = new Promise<never>((_, reject) => {
      timer = setTimeout(
        () => reject(new Error(`${this.title(harness)} did not answer within ${ms} ms`)),
        ms,
      );
    });
    return Promise.race([work, late]).finally(() => clearTimeout(timer));
  }

  private title(harness: string): string {
    return harness === this.parts.main ? harness : this.parts.guests.title(harness);
  }

  private async setting(session: string, name: HarnessSettingName, args: string[]): Promise<void> {
    const { guests, main } = this.parts;
    if (args.length > 2) throw new BadRequestError(`usage: /${name} [harness] [value]`);
    let harness = this.parts.harness(session);
    if (args[0] !== undefined && (args[0] === main || guests.has(args[0]))) {
      harness = args.shift() ?? harness;
    }
    const value = args[0];
    if (value !== undefined) {
      await this.apply(session, harness, name, value);
      this.followUp(session, harness, name, value);
      return;
    }
    const option = (await this.options(session, harness)).find(
      (o) => o.category === CATEGORIES[name],
    );
    if (!option || option.values.length === 0) {
      throw new BadRequestError(`${this.title(harness)} has no ${name} setting to pick from`);
    }
    let title = `${this.title(harness)} ${name}`;
    if (option.current !== undefined) title += ` (now: ${option.current})`;
    // The answer comes later, from a button; the command is done once the choice is shown.
    const choice: Omit<Choice, "id"> = {
      type: "choice",
      title,
      options: option.values.map((v) => {
        const entry = { id: v.value, name: v.name };
        return v.description === undefined ? entry : { ...entry, description: v.description };
      }),
    };
    if (option.values.some((v) => v.value === option.current)) choice.current = option.current;
    void this.parts.asker
      .ask(session, choice)
      .then(async (picked) => {
        if (picked === undefined) return;
        await this.apply(session, harness, name, picked);
        this.followUp(session, harness, name, picked);
      })
      .catch((err: unknown) => this.parts.log.warn(`/${name}: ${String(err)}`));
  }

  /**
   * Once a setting is set, the harness may have a second question (neo: which OpenRouter host
   * runs the model). It is asked after the command returns, as it may look something up; the
   * answer is applied like a typed value but is not followed up again.
   */
  private followUp(
    session: string,
    harness: string,
    name: HarnessSettingName,
    value: string,
  ): void {
    if (!this.isSessionAgent(session, harness)) return;
    const { sessions, asker, log } = this.parts;
    void sessions
      .followUp(session, CATEGORIES[name], value, harness)
      .then(async (next) => {
        if (!next) return;
        if (next.notice !== undefined) this.parts.notice(session, next.notice, "muted");
        if (!next.choice) return;
        const { title, options } = next.choice;
        const choice: Omit<Choice, "id"> = {
          type: "choice",
          title,
          options: options.map((o) => {
            const entry = { id: o.value, name: o.name };
            return o.description === undefined ? entry : { ...entry, description: o.description };
          }),
        };
        const picked = await asker.ask(session, choice);
        if (picked !== undefined) await this.apply(session, harness, name, picked);
      })
      .catch((err: unknown) => log.warn(`/${name}: ${String(err)}`));
  }

  /** Whether the session manager runs it (the daemon's agent, the session's harness), not a guest. */
  private isSessionAgent(session: string, harness: string): boolean {
    return harness === this.parts.main || harness === this.parts.harness(session);
  }

  /** What a harness offers without starting anything; none when that isn't known yet. */
  private async knownOptions(session: string, harness: string): Promise<ConfigOption[]> {
    if (harness === this.parts.main) return this.parts.sessions.configOptions(session, harness);
    return (await this.parts.guests.knownOptions(session, harness)) ?? [];
  }

  private options(session: string, harness: string): Promise<ConfigOption[]> {
    if (this.isSessionAgent(session, harness)) {
      return this.parts.sessions.configOptions(session, harness);
    }
    return this.parts.guests.configOptions(session, harness);
  }

  /** Sets it on the running harness (which rejects values it doesn't take), then records it. */
  private async apply(
    session: string,
    harness: string,
    name: HarnessSettingName,
    value: string,
  ): Promise<void> {
    const category = CATEGORIES[name];
    if (this.isSessionAgent(session, harness)) {
      await this.parts.sessions.configure(session, category, value, harness);
    } else {
      await this.parts.guests.configure(session, harness, category, value);
    }
    this.parts.settings.set(session, harness, name, value);
  }

  private approvals(session: string, args: string[]): Promise<void> {
    const [value] = args;
    const mode = value === undefined ? undefined : parseApprovals(value);
    if (args.length > 1 || (value !== undefined && mode === undefined)) {
      return Promise.reject(new BadRequestError(`usage: /approvals [${APPROVALS.join("|")}]`));
    }
    const { settings } = this.parts;
    // Without a value: the next mode, ask → review → edits → auto → ask.
    settings.setApprovals(session, mode ?? nextApprovals(settings.approvals(session)));
    return Promise.resolve();
  }

  private safeSearch(session: string, args: string[]): Promise<void> {
    const [value] = args;
    const level = value === undefined ? undefined : parseSafeSearch(value);
    if (args.length > 1 || (value !== undefined && level === undefined)) {
      return Promise.reject(new BadRequestError(`usage: /safesearch [${SAFE_SEARCH.join("|")}]`));
    }
    const set = (picked: SafeSearch): void => {
      this.parts.settings.setSafeSearch(picked);
      this.parts.notice(session, `Safe search: ${picked}, for web searches in every session.`);
    };
    if (level !== undefined) {
      set(level);
      return Promise.resolve();
    }
    const current = this.parts.settings.safeSearch();
    const choice: Omit<Choice, "id"> = {
      type: "choice",
      title: `Safe search for web searches (now: ${current ?? "the default from config.json"})`,
      options: [
        {
          id: "strict",
          name: "strict",
          description: "No adult content; engines without a filter are left out",
        },
        { id: "moderate", name: "moderate", description: "No explicit images or videos" },
        { id: "off", name: "off", description: "No filter" },
      ],
    };
    if (current !== undefined) choice.current = current;
    // The answer comes later, from a button; the command is done once the choice is shown.
    void this.parts.asker
      .ask(session, choice)
      .then((picked) => {
        const chosen = picked === undefined ? undefined : parseSafeSearch(picked);
        if (chosen !== undefined) set(chosen);
      })
      .catch((err: unknown) => this.parts.log.warn(`/safesearch: ${String(err)}`));
    return Promise.resolve();
  }

  private harness(session: string, args: string[]): Promise<void> {
    const [value] = args;
    const { settings, guests, main } = this.parts;
    const names = this.parts.harnesses();
    if (args.length > 1 || (value !== undefined && !names.includes(value))) {
      return Promise.reject(new BadRequestError(`usage: /harness [${names.join("|")}]`));
    }
    if (value !== undefined) {
      settings.setHarness(session, value);
      return Promise.resolve();
    }
    // The answer comes later, from a button; the command is done once the choice is shown.
    void this.parts.asker
      .ask(session, {
        type: "choice",
        title: `Harness (now: ${this.parts.harness(session)})`,
        options: names.map((id) => ({ id, name: id === main ? id : guests.title(id) })),
      })
      .then((picked) => {
        if (picked !== undefined) settings.setHarness(session, picked);
      })
      .catch((err: unknown) => this.parts.log.warn(`/harness: ${String(err)}`));
    return Promise.resolve();
  }

  private async titleCommand(session: string, args: string[]): Promise<void> {
    if (args.length > 0) throw new BadRequestError("usage: /title");
    await this.parts.titles.regenerate(session);
  }

  /**
   * /title-model: no value opens a choice of harness and then of its model; `default` goes back
   * to the suite's choice; a harness alone is its own default model.
   */
  private async titleModel(session: string, args: string[]): Promise<void> {
    const { guests, main, settings, titles } = this.parts;
    const names = this.parts.harnesses().filter((name) => name === main || guests.has(name));
    const usage = `usage: /title-model [${names.join("|")}] [model], or /title-model default`;
    const [harness, model, ...extra] = args;
    if (extra.length > 0 || (harness === "default" && model !== undefined)) {
      throw new BadRequestError(usage);
    }
    if (harness === "default") {
      settings.setTitleModel(session, undefined);
      return;
    }
    if (harness !== undefined && !names.includes(harness)) throw new BadRequestError(usage);
    if (harness !== undefined && model !== undefined) {
      await this.setTitleModel(session, { harness, model });
      return;
    }
    const { model: now, chosen } = titles.current();
    let current = titleModelText(now);
    if (!chosen) current += ", the default";
    // The answers come later, from buttons; the command is done once the choice is shown.
    const picked =
      harness !== undefined
        ? Promise.resolve(harness)
        : this.parts.asker.ask(session, {
            type: "choice",
            title: `Title model (now: ${current})`,
            options: [
              { id: "default", name: "Default", description: "The suite's choice" },
              ...names.map((id) => ({ id, name: id === main ? id : guests.title(id) })),
            ],
            current: chosen ? now.harness : "default",
          });
    void picked
      .then(async (name) => {
        if (name === undefined) return;
        if (name === "default") settings.setTitleModel(session, undefined);
        else await this.askTitleModel(session, name);
      })
      .catch((err: unknown) => this.parts.log.warn(`/title-model: ${String(err)}`));
  }

  /** The second step of /title-model: which of the harness's models, if it lists any. */
  private async askTitleModel(session: string, harness: string): Promise<void> {
    const options = await this.options(session, harness);
    const option = options.find((o) => o.category === "model");
    if (!option || option.values.length === 0) {
      await this.setTitleModel(session, { harness });
      return;
    }
    const picked = await this.parts.asker.ask(session, {
      type: "choice",
      title: `${this.title(harness)} model for titles`,
      options: option.values.map((v) => {
        const entry = { id: v.value, name: v.name };
        return v.description === undefined ? entry : { ...entry, description: v.description };
      }),
    });
    if (picked !== undefined) await this.setTitleModel(session, { harness, model: picked });
  }

  private async setTitleModel(session: string, value: TitleModel): Promise<void> {
    const { main, settings } = this.parts;
    // A guest lists the models it takes; pi takes any provider/model.
    let listed: ConfigOption["values"] | undefined;
    if (value.harness !== main) {
      const known = await this.knownOptions(session, value.harness);
      listed = known.find((o) => o.category === "model")?.values;
    }
    if (
      value.model !== undefined &&
      listed?.length &&
      !listed.some((v) => v.value === value.model)
    ) {
      const values = listed.map((v) => v.value).join(", ");
      throw new BadRequestError(
        `${this.title(value.harness)} has no model ${value.model}: ${values}`,
      );
    }
    settings.setTitleModel(session, value);
  }

  private async resume(session: string, args: string[]): Promise<void> {
    const { guests, sessions } = this.parts;
    const names = args.length > 0 ? args : guests.names();
    for (const name of names) {
      if (!guests.has(name)) throw new BadRequestError(`no guest named ${name}`);
    }
    const cwd = sessions.info(session)?.cwd;
    if (cwd === undefined) throw new BadRequestError(`no session ${session}`);
    const listed = await Promise.allSettled(names.map((name) => guests.list(name, cwd)));
    const options: { id: string; name: string; description?: string }[] = [];
    listed.forEach((result, i) => {
      const name = names[i] ?? "";
      if (result.status === "rejected") {
        this.parts.log.warn(`/resume: listing ${name} failed: ${String(result.reason)}`);
        return;
      }
      for (const conversation of result.value.slice(0, RESUME_LIMIT)) {
        options.push(this.resumeOption(name, conversation));
      }
    });
    if (options.length === 0) {
      throw new BadRequestError(`no stored ${names.join(" or ")} conversations in ${cwd}`);
    }
    void this.parts.asker
      .ask(session, { type: "choice", title: "Resume a conversation", options })
      .then(async (picked) => {
        if (picked === undefined) return;
        const colon = picked.indexOf(":");
        await guests.resume(session, picked.slice(0, colon), picked.slice(colon + 1));
      })
      .catch((err: unknown) => this.parts.log.warn(`/resume: ${String(err)}`));
  }

  private resumeOption(
    name: string,
    conversation: StoredConversation,
  ): { id: string; name: string; description?: string } {
    const option = {
      id: `${name}:${conversation.id}`,
      name: `${this.title(name)}: ${conversation.title ?? "untitled"}`,
    };
    const updated = when(conversation.updatedAt);
    return updated === undefined ? option : { ...option, description: updated };
  }
}
