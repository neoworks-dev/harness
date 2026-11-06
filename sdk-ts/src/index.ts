// "@neoworks-dev/neo-sdk" in Bun and Node: everything browser.ts has, plus finding the daemon on
// this machine (so `connect` needs no address) and hosting an app's tools.
import type { AddressSource } from "./address.ts";
import { connect as connectTo, type Client, type ConnectOptions as Options } from "./client.ts";
import { daemonJsonAddress } from "./node.ts";

export * from "./browser.ts";
export { daemonJsonAddress, daemonJsonPath } from "./node.ts";
export { createToolServer } from "./tools/tool-server.ts";
export type {
  SkillSpec,
  ToolContent,
  ToolResult,
  ToolServer,
  ToolServerOptions,
  ToolSpec,
} from "./tools/types.ts";

/** `connect`'s options; `address` defaults to the daemon running on this machine. */
export type ConnectOptions = Omit<Options, "address"> & { address?: AddressSource };

/** Connects to the daemon: the one on this machine unless `address` says otherwise. */
export function connect(options: ConnectOptions): Promise<Client> {
  const address: AddressSource = options.address ?? (() => daemonJsonAddress());
  return connectTo({ ...options, address });
}
