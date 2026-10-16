import type { List } from "../../kernel/slots/types.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** The panels of the column at the right, top to bottom. */
    "sidepanel.section": List;
  }
}
