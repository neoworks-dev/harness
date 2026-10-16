// What the window shows, as opposed to what the session holds (state/suite.svelte.ts). Shared
// so the keyboard shortcuts, the chat and the panels agree.
class Ui {
  /** Ctrl-R: the GitHub view instead of the chat. */
  githubOpen = $state(false);
}

export const ui = new Ui();
