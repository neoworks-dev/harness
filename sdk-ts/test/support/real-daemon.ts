// A real daemon (echo agent, no services, no guests) in a temp socket and data dir, and clients
// that find it through that dir's daemon.json the way an app on the machine would.
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startTestDaemon, type TestDaemon } from "@neoworks-dev/neo-daemon/testing";
import { connect, type Client, type ConnectOptions } from "../../src/index.ts";
import { daemonJsonAddress } from "../../src/node.ts";

export type Later = (undo: () => Promise<unknown> | void) => void;

/** A directory with no daemon in it. */
export function emptyDaemonDir(): string {
  return mkdtempSync(join(tmpdir(), "neo-none-"));
}

/** Finds the daemon running in `dir`, again on every call: a restarted daemon is found too. */
export function addressIn(dir: string): ConnectOptions["address"] {
  return () => daemonJsonAddress({ NEO_SOCKET: join(dir, "bus.sock") });
}

/**
 * Starts a daemon in a fresh dir, or in `dir` to restart one. Stopped after the test; a test may
 * stop it earlier (`stop` is safe to call twice).
 */
export async function startDaemon(later: Later, dir?: string): Promise<TestDaemon> {
  const started = await startTestDaemon({ dir });
  let stopping: Promise<void> | undefined;
  const stop = (timeoutMs = 200): Promise<void> => {
    stopping ??= started.stop(timeoutMs);
    return stopping;
  };
  later(() => stop());
  return { ...started, stop };
}

/** Connects to the daemon in `daemon.dir`; closed after the test. */
export async function connectTo(
  later: Later,
  daemon: TestDaemon,
  options: Partial<ConnectOptions> = {},
): Promise<Client> {
  const client = await connect({ client: "test", address: addressIn(daemon.dir), ...options });
  later(() => client.close());
  return client;
}

export interface Rig {
  daemon: TestDaemon;
  client: Client;
}

/** A daemon and a client connected to it. */
export async function startRig(later: Later): Promise<Rig> {
  const daemon = await startDaemon(later);
  const client = await connectTo(later, daemon);
  return { daemon, client };
}
