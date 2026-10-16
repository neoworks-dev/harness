import type { List } from "../../kernel/slots/types.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** After the segments, before the error: short messages from other plugins. */
    "statusline.right": List;
  }
}
