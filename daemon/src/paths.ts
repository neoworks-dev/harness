// Where the daemon keeps its socket and data. The Go CLI resolves the socket the same way.
import { homedir, userInfo } from "node:os";
import { dirname, join } from "node:path";

export interface DaemonPaths {
  socket: string;
  /** Discovery file for clients that can't use the socket: TCP url + token. */
  info: string;
  dataDir: string;
  db: string;
}

export function resolvePaths(env: Record<string, string | undefined> = process.env): DaemonPaths {
  let socket = env.NEO_SOCKET;
  if (!socket && env.XDG_RUNTIME_DIR) socket = join(env.XDG_RUNTIME_DIR, "neo", "bus.sock");
  if (!socket) socket = join("/tmp", `neo-${userInfo().uid}`, "bus.sock");

  let dataDir = env.NEO_DATA_DIR;
  if (!dataDir && env.XDG_DATA_HOME) dataDir = join(env.XDG_DATA_HOME, "neo");
  if (!dataDir) dataDir = join(homedir(), ".local", "share", "neo");

  return {
    socket,
    info: join(dirname(socket), "daemon.json"),
    dataDir,
    db: join(dataDir, "sessions.db"),
  };
}
