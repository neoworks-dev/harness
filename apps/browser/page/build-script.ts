// bun run apps/browser/page/build-script.ts > app.js
// Prints the page's bundled browser script. Tests run it in a child process: Bun's bundler
// misbehaves in a process that already started and stopped daemons (see
// renderers/web/demo/build-script.ts).
import { buildPageScript } from "../src/server.ts";

process.stdout.write(await buildPageScript());
