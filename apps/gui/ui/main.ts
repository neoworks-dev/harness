// The library must be imported before anything that pulls in Svelte's client runtime: it
// installs the DOM globals that runtime reads at module scope.
import { start } from "@neoworks/svelte-gpui";
import App from "./App.svelte";
import "./commands.ts";
import { boot } from "./kernel/boot.ts";
import { root } from "./kernel/context.ts";
import { services } from "./kernel/services.ts";
import { plugins } from "./plugins/index.ts";

// The window opens once the plugins are mounted, since they fill it.
boot(root, services, plugins)
  .catch((error: unknown) => console.error("boot failed", error))
  .finally(() =>
    start(
      App,
      {},
      { window: { title: "suite", width: 1100, height: 780, minWidth: 520, minHeight: 360 } },
    ),
  );
