/// <reference lib="dom" />
// Several sessions side by side, in a browser tab: each column beside the tab's own is the same
// page opened on another session in a frame. The sidebar's "Open in a new column" adds one; a
// column asks the tab around it, so columns never nest. Imported by main.web.ts before main.ts, so
// the plugin is there when the window boots; the native window has none of this.
import type { Context } from "@neoworks/extension-system";
import { target } from "../../kernel/target.ts";
import "../../plugins/shell/slots.ts";
import { plugins } from "../../plugins/index.ts";
import Columns from "./Columns.svelte";
import { columns } from "./state.svelte.ts";

/** What a column posts to the tab around it to open a session beside. */
interface OpenBeside {
  neoOpenColumn: string;
}

function isOpenBeside(data: unknown): data is OpenBeside {
  return typeof data === "object" && data !== null && typeof (data as OpenBeside).neoOpenColumn === "string";
}

if (target.column) {
  target.openBeside = (session) => {
    window.top?.postMessage({ neoOpenColumn: session } satisfies OpenBeside, location.origin);
  };
} else {
  plugins.push({
    name: "columns",
    inject: ["slots", "daemon"],
    apply(ctx: Context) {
      const { slots, daemon } = ctx;
      target.openBeside = (session) => columns.open(session, daemon.sessionId);
      const listen = (event: MessageEvent): void => {
        if (event.origin === location.origin && isOpenBeside(event.data)) {
          columns.open(event.data.neoOpenColumn, daemon.sessionId);
        }
      };
      window.addEventListener("message", listen);
      ctx.effect(() => () => {
        window.removeEventListener("message", listen);
        target.openBeside = undefined;
      }, "columns: open beside");
      // After the conversation and its side panels, at the window's right edge.
      slots.inject("shell.right", (slot) => {
        slot.add({ component: Columns, order: 100 });
      });
    },
  });
}
