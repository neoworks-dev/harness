// What the user chose for this session: each harness's settings (/model, /effort, /mode), the
// harness the session runs on (/harness) and the suite's approvals mode (/approvals). They are
// setting_changed events in the session's log; the last harness settings and the last harness
// choice are kept as the defaults for new sessions (approvals is not: a new session asks).
// Nothing is set that the user didn't choose: each harness keeps its own defaults.
import type { ConfigCategory } from "@suite/harness/agent";
import type { SettingName } from "@suite/protocol/types";
import type { SessionStore } from "./store.ts";

/** The settings that belong to a harness (approvals and harness are the suite's own). */
export type HarnessSettingName = Exclude<SettingName, "approvals" | "harness">;

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

export class Settings {
  /** `main`: the harness a session runs on until the user picks another (the daemon's agent). */
  constructor(
    private readonly store: SessionStore,
    private readonly main = "pi",
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
      harness: "suite",
      option: "harness",
      value,
    });
    this.store.setMeta(metaKey("suite", "harness"), value);
  }

  /**
   * A new session starts on the user's last harness choice, if it still exists. Logged when it
   * isn't main, so the session keeps it whatever the user picks elsewhere later (and panes see
   * it).
   */
  inheritHarness(session: string, exists: (harness: string) => boolean): void {
    const last = this.store.getMeta(metaKey("suite", "harness"));
    if (last === undefined || last === this.main || !exists(last)) return;
    this.store.append(session, {
      type: "setting_changed",
      harness: "suite",
      option: "harness",
      value: last,
    });
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

  setApprovals(session: string, value: Approvals): void {
    this.store.append(session, {
      type: "setting_changed",
      harness: "suite",
      option: "approvals",
      value,
    });
  }
}

function metaKey(harness: string, option: string): string {
  return `setting:${harness}:${option}`;
}
