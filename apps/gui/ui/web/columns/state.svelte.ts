/// <reference lib="dom" />
// The sessions open in columns beside the tab's own, in their order; kept for the tab, so a
// reload has them again.
import { addColumn, NO_COLUMNS } from "./columns.ts";

const KEY = "neo:columns";

function load(): readonly string[] {
  try {
    const kept: unknown = JSON.parse(sessionStorage.getItem(KEY) ?? "[]");
    return Array.isArray(kept) ? kept.filter((id): id is string => typeof id === "string") : NO_COLUMNS;
  } catch {
    return NO_COLUMNS;
  }
}

class Columns {
  ids = $state.raw<readonly string[]>(load());

  open(session: string, shown: string | undefined): void {
    this.set(addColumn(this.ids, session, shown));
  }

  close(session: string): void {
    this.set(this.ids.filter((id) => id !== session));
  }

  private set(ids: readonly string[]): void {
    this.ids = ids;
    try {
      sessionStorage.setItem(KEY, JSON.stringify(ids));
    } catch {
      // A tab that keeps nothing still has its columns until it is reloaded.
    }
  }
}

export const columns = new Columns();
