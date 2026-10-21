// Language servers that installed plugins bring (#25), as `Language`s next to the fixed ones.
import { join } from "node:path";
import { suiteConfigDir } from "../../../harness/src/mcp-config.ts";
import { enabledLspServers } from "../../../harness/src/plugin-lsp.ts";
import type { Language } from "./servers.ts";

/** Files that mark a project root for a plugin's server, which doesn't say. */
const ROOT_MARKERS = [".git"];

/** The enabled plugins' servers; read each time, so a plugin installed now is used at once. */
export function pluginLanguages(
  pluginsDir: string = join(suiteConfigDir(), "plugins"),
): Language[] {
  return enabledLspServers(pluginsDir).map((server) => ({
    id: server.name,
    extensions: server.extensions,
    command: server.command,
    rootMarkers: ROOT_MARKERS,
  }));
}
