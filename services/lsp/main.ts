// The lsp service: language servers as a background connector, so diagnostics and definitions
// work whether or not an editor is open. Editors use the same servers through the gateway.
//   bun run services/lsp/main.ts
import { SuiteClient } from "@suite/sdk";
import { createToolServer } from "@suite/sdk/tools";
import { ServerPool } from "./src/servers.ts";
import { createTools, templates } from "./src/tools.ts";

const log = (line: string): void => {
  process.stderr.write(`lsp: ${line}\n`);
};
const pool = new ServerPool(log);
const tools = createToolServer({
  name: "lsp",
  instructions:
    "Language servers (gopls, typescript-language-server, rust-analyzer when installed). Paths are absolute; lines are 1-based; pass the identifier as `symbol` instead of counting columns.",
  tools: createTools(pool),
});

const suite = await SuiteClient.connect({
  client: "lsp",
  waitForDaemon: true,
  reconnect: true,
  presence: {
    app: "lsp",
    kind: "service",
    title: "Language servers",
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
  void Promise.all([suite.close(), pool.shutdown(), tools.close()]).then(() => process.exit(0));
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);

// Started by the daemon: go when it goes (even if it was killed).
const daemonPid = Number(process.env.SUITE_DAEMON_PID);
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
