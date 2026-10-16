// What the Rust side (apps/gui/src/commands.rs) offers, written down once so `invoke` checks
// names, arguments and results.
import type { DaemonAddress } from "@suite/sdk";

export interface Startup {
  /** Where the window was started. */
  cwd: string;
  /** `suite gui --session ID`. */
  session: string | null;
  /** `suite gui --new`. */
  new: boolean;
}

declare module "@neoworks/svelte-gpui" {
  interface CommandMap {
    startup: { args: null; result: Startup };
    /** Rejects with the code `NotRunning` when there is no daemon. */
    daemon_address: { args: null; result: DaemonAddress };
    git_branch: { args: { cwd: string }; result: string | null };
  }
}
