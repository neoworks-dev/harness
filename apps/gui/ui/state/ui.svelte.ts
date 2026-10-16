// What the window shows, as opposed to what the session holds (state/suite.svelte.ts). Shared
// so the keyboard shortcuts, the chat and the panels agree.
class Ui {
  /** Ctrl-O: tool results and cards in full. */
  expanded = $state(false);
  /** Ctrl-T: the agent's thoughts. */
  showThoughts = $state(false);
  /** Ctrl-X: the agents list under the input has the keyboard. */
  agentsFocused = $state(false);
  /** Ctrl-R: the GitHub view instead of the chat. */
  githubOpen = $state(false);
  /**
   * The ref of the chat item (a permission or choice card) the panel under the chat is answering
   * in place of the input; the chat shows a one-line stub for it. Set by the input side.
   */
  panelRef = $state<string>();
}

export const ui = new Ui();
