// Finds the daemon from a program running on the same machine. The only file of the SDK that
// imports from node:, so the rest of it can run in a browser.
import { readFile } from "node:fs/promises";
import { userInfo } from "node:os";
import { dirname, join } from "node:path";
import type { DaemonAddress } from "./address.ts";
import { DisconnectedError } from "./errors.ts";

type Env = Record<string, string | undefined>;

/**
 * Where the daemon writes `daemon.json`: next to its socket, which is `$NEO_SOCKET`, else
 * `$XDG_RUNTIME_DIR/neo/bus.sock`, else `/tmp/neo-<uid>/bus.sock`.
 */
export function daemonJsonPath(env: Env = process.env): string {
  let socket = env.NEO_SOCKET;
  if (!socket && env.XDG_RUNTIME_DIR) socket = join(env.XDG_RUNTIME_DIR, "neo", "bus.sock");
  if (!socket) socket = join("/tmp", `neo-${userInfo().uid}`, "bus.sock");
  return join(dirname(socket), "daemon.json");
}

/**
 * Reads the address of the daemon running on this machine from its `daemon.json`. Use it as the
 * `address` of `connect`, which reads it again on every reconnect, since the port and token
 * change each time the daemon starts. `env` says where to look (defaults to `process.env`).
 *
 * Rejects with `DisconnectedError` when the daemon is not running or the file is not its own.
 */
export async function daemonJsonAddress(env: Env = process.env): Promise<DaemonAddress> {
  const path = daemonJsonPath(env);
  let text: string;
  try {
    text = await readFile(path, "utf8");
  } catch {
    throw new DisconnectedError(`neo daemon is not running (no ${path})`);
  }
  let info: unknown;
  try {
    info = JSON.parse(text);
  } catch {
    throw new DisconnectedError(`${path} is not a daemon info file`);
  }
  if (!isDaemonInfo(info)) throw new DisconnectedError(`${path} is not a daemon info file`);
  return { url: info.url, token: info.token };
}

function isDaemonInfo(value: unknown): value is DaemonAddress {
  return (
    typeof value === "object" &&
    value !== null &&
    "url" in value &&
    typeof value.url === "string" &&
    "token" in value &&
    typeof value.token === "string"
  );
}
