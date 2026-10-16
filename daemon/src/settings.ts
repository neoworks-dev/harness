// What the user chose for this session: each harness's settings (/model, /effort, /mode) and the
// suite's approvals mode (/approvals). They are setting_changed events in the session's log; the
// last harness choice is kept as the default for new sessions (approvals is not: a new session
// asks). Nothing is set that the user didn't choose: each harness keeps its own defaults.
import type { ConfigCategory } from "@suite/harness/agent";
import type { SettingName } from "@suite/protocol/types";
import type { SessionStore } from "./store.ts";

/** The settings that belong to a harness (approvals is the suite's own). */
export type HarnessSettingName = Exclude<SettingName, "approvals">;

export const CATEGORIES: Record<HarnessSettingName, ConfigCategory> = {
  model: "model",
  effort: "thought_level",
  mode: "mode",
};

export function isSettingName(name: string): name is HarnessSettingName {
  return Object.hasOwn(CATEGORIES, name);
}

export type HarnessSettings = Partial<Record<ConfigCategory, string>>;

export type Approvals = "ask" | "auto";

export class Settings {
  constructor(private readonly store: SessionStore) {}

  /** This session's choices for a harness, over the user's last choices. */
  get(session: string, harness: string): HarnessSettings {
    const out: HarnessSettings = {};
    for (const [name, category] of Object.entries(CATEGORIES)) {
      const last = this.store.getMeta(metaKey(harness, name));
      if (last !== undefined) out[category] = last;
    }
    for (const { event } of this.store.readTypes(session, ["setting_changed"])) {
      if (event.type !== "setting_changed" || event.harness !== harness) continue;
      if (event.option !== "approvals") out[CATEGORIES[event.option]] = event.value;
    }
    return out;
  }

  set(session: string, harness: string, option: HarnessSettingName, value: string): void {
    this.store.append(session, { type: "setting_changed", harness, option, value });
    this.store.setMeta(metaKey(harness, option), value);
  }

  /**
   * Whether guests' permission requests and starting a guest ask the user (ask) or are
   * allowed (auto). Per session and never carried over: a new session asks.
   */
  approvals(session: string): Approvals {
    let value: Approvals = "ask";
    for (const { event } of this.store.readTypes(session, ["setting_changed"])) {
      if (event.type === "setting_changed" && event.option === "approvals") {
        value = event.value === "auto" ? "auto" : "ask";
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
