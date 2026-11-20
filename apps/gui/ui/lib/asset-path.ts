// A file in the daemon's asset store named by its path, as an attachment is (lib/upload.ts
// attaches the stored copy's path): the store keeps `<data>/assets/<id>.<ext>`, which the daemon
// serves as /assets/<id>. A browser tab cannot read a path on the daemon's disk, so a picture
// attached there would not load; its asset handle can. Plain, tested without a window.

/** An absolute path (not a URL) ending in the store's `assets/<id>.<ext>`. */
const STORED = /^\/(?:.*\/)?assets\/([a-z]+-[0-9a-f]{12})\.[A-Za-z0-9]+$/;

/** `asset://<id>` for a path into the asset store; the source as it is otherwise. */
export function assetHandleOf(src: string): string {
  const id = STORED.exec(src)?.[1];
  return id === undefined ? src : `asset://${id}`;
}
