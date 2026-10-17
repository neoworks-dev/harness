// The comments waiting for the user's next message. They belong to the session that is shown:
// another one starts with none.
import { untrack } from "svelte";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { covers, formatMessage, type CodeComment } from "../../lib/comments.ts";

class Comments {
  list = $state.raw<readonly CodeComment[]>([]);
  private serial = 0;

  /** Drops the comments when another session is shown, until the returned call stops it. */
  start(daemon: DaemonService): () => void {
    return $effect.root(() => {
      $effect(() => {
        void daemon.sessionId;
        untrack(() => (this.list = []));
      });
    });
  }

  add(comment: Omit<CodeComment, "id">): void {
    this.serial += 1;
    this.list = [...this.list, { ...comment, id: `c${this.serial}` }];
  }

  remove(id: string): void {
    this.list = this.list.filter((comment) => comment.id !== id);
  }

  /** Whether a comment waits on this line. */
  marked(path: string, side: "new" | "old", no: number): boolean {
    return this.list.some((comment) => covers(comment, path, side, no));
  }

  /** `text` with every comment as a block after it; the comments are spent. */
  take(text: string): string {
    const message = formatMessage(text, this.list);
    this.list = [];
    return message;
  }
}

export const comments = new Comments();
