// The slots the shell declares: where the window's panes go.
import type { Chain, List } from "../../kernel/slots/types.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** Columns before the conversation, such as the sessions. */
    "shell.left": List;
    /** What fills the middle: the first entry whose `select` says yes draws, else the fallback. */
    "shell.main": Chain;
    /** Rows under the main pane, such as the input and the status line. */
    "shell.bottom": List;
    /** Columns after the conversation. */
    "shell.right": List;
    /** Drawn over everything. */
    "shell.overlay": List;
  }
}
