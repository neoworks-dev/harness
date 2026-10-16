// The slots the transcript declares: how one chat item is drawn, so another plugin can draw a
// kind of its own or add to the user's message.
import type { ItemKind, ViewItem } from "@suite/protocol/types";
import type { Keyed, List } from "../../kernel/slots/types.ts";

declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    /** The renderer of one item, by its kind; the fallback draws a kind nobody claimed. */
    "chat.item": Keyed<ItemKind, { item: ViewItem }>;
    /** After a user message's text, such as the app it came from. */
    "chat.item.header": List<{ item: ViewItem }>;
  }
}
