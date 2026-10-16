// The window's one registry: the `slots` service registers into it and `<Slot>` draws from it.
import { SlotRegistry } from "./registry.ts";

export const slotRegistry = new SlotRegistry();
