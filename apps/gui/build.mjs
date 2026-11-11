// Bundles the UI. svelte-gpui and extension-system are checked out next to this repo.
//   bun apps/gui/build.mjs           the native window's bundle (dist/gui.js), for the svelte-gpui host
//   bun apps/gui/build.mjs --web     the same UI for a browser tab (dist/web/: index.html, js, css),
//                                    which `neo-gui --web` serves (neo web opens it)
// `--watch` rebuilds on every save (for the native window, run the binary with --watch too).
import { fileURLToPath } from "node:url";

const here = (path) => fileURLToPath(new URL(path, import.meta.url));

// From source, like svelte-gpui. The tsconfigs and `bun test` read its built `lib/` (`bun run
// build` there): its source does not pass this repo's strict compiler flags.
const alias = { "@neoworks/extension-system": here("../../../extension-system/src/index.ts") };
const watch = process.argv.includes("--watch");

if (process.argv.includes("--web")) {
  // TODO: buildWeb is in svelte-gpui's web-target worktree until that branch is merged; then this
  // imports "../../../svelte-gpui/build.mjs" like the native build below.
  const { buildWeb } = await import("../../../svelte-gpui/.worktrees/web-target/build.mjs");
  await buildWeb({
    entry: here("./ui/main.web.ts"),
    outdir: here("./dist/web"),
    alias,
    watch,
    title: "neo",
  });
} else {
  const { build } = await import("../../../svelte-gpui/build.mjs");
  await build({ entry: here("./ui/main.ts"), outfile: here("./dist/gui.js"), watch, alias });
}
