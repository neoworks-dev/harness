// Which surfaces go where, and which page is shown. Plain functions, tested without a window.
import type { SurfacePlace, ViewItem } from "@neoworks-dev/neo-protocol/types";

/** The open surfaces of one place, in the order they opened. */
export function inPlace(surfaces: readonly ViewItem[], place: SurfacePlace): ViewItem[] {
  return surfaces.filter((surface) => surface.place === place);
}

/**
 * The page in the chat's place after the surfaces changed: a page that opened since (`seen` did
 * not have it) is shown at once; the one shown stays while it is open; else none, the chat.
 */
export function pageAfter(
  shown: string | undefined,
  surfaces: readonly ViewItem[],
  seen: ReadonlySet<string>,
): string | undefined {
  const pages = inPlace(surfaces, "page");
  const fresh = pages.filter((page) => !seen.has(page.ref)).at(-1);
  if (fresh !== undefined) return fresh.ref;
  return pages.some((page) => page.ref === shown) ? shown : undefined;
}

/** What a surface is called: its title (the item's label), else its name. */
export function titleOf(surface: ViewItem): string {
  return surface.label ?? surface.ref.replace(/^s-/, "");
}
