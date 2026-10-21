import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { startMcpApp } from "./mcp-app.ts";

const FIXTURE = join(import.meta.dir, "stdio-mcp-server.ts");

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function text(result: CallToolResult): string {
  return result.content
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

/** A fresh configuration directory with a config.json and an (empty) plugins directory. */
function configDir(config: Record<string, unknown>): { configFile: string; plugins: string } {
  const dir = mkdtempSync(join(tmpdir(), "suite-mcp-"));
  const configFile = join(dir, "config.json");
  writeFileSync(configFile, JSON.stringify(config));
  const plugins = join(dir, "plugins");
  mkdirSync(plugins);
  return { configFile, plugins };
}

async function daemonWith(
  mcp: { configFile: string; plugins: string },
  backoffMs = 20,
): Promise<TestDaemon> {
  const daemon = await startTestDaemon({
    plugins: mcp.plugins,
    mcp: { configFile: mcp.configFile, servers: { backoffMs, stableMs: 60_000 } },
  });
  cleanups.push(() => daemon.stop());
  await daemon.mcp.reload();
  return daemon;
}

function fixtureServer(
  name = "fixture",
  env: Record<string, string> = {},
): Record<string, unknown> {
  return { command: process.execPath, args: ["run", FIXTURE, name], env };
}

async function until(check: () => boolean | Promise<boolean>, what: string): Promise<void> {
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await Bun.sleep(25);
  }
  throw new Error(`timed out waiting for ${what}`);
}

