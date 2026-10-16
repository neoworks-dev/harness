// The slots the composer declares: places in the row of the input for other plugins' controls.
import type { List } from "../../kernel/slots/types.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** Before the input's prompt mark. */
    "composer.left": List;
    /** After the input. */
    "composer.right": List;
  }
}
