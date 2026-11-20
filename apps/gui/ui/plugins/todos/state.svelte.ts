// The shown session's shared todo list, from its events.
import type { TodoItem } from "@neoworks-dev/neo-protocol/types";
import { Folded } from "../../kernel/folded.svelte.ts";
import type { DaemonService } from "../../kernel/services/daemon.ts";
import { NO_TODOS } from "./todos.ts";

class Todos {
  private readonly fold = new Folded<readonly TodoItem[]>(NO_TODOS, (items, { event }) =>
    event.type === "todos" ? event.items : items,
  );

  get items(): readonly TodoItem[] {
    return this.fold.value;
  }

  /** Folds the session's events until the plugin unloads. */
  follow(daemon: DaemonService): void {
    this.fold.follow(daemon);
  }
}

export const todos = new Todos();