describe("stdio MCP servers", () => {
  test("a configured server's tools come through the gateway", async () => {
    const daemon = await daemonWith(
      configDir({ mcpServers: { fixture: fixtureServer("fixture", { GREETING: "hello" }) } }),
    );
    const connector = daemon.connectors.connector("mcp-fixture");
    expect(connector?.kind).toBe("service");
    expect(connector?.tools.map((t) => t.name).sort()).toEqual(["echo", "env", "pid"]);
    expect(daemon.connectors.snapshot().map((c) => c.app)).toContain("mcp-fixture");

    const found = text(await daemon.gateway.call("tools_search", { query: "repeats text back" }));
    expect(found).toContain("mcp-fixture.echo");
    const echoed = await daemon.gateway.call("tools_call", {
      name: "mcp-fixture.echo",
      args: { text: "hi" },
    });
    expect(text(echoed)).toBe("fixture: hi");
    // Its own variables reach it, the daemon's secrets do not.
    process.env.SUITE_TEST_SECRET = "s3cret";
    cleanups.push(() => Promise.resolve(void delete process.env.SUITE_TEST_SECRET));
    const own = await daemon.gateway.call("tools_call", {
      name: "mcp-fixture.env",
      args: { name: "GREETING" },
    });
    expect(text(own)).toBe("hello");
    const secret = await daemon.gateway.call("tools_call", {
      name: "mcp-fixture.env",
      args: { name: "SUITE_TEST_SECRET" },
    });
    expect(text(secret)).toBe("(unset)");
    // The gateway checks arguments against the tool's schema.
    const bad = await daemon.gateway.call("tools_call", { name: "mcp-fixture.echo", args: {} });
    expect(bad.isError).toBe(true);
  });

  test("a server that dies is started again", async () => {
    const daemon = await daemonWith(configDir({ mcpServers: { fixture: fixtureServer() } }));
    const pid = async (): Promise<number> =>
      Number(text(await daemon.gateway.call("tools_call", { name: "mcp-fixture.pid", args: {} })));
    const first = await pid();
    process.kill(first, "SIGKILL");
    await until(() => !daemon.connectors.isOpen("mcp-fixture"), "the connector to close");
    // Its tools stay known while it is down.
    expect(daemon.connectors.connector("mcp-fixture")?.tools).toHaveLength(3);
    const down = await daemon.gateway.call("tools_call", { name: "mcp-fixture.pid", args: {} });
    expect(down.isError).toBe(true);
    await until(
      () => daemon.connectors.upstream("mcp-fixture")?.upstream !== undefined,
      "a restart",
    );
    const second = await pid();
    expect(second).not.toBe(first);
    expect(daemon.mcp.servers.status()).toMatchObject([{ app: "mcp-fixture", state: "up" }]);
  });

  test("a program that does not exist is retried, then left alone", async () => {
    const config = configDir({ mcpServers: { missing: { command: "/no/such/program" } } });
    const daemon = await startTestDaemon({
      plugins: config.plugins,
      mcp: {
        configFile: config.configFile,
        servers: { backoffMs: 5, maxBackoffMs: 5, maxFailures: 2 },
      },
    });
    cleanups.push(() => daemon.stop());
    await daemon.mcp.reload();
    await until(
      () => daemon.mcp.servers.status()[0]?.state === "failed",
      "the server to be given up on",
    );
    expect(daemon.mcp.servers.status()[0]?.error).toBeTruthy();
    expect(daemon.connectors.isOpen("mcp-missing")).toBe(false);
  });

  test("a plugin's .mcp.json runs with ${CLAUDE_PLUGIN_ROOT} expanded", async () => {
    const config = configDir({});
    const root = join(config.plugins, "cache", "market", "tools", "1.0.0");
    mkdirSync(root, { recursive: true });
    writeFileSync(
      join(root, ".mcp.json"),
      JSON.stringify({
        mcpServers: {
          tools: {
            command: process.execPath,
            args: ["run", "${CLAUDE_PLUGIN_ROOT}/server.ts", "from-plugin"],
          },
        },
      }),
    );
    writeFileSync(join(root, "server.ts"), `await import(${JSON.stringify(FIXTURE)});\n`);
    writeFileSync(
      join(config.plugins, "installed.json"),
      JSON.stringify({
        version: 1,
        plugins: { "tools@market": { root, version: "1.0.0", enabled: true, installedAt: "" } },
      }),
    );
    const daemon = await daemonWith(config);
    const echoed = await daemon.gateway.call("tools_call", {
      name: "mcp-tools.echo",
      args: { text: "x" },
    });
    expect(text(echoed)).toBe("from-plugin: x");

    // Disabling the plugin stops its server on the next reload.
    writeFileSync(
      join(config.plugins, "installed.json"),
      JSON.stringify({
        version: 1,
        plugins: { "tools@market": { root, version: "1.0.0", enabled: false, installedAt: "" } },
      }),
    );
    await daemon.mcp.reload();
    expect(daemon.connectors.isOpen("mcp-tools")).toBe(false);
    expect(daemon.mcp.servers.status()).toEqual([]);
  });

  test("a project's .mcp.json counts only when config.json allows it", async () => {
    const project = mkdtempSync(join(tmpdir(), "suite-project-"));
    writeFileSync(
      join(project, ".mcp.json"),
      JSON.stringify({ mcpServers: { proj: fixtureServer("proj") } }),
    );

    const off = configDir({});
    const daemon = await startTestDaemon({
      plugins: off.plugins,
      mcp: { configFile: off.configFile, servers: { backoffMs: 20 } },
    });
    cleanups.push(() => daemon.stop());
    daemon.sessions.create({ cwd: project });
    await daemon.mcp.reload();
    expect(daemon.mcp.servers.status()).toEqual([]);

    const on = configDir({ projectMcp: true });
    const allowed = await startTestDaemon({
      plugins: on.plugins,
      mcp: { configFile: on.configFile, servers: { backoffMs: 20 } },
    });
    cleanups.push(() => allowed.stop());
    allowed.sessions.create({ cwd: project });
    await allowed.mcp.reload();
    expect(allowed.connectors.isOpen("mcp-proj")).toBe(true);
  });

  test("a remote server in the same shape is reached over HTTP", async () => {
    const app = startMcpApp({
      tools: [
        {
          name: "ping",
          description: "Answers pong",
          inputSchema: {},
          handler: () => ({ content: [{ type: "text", text: "pong" }] }),
        },
      ],
    });
    cleanups.push(() => app.close());
    const daemon = await daemonWith(
      configDir({
        mcpServers: {
          remote: { type: "http", url: app.endpoint.url, headers: app.endpoint.headers ?? {} },
        },
      }),
    );
    expect(
      text(await daemon.gateway.call("tools_call", { name: "mcp-remote.ping", args: {} })),
    ).toBe("pong");
  });

  test("the servers stop with the daemon", async () => {
    const config = configDir({ mcpServers: { fixture: fixtureServer() } });
    const daemon = await startTestDaemon({
      plugins: config.plugins,
      mcp: { configFile: config.configFile, servers: { backoffMs: 20 } },
    });
    await daemon.mcp.reload();
    const pid = Number(
      text(await daemon.gateway.call("tools_call", { name: "mcp-fixture.pid", args: {} })),
    );
    expect(() => process.kill(pid, 0)).not.toThrow();
    await daemon.stop();
    await until(() => {
      try {
        process.kill(pid, 0);
        return false;
      } catch {
        return true;
      }
    }, "the process to end");
  });
});
