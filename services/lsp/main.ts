// The lsp service: language servers as a background connector, so diagnostics and definitions
// work whether or not an editor is open. Editors use the same servers through the gateway.
//   bun run services/lsp/main.ts
import { connect } from "@neoworks-dev/neo-sdk";
import { daemonJsonAddress } from "@neoworks-dev/neo-sdk";
import { createToolServer } from "@neoworks-dev/neo-sdk";
import { pluginLanguages } from "./src/plugins.ts";
import { ServerPool } from "./src/servers.ts";
import { createTools, templates } from "./src/tools.ts";

const log = (line: string): void => {
  process.stderr.write(`lsp: ${line}\n`);
};
const pool = new ServerPool(log, () => pluginLanguages());
const tools = await createToolServer({
  name: "lsp",
  instructions:
    "Language servers (gopls, typescript-language-server, rust-analyzer when installed, and those of installed plugins). Paths are absolute; lines are 1-based; pass the identifier as `symbol` instead of counting columns.",
  tools: createTools(pool),
});

const neo = await connect({
  client: "lsp",
  address: daemonJsonAddress,
  waitForDaemon: true,
  reconnect: true,
  presence: {
    app: "lsp",
    kind: "service",
    title: "Language servers",
    description:
      "Language servers: diagnostics, definitions, references, hover and rename for Go, TypeScript and Rust files.",
    open: true,
    mcp: tools.endpoint,
    templates,
    launch: { command: [process.execPath, "run", import.meta.path] },
  },
});
log("registered; Ctrl-C to quit");

let stopping = false;
const stop = (): void => {
  if (stopping) return;
  stopping = true;
  void Promise.all([neo.close(), pool.shutdown(), tools.close()]).then(() => process.exit(0));
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);

// Started by the daemon: go when it goes (even if it was killed).
const daemonPid = Number(process.env.NEO_DAEMON_PID);
if (Number.isInteger(daemonPid) && daemonPid > 0) {
  setInterval(() => {
    try {
      process.kill(daemonPid, 0);
    } catch {
      log("daemon is gone; exiting");
      stop();
    }
  }, 1000).unref();
}
