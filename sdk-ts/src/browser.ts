// What a browser gets of "@neoworks-dev/neo-sdk" (the "browser" export condition): the client and
// the view, with no node: imports and no MCP SDK. Here `connect` needs an `address`; index.ts is
// the same plus finding the local daemon and hosting tools.
export { connect, type Client, type ConnectOptions, type InputOptions } from "./client.ts";
export type { ConnectionState } from "./connection.ts";
export type { SubscribeOptions, ViewSubscribeOptions } from "./subscriptions.ts";
export type { Unsubscribe, Watched } from "./watched.ts";
export { DisconnectedError, SuiteError, TimeoutError } from "./errors.ts";
export {
  assetUrl,
  busUrl,
  fixedAddress,
  type AddressSource,
  type DaemonAddress,
} from "./address.ts";
export { componentAtPath, hasChildren, ViewState, type ViewChange } from "./view.ts";
