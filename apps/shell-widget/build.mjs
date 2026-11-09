// Bundles the widget into dist/client.js and puts its manifest next to it: the folder neoshell
// reads from <data dir>/neoshell/extensions/dev.neoworks.agents/.
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { buildWidget } from "@neoworks/neoshell-sdk/build.mjs";

const here = (path) => fileURLToPath(new URL(path, import.meta.url));

mkdirSync(here("./dist"), { recursive: true });
copyFileSync(here("./widget/manifest.json"), here("./dist/manifest.json"));
await buildWidget({ entry: here("./widget/client.ts"), outfile: here("./dist/client.js") });
