// The theme the daemon serves: SUITE_THEME or ~/.config/suite/theme.json if present, else the
// built-in default. An invalid file is reported and ignored.
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Theme } from "@suite/protocol/types";
import { validateDef } from "@suite/protocol/validate";
import defaultTheme from "../../protocol/themes/default.json";
import type { Logger } from "./log.ts";

export function loadTheme(env: Record<string, string | undefined>, log: Logger): Theme {
  const configHome = env.XDG_CONFIG_HOME ?? join(homedir(), ".config");
  const path = env.SUITE_THEME ?? join(configHome, "suite", "theme.json");
  if (!existsSync(path)) return defaultTheme;
  try {
    const theme: unknown = JSON.parse(readFileSync(path, "utf8"));
    const problem = validateDef("view.schema.json#/$defs/Theme", theme);
    if (problem === undefined) return theme as Theme;
    log.warn(`theme ${path} is invalid (${problem}); using the default`);
  } catch (err) {
    log.warn(`theme ${path} can't be read (${String(err)}); using the default`);
  }
  return defaultTheme;
}
