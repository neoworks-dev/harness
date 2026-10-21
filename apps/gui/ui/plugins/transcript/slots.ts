// The slots the transcript declares: how one row of the chat is drawn, and how one item is, so
// another plugin can draw a kind of its own or add to the user's message.
import type { ItemKind, ViewItem } from "@neoworks-dev/neo-protocol/types";
import type { Keyed, List } from "../../kernel/slots/types.ts";
import type { Row, RowKind } from "./rows.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** One row of the chat, by its kind: an item, a reply, a group of work, a fold, the clock. */
    "chat.row": Keyed<RowKind, { row: Row }>;
    /** The renderer of one item, by its kind; the fallback draws a kind nobody claimed. */
    "chat.item": Keyed<ItemKind, { item: ViewItem }>;
    /** After a user message's text, such as the app it came from. */
    "chat.item.header": List<{ item: ViewItem }>;
  }
}
