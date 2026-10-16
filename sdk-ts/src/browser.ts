// Bus client for browsers: the same protocol client as the Bun entry, but it is told where the
// daemon is (`url` + `token`, e.g. from daemon.json read by the page's server) instead of reading
// daemon.json. Imports nothing from node.
import { BusClient, type BusOptions } from "./bus-client.ts";

export interface DaemonAddress {
  /** `http://127.0.0.1:PORT` or `ws://127.0.0.1:PORT`: the daemon's TCP listener. */
  url: string;
  token: string;
}

export interface BrowserConnectOptions extends BusOptions, DaemonAddress {
  /**
   * Asked for a fresh address on every reconnect, since a restarted daemon has a new port and
   * token. Without it the original `url` and `token` are reused.
   */
  refreshAddress?: () => Promise<DaemonAddress>;
}

/** `ws://host:port/bus?token=…` for a base URL (http, https, ws or wss) and a token. */
export function busUrl({ url, token }: DaemonAddress): string {
  const base = url.replace(/^http/, "ws").replace(/\/+$/, "");
  return `${base}/bus?token=${encodeURIComponent(token)}`;
}

const ASSET_HANDLE = /^asset:\/\/([a-z]+-[0-9a-f]{12})$/;

/**
 * The URL a page can load an `asset://` handle from: the daemon's `GET /assets/<id>`, with the
 * token in the query since an `<img>` can't send headers. Undefined for anything that is not a
 * handle. The URL carries the daemon token: only hand it to the page's own DOM.
 */
export function assetUrl({ url, token }: DaemonAddress, handle: string): string | undefined {
  const id = ASSET_HANDLE.exec(handle)?.[1];
  if (id === undefined) return undefined;
  const base = url.replace(/^ws/, "http").replace(/\/+$/, "");
  return `${base}/assets/${id}?token=${encodeURIComponent(token)}`;
}

export class BrowserClient extends BusClient {
  protected constructor(options: BrowserConnectOptions) {
    let first = true;
    super(options, async () => {
      if (first || !options.refreshAddress) {
        first = false;
        return busUrl(options);
      }
      return busUrl(await options.refreshAddress());
    });
  }

  static connect(options: BrowserConnectOptions): Promise<BrowserClient> {
    return BusClient.start(new BrowserClient(options));
  }
}
