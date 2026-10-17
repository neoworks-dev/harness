// The slots the shell declares: where the window's panes go.
import type { Chain, List } from "../../kernel/slots/types.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** Columns before the conversation, such as the sessions. */
    "shell.left": List;
    /** The bar over the conversation: what it shows at its start (where we are). */
    "shell.top": List;
    /** The bar over the conversation: buttons at its end. */
    "shell.top.right": List;
    /** The strip of buttons at the window's right edge, such as the panels' toggles. */
    "shell.toolbar": List;
    /** What fills the middle: the first entry whose `select` says yes draws, else the fallback. */
    "shell.main": Chain;
    /** Rows under the main pane, such as the input and the status line. */
    "shell.bottom": List;
    /** Columns after the conversation. */
    "shell.right": List;
    /** Floating cards at the conversation's top right, 12px from its edges. */
    "shell.card": List;
    /** Drawn over everything. */
    "shell.overlay": List;
  }
}
