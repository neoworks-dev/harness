// The pictures of the chat as a gallery: which there are, in order, and where a click on one
// starts it. Plain functions; the open gallery is in gallery.svelte.ts, drawn by Gallery.svelte.
import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
import { baseName, imagesOf, oneLine } from "./components.ts";

/** One picture of the gallery: where to load it from, and what to call it. */
export interface Picture {
  src: string;
  name: string;
  caption?: string;
}

/** Every picture in the chat's items, top to bottom: the user's, and those tools brought back. */
export function chatPictures(
  items: readonly ViewItem[],
  resolve: (src: string) => string,
): Picture[] {
  return items.flatMap((item) =>
    imagesOf(item.component).map((image) => ({
      src: resolve(image.src),
      name: image.alt ? oneLine(image.alt) : baseName(image.src),
      ...(image.caption ? { caption: image.caption } : {}),
    })),
  );
}

/**
 * The gallery a click on `picked` opens: the chat's pictures, starting at it; just the one when it
 * is not among them (a picture in a doc of the agent's).
 */
export function galleryAt(
  pictures: readonly Picture[],
  picked: Picture,
): { pictures: Picture[]; index: number } {
  const index = pictures.findIndex((picture) => picture.src === picked.src);
  return index < 0 ? { pictures: [picked], index: 0 } : { pictures: [...pictures], index };
}

/** The index `by` steps from `index` among `count`, going round at the ends. */
export function stepIndex(index: number, count: number, by: number): number {
  if (count === 0) return 0;
  return (((index + by) % count) + count) % count;
}
