import type { DiffComponent } from "@suite/protocol/types";

// What the overlay shows: a diff over the whole window, until it is closed.
class Viewing {
  current = $state.raw<DiffComponent>();
}

export const viewing = new Viewing();
