// bun run renderers/web/demo/serve.ts [--port N]
// Serves the demo chat page on 127.0.0.1 (see ../README.md).
import { parseArgs } from "node:util";
import { startDemoServer } from "./server.ts";

const { values } = parseArgs({ options: { port: { type: "string" } } });
const port = Number(values.port ?? 0);
if (!Number.isInteger(port) || port < 0 || port > 65535)
  throw new Error(`bad --port ${values.port}`);
const server = await startDemoServer({ port });
process.stdout.write(`neo web demo: ${server.url}\n`);
