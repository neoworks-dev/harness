// Bus client for Bun apps. Bun's WebSocket can't dial Unix sockets, so it uses the daemon's
// loopback listener: daemon.json (next to the socket) holds the URL and a token that change on
// every daemon start, so it is re-read on each (re)connect. The protocol itself lives in
// bus-client.ts, which the browser entry (browser.ts) shares.
import { readFile } from "node:fs/promises";
import { userInfo } from "node:os";
import { dirname, join } from "node:path";
import { BusClient, DisconnectedError, type BusOptions } from "./bus-client.ts";

export { DisconnectedError, SuiteError } from "./bus-client.ts";

type Env = Record<string, string | undefined>;

export interface DaemonEndpoint {
  /** `ws://127.0.0.1:PORT/bus?token=…` */
  wsUrl: string;
  /** `http://127.0.0.1:PORT`, also serves the gateway at /mcp. */
  url: string;
  token: string;
}

/** Where the daemon puts daemon.json: next to its socket (see daemon/src/paths.ts). */
export function daemonInfoPath(env: Env = process.env): string {
  let socket = env.SUITE_SOCKET;
  if (!socket && env.XDG_RUNTIME_DIR) socket = join(env.XDG_RUNTIME_DIR, "suite", "bus.sock");
  if (!socket) socket = join("/tmp", `suite-${userInfo().uid}`, "bus.sock");
  return join(dirname(socket), "daemon.json");
}

export async function readDaemonEndpoint(env: Env = process.env): Promise<DaemonEndpoint> {
  const path = daemonInfoPath(env);
  let raw: string;
  try {
    raw = await readFile(path, "utf8");
  } catch {
    throw new DisconnectedError(`the suite daemon is not running (no ${path})`);
  }
  const info: { url?: unknown; token?: unknown } = JSON.parse(raw);
  if (typeof info.url !== "string" || typeof info.token !== "string") {
    throw new DisconnectedError(`${path} is not a daemon info file`);
  }
  const wsBase = info.url.replace(/^http/, "ws");
  return {
    url: info.url,
    token: info.token,
    wsUrl: `${wsBase}/bus?token=${encodeURIComponent(info.token)}`,
  };
}

export interface ConnectOptions extends BusOptions {
  /** Where to look for the daemon (SUITE_SOCKET etc.); defaults to process.env. */
  env?: Env;
}

export class SuiteClient extends BusClient {
  protected constructor(options: ConnectOptions) {
    super(options, async () => (await readDaemonEndpoint(options.env)).wsUrl);
  }

  /**
   * Connects, says hello and registers the presence. Rejects if the daemon isn't running, unless
   * `waitForDaemon` is set.
   */
  static connect(options: ConnectOptions): Promise<SuiteClient> {
    return BusClient.start(new SuiteClient(options));
  }
}
