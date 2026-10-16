// bun run apps/browser/main.ts [--port N]
// A rough web browser for the suite: serves a page on 127.0.0.1 (the URL goes to stderr) and
// registers the "browser" app with the daemon, waiting for it if it isn't running yet.
import { parseArgs } from "node:util";
import { startBrowserApp } from "./src/app.ts";

const { values } = parseArgs({ options: { port: { type: "string" } } });
const port = Number(values.port ?? 0);
if (!Number.isInteger(port) || port < 0 || port > 65535) {
  throw new Error(`bad --port ${values.port}`);
}

const app = await startBrowserApp({
  port,
  waitForDaemon: true,
  onListening: (url) => process.stderr.write(`suite browser: ${url}\n`),
});
process.stderr.write("browser app registered; Ctrl-C to quit\n");

let stopping = false;
function shutdown(): void {
  if (stopping) return;
  stopping = true;
  void app.stop().then(() => process.exit(0));
}
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
