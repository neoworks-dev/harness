// What the session's harnesses offer (models, reasoning levels), asked for when a picker opens.
// Asking may start a guest harness's adapter, so it is not asked before the user wants to pick.
import type { ConfigOption, HarnessSettings } from "@neoworks-dev/neo-protocol/types";
import type { DaemonService } from "../../kernel/services/daemon.ts";

class Settings {
  harnesses = $state.raw<readonly HarnessSettings[]>([]);
  loading = $state(false);
  error = $state<string>();
  /** The session the list is for; another session asks again. */
  private session: string | undefined;

  /** Asks again, for the shown session. */
  async load(daemon: DaemonService): Promise<void> {
    const session = daemon.sessionId;
    if (session !== this.session) this.harnesses = [];
    this.session = session;
    this.loading = true;
    this.error = undefined;
    try {
      const reply = await daemon.settings();
      if (this.session === session) this.harnesses = reply?.harnesses ?? [];
    } catch (thrown) {
      this.error = thrown instanceof Error ? thrown.message : String(thrown);
    } finally {
      this.loading = false;
    }
  }
}

/** The harness's values for one setting, and its current one. */
export function optionOf(
  harness: HarnessSettings | undefined,
  category: "model" | "thought_level",
): ConfigOption | undefined {
  return harness?.options.find((option) => option.category === category);
}

export const settings = new Settings();
