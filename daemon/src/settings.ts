// What the user chose for this session: each harness's settings (/model, /effort, /mode), the
// harness the session runs on (/harness) and neo's approvals mode (/approvals). They are
// setting_changed events in the session's log; the last harness settings and the last harness
// choice are kept as the defaults for new sessions (approvals is not: a new session asks). The
// model that writes session titles (/title-model) is the user's last choice, for every session.
// Safe search (/safesearch) is the user's, not a session's: one value for every session, and so
// are the toggles /settings lists ("Show, don't tell.").
// Nothing is set that the user didn't choose: each harness keeps its own defaults.
import type { ConfigCategory, SafeSearch } from "@neoworks-dev/neo/agent";
import type { SettingName, UserSettings } from "@neoworks-dev/neo-protocol/types";
import type { SessionStore } from "./store.ts";

/** The settings that belong to a harness (approvals and harness are neo's own). */
export type HarnessSettingName = Exclude<SettingName, "approvals" | "harness" | "title_model">;

export const CATEGORIES: Record<HarnessSettingName, ConfigCategory> = {
  model: "model",
  effort: "thought_level",
  mode: "mode",
};

export function isSettingName(name: string): name is HarnessSettingName {
  return Object.hasOwn(CATEGORIES, name);
}

export type HarnessSettings = Partial<Record<ConfigCategory, string>>;

/**
 * ask: edits are previewed before they are written, commands and guests ask. review: each edit
 * is written right away and then kept or reverted by the user; the rest asks. edits: edits are
 * written right away (reviewed after the turn), everything else asks. auto: nothing asks.
 * Reads are allowed in every mode, except sensitive files (see sensitive.ts).
 */
export type Approvals = "ask" | "review" | "edits" | "auto";

export const APPROVALS: readonly Approvals[] = ["ask", "review", "edits", "auto"];

export function parseApprovals(value: string): Approvals | undefined {
  return APPROVALS.find((mode) => mode === value);
}

/** The mode after this one: ask, review, edits, auto, ask… */
export function nextApprovals(mode: Approvals): Approvals {
  return APPROVALS[(APPROVALS.indexOf(mode) + 1) % APPROVALS.length] ?? "ask";
}

/**
 * Which subagents the agents may start: both their own (Claude Code's Agent tool, Codex's
 * multi-agent tools) and neo's delegate, only delegate, or only their own (neo, which has none,
 * keeps delegate).
 */
export type Subagents = "both" | "delegate" | "native";

export const SUBAGENTS: readonly Subagents[] = ["both", "delegate", "native"];

export function parseSubagents(value: string): Subagents | undefined {
  return SUBAGENTS.find((kind) => kind === value);
}

/** The model that writes session titles: one of a harness's models, or the harness's own default. */
export interface TitleModel {
  harness: string;
  model?: string;
}

/** How a TitleModel is written in a command, an event and the store: `claude haiku`, `pi`. */
export function titleModelText(value: TitleModel): string {
  return value.model === undefined ? value.harness : `${value.harness} ${value.model}`;
}

/** The TitleModel in `text`; undefined for `default` and for nothing. */
export function parseTitleModel(text: string): TitleModel | undefined {
  const [harness, model] = text.trim().split(/\s+/);
  if (harness === undefined || harness === "" || harness === "default") return undefined;
  return model === undefined ? { harness } : { harness, model };
}

/** How much adult content neo's web_search may return; see @neoworks/websearch. */
export const SAFE_SEARCH: readonly SafeSearch[] = ["strict", "moderate", "off"];

export function parseSafeSearch(value: string): SafeSearch | undefined {
  return SAFE_SEARCH.find((level) => level === value);
}

export class Settings {
  private readonly toggleListeners: ((settings: UserSettings) => void)[] = [];

  /** `main`: the harness a session runs on until the user picks another (the daemon's agent). */
  constructor(
    private readonly store: SessionStore,
    private readonly main = "neo",
  ) {}

