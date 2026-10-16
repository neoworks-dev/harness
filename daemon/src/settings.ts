// What the user chose for each harness (/model, /effort, /mode). A session's choices are
// setting_changed events in its log; the last choice per harness is kept as the default for new
// sessions. Nothing is set that the user didn't choose: each harness keeps its own defaults.
import type { ConfigCategory } from "@suite/harness/agent";
import type { SettingName } from "@suite/protocol/types";
import type { SessionStore } from "./store.ts";

export const CATEGORIES: Record<SettingName, ConfigCategory> = {
  model: "model",
  effort: "thought_level",
  mode: "mode",
};

export function isSettingName(name: string): name is SettingName {
  return Object.hasOwn(CATEGORIES, name);
}

export type HarnessSettings = Partial<Record<ConfigCategory, string>>;

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
      if (event.type === "setting_changed" && event.harness === harness) {
        out[CATEGORIES[event.option]] = event.value;
      }
    }
    return out;
  }

  set(session: string, harness: string, option: SettingName, value: string): void {
    this.store.append(session, { type: "setting_changed", harness, option, value });
    this.store.setMeta(metaKey(harness, option), value);
  }
}

function metaKey(harness: string, option: string): string {
  return `setting:${harness}:${option}`;
}
