import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pluginLanguages } from "../src/plugins.ts";
import { languageOf, LANGUAGES, ServerPool } from "../src/servers.ts";

const dirs: string[] = [];
afterAll(() => {
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
});

function pluginsDir(): string {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "lsp-plugins-")));
  dirs.push(dir);
  return dir;
}

function install(dir: string, enabled: boolean): void {
  writeFileSync(
    join(dir, "installed.json"),
    JSON.stringify({
      version: 1,
      plugins: {
        "zig-lsp@market": {
          root: join(dir, "zig"),
          version: "1",
          enabled,
          installedAt: "",
          entry: {
            lspServers: {
              zls: {
                command: "no-such-zls",
                args: ["--stdio"],
                extensionToLanguage: { ".zig": "zig", ".ZON": "zon" },
              },
            },
          },
        },
      },
    }),
  );
}

test("an enabled plugin's language servers are languages of their own", () => {
  const dir = pluginsDir();
  expect(pluginLanguages(dir)).toEqual([]);
  install(dir, true);
  expect(pluginLanguages(dir)).toEqual([
    {
      id: "zls",
      extensions: { ".zig": "zig", ".zon": "zon" },
      command: ["no-such-zls", "--stdio"],
      rootMarkers: [".git"],
    },
  ]);
  install(dir, false);
  expect(pluginLanguages(dir)).toEqual([]);
});

test("the fixed languages come first, and the pool asks for the plugins' on every file", () => {
  const dir = pluginsDir();
  const pool = new ServerPool(
    () => {},
    () => pluginLanguages(dir),
  );
  expect(() => pool.get("/w/a.zig")).toThrow("No language server for .zig");
  install(dir, true);
  // Installed while the pool runs: used at once, here by failing on the program it names.
  expect(() => pool.get("/w/a.zig")).toThrow("no-such-zls isn't installed");
  expect(languageOf("/w/a.go", [...LANGUAGES, ...pluginLanguages(dir)])?.language.id).toBe("go");
});