  /** This session's choices for a harness, over the user's last choices. */
  get(session: string, harness: string): HarnessSettings {
    const out: HarnessSettings = {};
    for (const [name, category] of Object.entries(CATEGORIES)) {
      const last = this.store.getMeta(metaKey(harness, name));
      if (last !== undefined) out[category] = last;
    }
    for (const { event } of this.store.readTypes(session, ["setting_changed"])) {
      if (event.type !== "setting_changed" || event.harness !== harness) continue;
      if (isSettingName(event.option)) out[CATEGORIES[event.option]] = event.value;
    }
    return out;
  }

  set(session: string, harness: string, option: HarnessSettingName, value: string): void {
    this.store.append(session, { type: "setting_changed", harness, option, value });
    this.store.setMeta(metaKey(harness, option), value);
  }

  /** What the user chose to write titles, or undefined: neo's choice (see titles.ts). */
  titleModel(): TitleModel | undefined {
    const last = this.store.getMeta(metaKey("daemon", "title_model"));
    return last === undefined ? undefined : parseTitleModel(last);
  }

  /** `undefined` goes back to neo's choice. Logged in the session it was set in. */
  setTitleModel(session: string, value: TitleModel | undefined): void {
    const text = value === undefined ? "default" : titleModelText(value);
    this.store.append(session, {
      type: "setting_changed",
      harness: "daemon",
      option: "title_model",
      value: text,
    });
    this.store.setMeta(metaKey("daemon", "title_model"), text);
  }

  /** The harness the session's turns run on: the last /harness choice in its log, else main. */
  harness(session: string): string {
    let value = this.main;
    for (const { event } of this.store.readTypes(session, ["setting_changed"])) {
      if (event.type === "setting_changed" && event.option === "harness") value = event.value;
    }
    return value;
  }

  /** Chooses the harness for the session's following turns; also the default for new sessions. */
  setHarness(session: string, value: string): void {
    this.store.append(session, {
      type: "setting_changed",
      harness: "daemon",
      option: "harness",
      value,
    });
    this.store.setMeta(metaKey("daemon", "harness"), value);
    this.carryOver(session, value);
  }

  /**
   * The user's last choices for a harness the session starts on (model, effort, mode), logged
   * when the session has none of its own yet: they apply anyway (see get), and logged they show
   * in the panes' status like a choice made here.
   */
  carryOver(session: string, harness: string): void {
    const chosen = new Set<string>();
    for (const { event } of this.store.readTypes(session, ["setting_changed"])) {
      if (event.type === "setting_changed" && event.harness === harness) chosen.add(event.option);
    }
    for (const name of Object.keys(CATEGORIES)) {
      const last = this.store.getMeta(metaKey(harness, name));
      if (last === undefined || chosen.has(name) || !isSettingName(name)) continue;
      this.store.append(session, { type: "setting_changed", harness, option: name, value: last });
    }
  }

  /**
   * A new session starts on the user's last harness choice, if it still exists. Logged when it
   * isn't main, so the session keeps it whatever the user picks elsewhere later (and panes see
   * it); so are that harness's last settings (carryOver).
   */
  inheritHarness(session: string, exists: (harness: string) => boolean): void {
    const last = this.store.getMeta(metaKey("daemon", "harness"));
    if (last !== undefined && last !== this.main && exists(last)) {
      this.store.append(session, {
        type: "setting_changed",
        harness: "daemon",
        option: "harness",
        value: last,
      });
    }
    this.carryOver(session, this.harness(session));
  }

  /**
   * How much the agents ask the user first (see Approvals). Per session and never carried
   * over: a new session asks.
   */
  approvals(session: string): Approvals {
    let value: Approvals = "ask";
    for (const { event } of this.store.readTypes(session, ["setting_changed"])) {
      if (event.type === "setting_changed" && event.option === "approvals") {
        value = parseApprovals(event.value) ?? "ask";
      }
    }
    return value;
  }

  /** The user's safe-search choice; undefined until they make one (the harness's config holds). */
  safeSearch(): SafeSearch | undefined {
    const value = this.store.getMeta(metaKey("daemon", "safesearch"));
    return value === undefined ? undefined : parseSafeSearch(value);
  }

  /** For every session, from their next search on. Not logged in a session: it isn't one's. */
  setSafeSearch(value: SafeSearch): void {
    this.store.setMeta(metaKey("daemon", "safesearch"), value);
  }

