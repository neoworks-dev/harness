// What the transcript shows beyond the items: whether results and thoughts are shown in full,
// and which item another pane is answering. What the user opened by pressing a row is in
// opened.svelte.ts.
import type { ViewItem } from "@neoworks-dev/neo-protocol/types";

class Transcript {
  /** Ctrl-O: tool results and cards in full. */
  expanded = $state(false);
  /**
   * The agent's thoughts, as text in the chat (Ctrl-T hides them to one-line rows); in another
   * agent's chat, shown at all.
   */
  showThoughts = $state(true);
  /**
   * The ref of the item (a permission or choice card) a pane answers in place of the transcript;
   * the item shows a one-line stub for it.
   */
  stubbed = $state<string>();
  /** The reply under the pointer: its rows are apart, its copy button shows for any of them. */
  hovered = $state<string>();

  /** The items to draw: thoughts only while they are shown. */
  visible(items: readonly ViewItem[]): ViewItem[] {
    return items.filter((item) => item.kind !== "thought" || this.showThoughts);
  }

  /** Puts a stub in place of item `ref`; returns the call that brings the item back. */
  stub(ref: string): () => void {
    this.stubbed = ref;
    return () => (this.stubbed = undefined);
  }
}

export const transcript = new Transcript();
