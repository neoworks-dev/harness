// How large the chat is drawn: a factor on everything in it (text, code, cards), as CSS's zoom.
// A browser tab starts larger than the window, whose text is the system's size. Plain functions.

export const ZOOM_MIN = 0.8;
export const ZOOM_MAX = 1.6;
const STEP = 0.1;

/** Where the chat starts until the user changes it. */
export function defaultZoom(web: boolean): number {
  return web ? 1.15 : 1;
}

/** One step larger (`by` 1) or smaller (-1), kept within bounds and to two decimals. */
export function stepZoom(zoom: number, by: 1 | -1): number {
  const next = Math.round((zoom + by * STEP) * 100) / 100;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, next));
}

/** The zoom a settings file holds, or undefined when it holds none that can be used. */
export function parseZoom(text: string): number | undefined {
  try {
    const zoom = (JSON.parse(text) as { chatZoom?: unknown }).chatZoom;
    if (typeof zoom !== "number" || !Number.isFinite(zoom)) return undefined;
    return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom));
  } catch {
    return undefined;
  }
}

/** "115%". */
export function zoomLabel(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}
