// The status line under the input. It declares `statusline.right` for other plugins to put short
// messages in.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import StatusLine from "./StatusLine.svelte";
import "./slots.ts";

export const statusLine = {
  name: "statusline",
  inject: ["slots", "daemon"],
  apply({ slots }: Context) {
    slots.inject("shell.bottom", (slot) => {
      slot.add({ component: StatusLine, order: 30, slots: { "statusline.right": "list" } });
    });
  },
};
