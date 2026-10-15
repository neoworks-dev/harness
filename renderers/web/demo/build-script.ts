// bun run renderers/web/demo/build-script.ts > app.js
// Prints the demo's bundled browser script. Tests run it in a child process: Bun 1.3.11's bundler
// reads garbage when it runs in a process that already started and stopped daemons (it shares file
// descriptors with their SQLite stores).
import { buildDemoScript } from "./server.ts";

process.stdout.write(await buildDemoScript());
