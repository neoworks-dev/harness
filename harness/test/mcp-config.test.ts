import { expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { appIdFor, expandEnv, loadMcpServers, parseServer } from "../src/mcp-config.ts";
import { tempDir } from "./helpers.ts";

test("app ids fit the pattern and can't clash with an app's", () => {
  expect(appIdFor("headroom")).toBe("mcp-headroom");
  expect(appIdFor("My Server_2")).toBe("mcp-my-server-2");
  expect(appIdFor("plugin", "server")).toBe("mcp-plugin-server");
  expect(appIdFor("a".repeat(60))).toMatch(/^[a-z][a-z0-9-]{0,31}$/);
  expect(appIdFor("***")).toBeUndefined();
});

test("${VAR} and ${VAR:-default} come from the environment", () => {
  const env = { A: "1", EMPTY: "" };
  expect(expandEnv("${A}-${B}-${B:-x}-${EMPTY:-y}", env)).toBe("1--x-y");
});

test("a server is stdio or http, in Claude Code's shape", () => {
  const same = (text: string): string => text;
  expect(
    parseServer({ command: "headroom", args: ["mcp", "serve"], env: { K: "v" } }, same),
  ).toEqual({
    type: "stdio",
    command: "headroom",
    args: ["mcp", "serve"],
    env: { K: "v" },
  });
  expect(parseServer({ type: "http", url: "https://x/mcp", headers: { A: "b" } }, same)).toEqual({
    type: "http",
    url: "https://x/mcp",
    headers: { A: "b" },
  });
  expect(parseServer({ url: "https://x/mcp" }, same)).toMatchObject({ type: "http" });
  expect(parseServer({ type: "sse", url: "https://x" }, same)).toContain("isn't supported");
  expect(parseServer({ args: [] }, same)).toContain("command");
  expect(parseServer({ command: "x", args: [1] }, same)).toContain("command");
});

test("servers come from config.json, projects (if allowed) and enabled plugins, first one wins", async () => {
  const dir = await tempDir();
  const plugins = join(dir, "plugins");
  const configFile = join(dir, "config.json");
  const project = join(dir, "project");
  await mkdir(project, { recursive: true });
  await writeFile(
    join(project, ".mcp.json"),
    JSON.stringify({
      mcpServers: { local: { command: "local-server" }, shared: { command: "project" } },
    }),
  );
  const root = join(plugins, "cache", "m", "tools", "1");
  await mkdir(root, { recursive: true });
  // Both file shapes: wrapped in mcpServers, and the table itself.
  await writeFile(
    join(root, ".mcp.json"),
    JSON.stringify({
      tools: { command: "${CLAUDE_PLUGIN_ROOT}/run", args: ["--home", "${HOME_FOR_TEST}"] },
      other: { type: "http", url: "https://x/mcp", headers: { Authorization: "Bearer ${TOKEN}" } },
      broken: { args: [] },
    }),
  );
  const off = join(plugins, "cache", "m", "off", "1");
  await mkdir(off, { recursive: true });
  await writeFile(join(off, ".mcp.json"), JSON.stringify({ off: { command: "off" } }));
  await writeFile(
    join(plugins, "installed.json"),
    JSON.stringify({
      version: 1,
      plugins: {
        "tools@m": { root, version: "1", enabled: true, installedAt: "" },
        "off@m": { root: off, version: "1", enabled: false, installedAt: "" },
      },
    }),
  );
  const env = { HOME_FOR_TEST: "/home/me", TOKEN: "t0k" };

  await writeFile(configFile, JSON.stringify({ mcpServers: { shared: { command: "config" } } }));
  let loaded = loadMcpServers({ configFile, plugins, projects: [project], env });
  expect(loaded.servers.map((s) => s.app)).toEqual(["mcp-shared", "mcp-tools", "mcp-tools-other"]);
  expect(loaded.servers[0]?.config).toMatchObject({ command: "config" });
  expect(loaded.servers[1]?.config).toEqual({
    type: "stdio",
    command: `${root}/run`,
    args: ["--home", "/home/me"],
    env: {},
  });
  expect(loaded.servers[2]?.config).toMatchObject({ headers: { Authorization: "Bearer t0k" } });
  expect(loaded.problems.join("\n")).toContain("broken");

  await writeFile(
    configFile,
    JSON.stringify({ projectMcp: true, mcpServers: { shared: { command: "config" } } }),
  );
  loaded = loadMcpServers({ configFile, plugins, projects: [project], env });
  expect(loaded.servers.map((s) => s.app)).toEqual([
    "mcp-shared",
    "mcp-local",
    "mcp-tools",
    "mcp-tools-other",
  ]);
  // The project's "shared" lost to config.json's.
  expect(loaded.problems.join("\n")).toContain("mcp-shared already");
});