  /** Whether the user asked for "Show, don't tell." (off until they turn it on). */
  showDontTell(): boolean {
    return this.store.getMeta(metaKey("daemon", "show_dont_tell")) === "on";
  }

  /** For every session: each agent is told with its next prompt (see told.ts). */
  setShowDontTell(on: boolean): void {
    this.store.setMeta(metaKey("daemon", "show_dont_tell"), on ? "on" : "off");
    this.togglesChanged();
  }

  /** Whether apps draw diffs side by side (off until the user turns it on). */
  splitDiffs(): boolean {
    return this.store.getMeta(metaKey("daemon", "split_diffs")) === "on";
  }

  setSplitDiffs(on: boolean): void {
    this.store.setMeta(metaKey("daemon", "split_diffs"), on ? "on" : "off");
    this.togglesChanged();
  }

  /** Whether apps that show images draw charts as pictures (off until the user turns it on). */
  chartImages(): boolean {
    return this.store.getMeta(metaKey("daemon", "chart_images")) === "on";
  }

  setChartImages(on: boolean): void {
    this.store.setMeta(metaKey("daemon", "chart_images"), on ? "on" : "off");
    this.togglesChanged();
  }

  /** Whether apps show only the newest edit's changes (off until the user turns it on). */
  hideEdits(): boolean {
    return this.store.getMeta(metaKey("daemon", "hide_edits")) === "on";
  }

  setHideEdits(on: boolean): void {
    this.store.setMeta(metaKey("daemon", "hide_edits"), on ? "on" : "off");
    this.togglesChanged();
  }

  /**
   * Whether starting a subagent always asks: not allowed by /approvals auto nor by an earlier
   * "allow for this session" (off until the user turns it on).
   */
  alwaysAskForAgents(): boolean {
    return this.store.getMeta(metaKey("daemon", "ask_agents")) === "on";
  }

  setAlwaysAskForAgents(on: boolean): void {
    this.store.setMeta(metaKey("daemon", "ask_agents"), on ? "on" : "off");
  }

  /**
   * Whether what the user sends while a turn runs goes into it at once (steering, for harnesses
   * that take it); off until the user turns it on: it waits for the turn to end.
   */
  steer(): boolean {
    return this.store.getMeta(metaKey("daemon", "steer")) === "on";
  }

  setSteer(on: boolean): void {
    this.store.setMeta(metaKey("daemon", "steer"), on ? "on" : "off");
  }

  /** Which subagents the agents may start (both until the user picks). */
  subagents(): Subagents {
    const value = this.store.getMeta(metaKey("daemon", "subagents"));
    return (value === undefined ? undefined : parseSubagents(value)) ?? "both";
  }

  setSubagents(value: Subagents): void {
    this.store.setMeta(metaKey("daemon", "subagents"), value);
  }

  /** The /settings toggles, as apps get them (user.settings). */
  userSettings(): UserSettings {
    return {
      showDontTell: this.showDontTell(),
      splitDiffs: this.splitDiffs(),
      chartImages: this.chartImages(),
      hideEdits: this.hideEdits(),
    };
  }

  /** Calls `listener` with the toggles after each change. */
  onToggle(listener: (settings: UserSettings) => void): void {
    this.toggleListeners.push(listener);
  }

  private togglesChanged(): void {
    const settings = this.userSettings();
    for (const listener of this.toggleListeners) listener(settings);
  }

  /**
   * The session's mode, and its subagents' sessions' (delegations into a directory of their
   * own, and theirs in turn): they work the way the user said the session should.
   */
  setApprovals(session: string, value: Approvals): void {
    this.store.append(session, {
      type: "setting_changed",
      harness: "daemon",
      option: "approvals",
      value,
    });
    const rows = this.store.listSessions();
    const down = (parent: string): void => {
      for (const row of rows.filter((r) => r.parent === parent)) {
        if (this.approvals(row.id) !== value) {
          this.store.append(row.id, {
            type: "setting_changed",
            harness: "daemon",
            option: "approvals",
            value,
          });
        }
        down(row.id);
      }
    };
    down(session);
  }
}

function metaKey(harness: string, option: string): string {
  return `setting:${harness}:${option}`;
}
