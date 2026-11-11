// Getting a file the user gave a browser tab to the daemon. A tab has File objects, not paths, but
// the composer attaches paths; so the file is stored in the daemon's asset store (POST /assets) and
// the path of the stored copy on the daemon's disk is what gets attached. From there on it is a
// file like any the native window attaches. Pure: `send` is `fetch`, so it is tested without a
// browser (apps/gui/test/upload.test.ts).
import type { DaemonAddress } from "@neoworks-dev/neo-sdk";

/** What `fetch` gives back, as far as an upload needs it. */
export interface Sent {
  ok: boolean;
  status: number;
  json(): Promise<unknown>;
}

/** `fetch`, as far as an upload needs it. */
export type Send = (
  url: string,
  init: { method: "POST"; headers: Record<string, string>; body: unknown },
) => Promise<Sent>;

/** A file the user gave the page (a File is one). */
export interface Upload {
  name: string;
  type: string;
  arrayBuffer(): Promise<ArrayBuffer>;
}

/** Stores `file` with the daemon and returns the path of the stored copy on the daemon's disk. */
export async function upload(file: Upload, address: DaemonAddress, send: Send): Promise<string> {
  const query = new URLSearchParams({ name: file.name });
  if (file.type !== "") query.set("mime", file.type);
  const response = await send(`${address.url.replace(/\/+$/, "")}/assets?${query.toString()}`, {
    method: "POST",
    headers: { authorization: `Bearer ${address.token}` },
    // Its bytes, not the File: the page's fetch is svelte-gpui's, through the host, and that
    // takes no Blob.
    body: new Uint8Array(await file.arrayBuffer()),
  });
  if (!response.ok) {
    throw new Error(`Could not upload ${file.name}: the daemon said ${response.status}`);
  }
  const asset = (await response.json()) as { path?: unknown };
  if (typeof asset.path !== "string") {
    throw new Error(`Could not upload ${file.name}: no path came back`);
  }
  return asset.path;
}

/** `upload` for each file, in order. One that fails fails them all, and none is attached. */
export function uploadAll(
  files: readonly Upload[],
  address: DaemonAddress,
  send: Send,
): Promise<string[]> {
  return Promise.all(files.map((file) => upload(file, address, send)));
}
