// Surfaces (docs/neoui.md): neoui programs of a session that stay, drawn in a fixed place of the
// client instead of in the chat. They open and close in the session's log; this reads which are
// open and runs the `ui_surface` tool, which checks a program before it shows.
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { AppComponents } from "@neoworks/neoui";
import type { SurfaceOpened, SurfacePlace } from "@neoworks-dev/neo-protocol/types";
import type { SessionStore } from "./store.ts";
import { mistakesIn, mistakesNote } from "./ui-lint.ts";

const PLACES: readonly SurfacePlace[] = ["bottom", "side", "page"];
/** A name is a short slug: it is in the ref and in /surface. */
const NAME = /^[a-z0-9][a-z0-9-]{0,39}$/;

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

/** The surfaces open in the session, by name, in the order they (last) opened. */
export function openSurfaces(
  store: Pick<SessionStore, "readTypes">,
  session: string,
): SurfaceOpened[] {
  const open = new Map<string, SurfaceOpened>();
  for (const { event } of store.readTypes(session, ["surface_opened", "surface_closed"])) {
    if (event.type === "surface_opened") {
      open.delete(event.name);
      open.set(event.name, event);
    } else if (event.type === "surface_closed") {
      open.delete(event.name);
    }
  }
  return [...open.values()];
}

/**
 * The `ui_surface` tool: `open` a surface (or replace the open one of that name, to update it),
 * `close` one, or `list` the open ones. A program with mistakes is refused with them.
 */
export function surfaceTool(
  store: Pick<SessionStore, "readTypes" | "append">,
  session: string,
  args: Record<string, unknown>,
  apps: AppComponents = {},
): CallToolResult {
  const action = args.action ?? "list";
  if (action === "list") {
    const open = openSurfaces(store, session);
    if (open.length === 0) return text("No surfaces are open.");
    return text(
      open.map((s) => `- ${s.name} (${s.place})${s.title ? `: ${s.title}` : ""}`).join("\n"),
    );
  }
  const name = args.name;
  if (typeof name !== "string" || !NAME.test(name)) {
    return text(
      "ui_surface: name is required: lower case letters, digits and -, like training-status",
      true,
    );
  }
  if (action === "close") {
    if (!openSurfaces(store, session).some((s) => s.name === name)) {
      return text(`ui_surface: no surface ${name} is open`, true);
    }
    store.append(session, { type: "surface_closed", name });
    return text(`Closed ${name}.`);
  }
  if (action !== "open") return text("ui_surface: action is open, close or list", true);
  const { source, place, title } = args;
  if (typeof source !== "string" || source.trim() === "") {
    return text("ui_surface: source is required: the neoui program", true);
  }
  const was = openSurfaces(store, session).find((s) => s.name === name);
  const where = place ?? was?.place ?? "bottom";
  if (typeof where !== "string" || !PLACES.includes(where as SurfacePlace)) {
    return text("ui_surface: place is bottom, side or page", true);
  }
  if (title !== undefined && typeof title !== "string") {
    return text("ui_surface: title must be a string", true);
  }
  const wrong = mistakesIn(`\`\`\`neoui\n${source}\n\`\`\``, apps);
  if (wrong.length > 0) {
    return text(
      `ui_surface: the program has mistakes, so it was not opened. Fix them and open it again:\n${mistakesNote(wrong, apps).split("\n").slice(1).join("\n")}`,
      true,
    );
  }
  const event: SurfaceOpened = {
    type: "surface_opened",
    name,
    place: where as SurfacePlace,
    source,
  };
  const named = title ?? was?.title;
  if (named !== undefined) event.title = named;
  store.append(session, event);
  return text(`${was ? "Updated" : "Opened"} ${name} (${event.place}).`);
}
