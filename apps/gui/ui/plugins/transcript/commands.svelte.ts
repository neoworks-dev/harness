// Which commands show their output without being pressed: one that runs, so what it prints is in
// sight as it comes; and one that finished while the user was scrolled up, so the chat they are
// reading doesn't fold under them. One that finished while the chat followed its end folds.
// Watched here, not in the rows: the list builds rows only near the window.
import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { opened } from "./opened.svelte.ts";

/** A shell command's tool item that hasn't finished. */
function running(item: ViewItem): boolean {
  return item.kind === "tool" && item.command !== undefined && (item.state === "running" || item.state === "pending");
}

class OpenCommands {
  /** The chat is at its end, following what comes (Chat.svelte says so as it scrolls). */
  following = $state(true);
  /** Commands that finished while the user was scrolled up: they stay open. */
  private held = $state.raw<ReadonlySet<string>>(new Set());
  private runningRefs = new Set<string>();
  private session: string | undefined;

  /** Whether the command `ref`, running or not as `live` says, shows its output unpressed. */
  isOpen(ref: string, live: boolean): boolean {
    return live || this.held.has(ref);
  }

  /** Follows the session's items until the returned call stops it. */
  follow(daemon: DaemonService): () => void {
    return $effect.root(() => {
      $effect(() => {
        const items = daemon.items;
        const session = daemon.sessionId;
        if (session !== this.session) {
          // Another session: what was running in this one is no news.
          this.session = session;
          this.runningRefs = new Set(items.filter(running).map((item) => item.ref));
          this.held = new Set();
          return;
        }
        const now = new Set(items.filter(running).map((item) => item.ref));
        const ended = [...this.runningRefs].filter((ref) => !now.has(ref));
        this.runningRefs = now;
        if (ended.length === 0) return;
        if (!this.following) {
          this.held = new Set([...this.held, ...ended]);
          return;
        }
        // Folding now: one the user closed while it ran stays closed (a press turns the default over).
        for (const ref of ended) {
          const key = `${session}/${ref}`;
          if (opened.has(key)) opened.toggle(key);
        }
      });
    });
  }
}

export const openCommands = new OpenCommands();
