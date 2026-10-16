// The `/` menu's entries: the daemon's commands, and the ones plugins add for the window to run.
// Reactive, so the menu follows a plugin loading or unloading.
import { COMMANDS, type Command } from "../lib/commands.ts";

class CommandStore {
  /** In the order the menu lists them. */
  list = $state.raw<readonly Command[]>(COMMANDS);

  /** Adds an entry after the others; returns the call that takes it away. */
  add(command: Command): () => void {
    this.list = [...this.list, command];
    return () => (this.list = this.list.filter((entry) => entry !== command));
  }
}

export const commandStore = new CommandStore();
