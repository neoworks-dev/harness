// The open secret question and what is typed for it. The text lives here and nowhere else: it is
// not in the composer's draft or history, it is not stored, and it is cleared when it is sent.
import type { SecretRequest } from "@neoworks-dev/neo-protocol/types";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { incoming, stale, submitted, type Ask } from "./ask.ts";

class SecretField {
  /** The question on screen, if any. */
  ask = $state.raw<Ask>();
  /** What is typed, bound to the masked field. */
  value = $state("");

  private daemon?: DaemonService;

  /** Follows the daemon's questions until the returned call stops it. */
  start(daemon: DaemonService): () => void {
    this.daemon = daemon;
    const stopAsking = daemon.onSecretRequest((request) => this.arrived(request));
    const stopWatching = $effect.root(() => {
      $effect(() => {
        const open = this.ask;
        if (open === undefined) return;
        const why = stale(open, daemon.sessionId, daemon.connection);
        // Nothing can be sent over a dead connection; the daemon dropped the question with it.
        if (why === "connection") this.close();
        else if (why === "session") this.cancel();
      });
    });
    return () => {
      stopAsking();
      stopWatching();
      this.close();
      this.daemon = undefined;
    };
  }

  private arrived(request: SecretRequest): void {
    const next = incoming(this.ask, request, this.daemon?.sessionId);
    for (const id of next.cancel) this.daemon?.answerSecret(id);
    if (next.show !== this.ask) {
      this.value = "";
      this.ask = next.show;
    }
  }

  /** Enter: sends the text. An empty field stays open. */
  submit(): void {
    const open = this.ask;
    if (open === undefined) return;
    const value = submitted(this.value);
    if (value === undefined) return;
    this.close();
    this.daemon?.answerSecret(open.id, value);
  }

  /** Esc: answers without a value. */
  cancel(): void {
    const open = this.ask;
    if (open === undefined) return;
    this.close();
    this.daemon?.answerSecret(open.id);
  }

  /** Forgets the question and the text. */
  private close(): void {
    this.value = "";
    this.ask = undefined;
  }
}

export const secretField = new SecretField();
