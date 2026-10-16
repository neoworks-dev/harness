// Bundles the UI for the svelte-gpui host. svelte-gpui and extension-system are checked out next
// to this repo.
// `bun apps/gui/build.mjs --watch` rebuilds on every save (run the binary with --watch too).
import { fileURLToPath } from "node:url";
import { build } from "../../../svelte-gpui/build.mjs";

const here = (path) => fileURLToPath(new URL(path, import.meta.url));

await build({
  entry: here("./ui/main.ts"),
  outfile: here("./dist/gui.js"),
  watch: process.argv.includes("--watch"),
  // From source, like svelte-gpui. The tsconfigs and `bun test` read its built `lib/` (`bun run
  // build` there): its source does not pass this repo's strict compiler flags.
  alias: { "@neoworks/extension-system": here("../../../extension-system/src/index.ts") },
});
