import { expect, test } from "bun:test";
import { writeFile } from "node:fs/promises";
import { enabledPlugins, expandPluginRoot } from "../src/plugins.ts";
import { tempDir } from "./helpers.ts";

test("enabled plugins come from installed.json, broken and disabled ones left out", async () => {
  const dir = await tempDir();
  expect(enabledPlugins(dir)).toEqual([]);
  await writeFile(
    `${dir}/installed.json`,
    JSON.stringify({
      version: 1,
      plugins: {
        "svelte@svelte": { root: "/p/svelte", version: "1.0.8", installedAt: "t", enabled: true },
        "off@x": { root: "/p/off", version: "1", installedAt: "t", enabled: false },
        "relative@x": { root: "p/rel", version: "1", installedAt: "t" },
        noat: { root: "/p/noat", version: "1", installedAt: "t" },
        "lsp@official": {
          root: "/p/lsp",
          version: "1",
          installedAt: "t",
          entry: { lspServers: {} },
        },
      },
    }),
  );
  expect(enabledPlugins(dir)).toEqual([
    {
      id: "lsp@official",
      name: "lsp",
      marketplace: "official",
      root: "/p/lsp",
      version: "1",
      entry: { lspServers: {} },
    },
    {
      id: "svelte@svelte",
      name: "svelte",
      marketplace: "svelte",
      root: "/p/svelte",
      version: "1.0.8",
    },
  ]);
  const [plugin] = enabledPlugins(dir);
  if (!plugin) throw new Error("no plugin");
  expect(expandPluginRoot("${CLAUDE_PLUGIN_ROOT}/hooks/run.sh", plugin)).toBe(
    "/p/lsp/hooks/run.sh",
  );
});
