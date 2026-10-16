// Where the daemon is and how to get in. Pure string handling, so it works in a browser too.

/** The daemon's loopback listener and the token that lets a client in. */
export interface DaemonAddress {
  /** `http://127.0.0.1:PORT` or `ws://127.0.0.1:PORT` (https and wss work as well). */
  url: string;
  token: string;
}

/**
 * Finds the daemon's current address. `connect` calls it on every (re)connect, because a
 * restarted daemon has a new port and token.
 */
export type AddressSource = () => Promise<DaemonAddress>;

/** An address source that always answers with `address`, for a daemon you were told about. */
export function fixedAddress(address: DaemonAddress): AddressSource {
  return () => Promise.resolve(address);
}

/** The WebSocket URL for the bus: `ws://host:port/bus?token=…`. */
export function busUrl({ url, token }: DaemonAddress): string {
  const base = url.replace(/^http/, "ws").replace(/\/+$/, "");
  return `${base}/bus?token=${encodeURIComponent(token)}`;
}

const ASSET_HANDLE = /^asset:\/\/([a-z]+-[0-9a-f]{12})$/;

/**
 * The URL a page can load an `asset://` handle from (the daemon's `GET /assets/<id>`), with the
 * token in the query because an `<img>` cannot send headers. Returns undefined for anything that
 * is not an asset handle. The URL contains the daemon token: only give it to the page's own DOM.
 */
export function assetUrl({ url, token }: DaemonAddress, handle: string): string | undefined {
  const id = ASSET_HANDLE.exec(handle)?.[1];
  if (id === undefined) return undefined;
  const base = url.replace(/^ws/, "http").replace(/\/+$/, "");
  return `${base}/assets/${id}?token=${encodeURIComponent(token)}`;
}
