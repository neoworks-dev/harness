// The library must be imported before anything that pulls in Svelte's client runtime: it
// installs the DOM globals that runtime reads at module scope.
import { start } from "@neoworks/svelte-gpui";
import App from "./App.svelte";
import "./commands.ts";

start(
  App,
  {},
  { window: { title: "suite", width: 1100, height: 780, minWidth: 520, minHeight: 360 } },
);
