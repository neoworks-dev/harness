import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { AppPresence, SessionEvent } from "@suite/protocol/types";
import { Gateway } from "../src/gateway/gateway.ts";
import { ConnectorRegistry } from "../src/gateway/registry.ts";
import { connectMcp, type Upstream } from "../src/gateway/upstream.ts";
import { silentLogger } from "../src/log.ts";
import { SessionManager } from "../src/sessions.ts";
import { SessionStore, type StoredTool } from "../src/store.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { fauxToolCall } from "@mariozechner/pi-ai";
import { createPiAgent } from "@suite/harness/pi";
import { fakeModel, reply, TEST_CONFIG } from "../../harness/test/fake-model.ts";
import { FakeAgent } from "./fake-agent.ts";
import { type AppTool, type McpApp, startMcpApp } from "./mcp-app.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function tool(name: string, description: string, props: Record<string, unknown> = {}): StoredTool {
  return {
    name,
    description,
    inputSchema: { type: "object", properties: props, required: Object.keys(props) },
  };
}

type FakeUpstream = Upstream & { setTools(tools: StoredTool[]): void; calls: string[] };

interface Setup {
  store: SessionStore;
  registry: ConnectorRegistry;
  gateway: Gateway;
  upstreams: Map<string, FakeUpstream>;
}

/** An in-memory upstream whose tools and results the test controls. */
function fakeUpstream(initial: StoredTool[]): FakeUpstream {
  let tools = initial;
  const listeners: (() => void)[] = [];
  const calls: string[] = [];
  return {
    calls,
    listTools: () => Promise.resolve(tools),
    instructions: () => "Use crop before export.",
    call: (name, args) => {
      calls.push(`${name} ${JSON.stringify(args)}`);
      return Promise.resolve({
        content: [{ type: "text", text: `did ${name}` }],
        structuredContent: { ok: true },
      });
    },
    onToolsChanged: (l) => void listeners.push(l),
    close: () => Promise.resolve(),
    setTools(next) {
      tools = next;
      for (const l of listeners) l();
    },
  };
}

function setup(store = new SessionStore(":memory:")): Setup {
  const upstreams = new Map<string, FakeUpstream>();
  const registry = new ConnectorRegistry(
    store,
    (endpoint) => {
      const upstream = upstreams.get(endpoint.url);
      if (!upstream) return Promise.reject(new Error("connection refused"));
      return Promise.resolve(upstream);
    },
    silentLogger,
  );
  const gateway = new Gateway(registry, { log: silentLogger, launchTimeoutMs: 1000 });
  return { store, registry, gateway, upstreams };
}

const photoTools = [
  tool("crop", "Crop an image to a rectangle", {
    asset: { type: "string" },
    w: { type: "number" },
  }),
  tool("export", "Export the image as PNG or JPEG"),
];

function photoPresence(open = true): AppPresence {
  return {
    app: "photo-editor",
    kind: "app",
    title: "Photo editor",
    open,
    mcp: { url: "mem://photo" },
    launch: { command: ["sh", "-c", "true"] },
  };
}

function resultText(r: CallToolResult): string {
  const parts: string[] = [];
  for (const c of r.content) if (c.type === "text") parts.push(c.text);
  return parts.join("\n");
}

describe("connector registry", () => {
  test("presence lists the app's tools; schema changes bump v; closing keeps the schemas", async () => {
    const { registry, upstreams } = setup();
    const upstream = fakeUpstream(photoTools);
    upstreams.set("mem://photo", upstream);
    const owner = {};
    await registry.presence(owner, photoPresence());
    expect(registry.snapshot()).toEqual([
      { app: "photo-editor", kind: "app", v: 1, tools: ["crop", "export"] },
    ]);

    upstream.setTools([...photoTools, tool("resize", "Resize")]);
    await Bun.sleep(1);
    expect(registry.snapshot()[0]?.v).toBe(2);

    await registry.presence(owner, photoPresence(false));
    expect(registry.snapshot()).toEqual([]);
    expect(registry.list()[0]).toMatchObject({ app: "photo-editor", connected: false, v: 2 });
    expect(registry.list()[0]?.tools.map((t) => t.name)).toContain("photo-editor.resize");
  });

  test("re-registering with the same tools keeps the version; state survives a restart", async () => {
    const store = new SessionStore(":memory:");
    const first = setup(store);
    first.upstreams.set("mem://photo", fakeUpstream(photoTools));
    const owner = {};
    await first.registry.presence(owner, photoPresence());
    await first.registry.disconnect(owner);

    const second = setup(store);
    second.upstreams.set("mem://photo", fakeUpstream(photoTools));
    expect(second.registry.list()[0]).toMatchObject({ connected: false, v: 1 });
    await second.registry.presence({}, photoPresence());
    expect(second.registry.snapshot()[0]?.v).toBe(1);
  });

  test("only the registering client can unregister; its disconnect closes its apps", async () => {
    const { registry, upstreams } = setup();
    upstreams.set("mem://photo", fakeUpstream(photoTools));
    const a = {};
    const b = {};
    await registry.presence(a, photoPresence());
    await registry.presence(b, photoPresence(false));
    await registry.disconnect(b);
    expect(registry.isOpen("photo-editor")).toBe(true);
    await registry.disconnect(a);
    expect(registry.isOpen("photo-editor")).toBe(false);
  });

  test("an unreachable MCP server leaves the app open with an error", async () => {
    const { registry } = setup();
    await registry.presence({}, { ...photoPresence(), mcp: { url: "mem://nowhere" } });
    expect(registry.list()[0]).toMatchObject({
      connected: true,
      error: expect.stringContaining("connection refused"),
    });
  });
});

describe("core tools", () => {
  async function opened(): Promise<Setup & { upstream: FakeUpstream; owner: object }> {
    const s = setup();
    const upstream = fakeUpstream(photoTools);
    s.upstreams.set("mem://photo", upstream);
    const owner = {};
    await s.registry.presence(owner, photoPresence());
    return { ...s, upstream, owner };
  }

  test("tools_search returns full schemas and the connector's instructions", async () => {
    const { gateway } = await opened();
    const result = await gateway.call("tools_search", { query: "crop image" });
    const out = resultText(result);
    expect(out).toContain("# photo-editor (app, v1, open)");
    expect(out).toContain("Use crop before export.");
    expect(out).toContain("## photo-editor.crop");
    expect(out).toContain('"asset":{"type":"string"}');
    expect(result.structuredContent).toMatchObject({
      tools: [{ name: "photo-editor.crop", open: true }, { name: "photo-editor.export" }],
    });
  });

  test("tools_search: exact names, app listing, no matches, unknown app", async () => {
    const { gateway } = await opened();
    const exact = await gateway.call("tools_search", { query: "photo-editor.export" });
    expect(exact.structuredContent).toMatchObject({ tools: [{ name: "photo-editor.export" }] });
    const listing = await gateway.call("tools_search", { query: "", app: "photo-editor" });
    expect((listing.structuredContent as { tools: unknown[] }).tools).toHaveLength(2);
    expect(resultText(await gateway.call("tools_search", { query: "weather" }))).toContain(
      "No tools match",
    );
    expect((await gateway.call("tools_search", { query: "x", app: "nope" })).isError).toBe(true);
  });

  test("tools_call forwards valid calls and passes the result through", async () => {
    const { gateway, upstream } = await opened();
    const result = await gateway.call("tools_call", {
      name: "photo-editor.crop",
      args: { asset: "asset://img-1", w: 10 },
    });
    expect(result).toEqual({
      content: [{ type: "text", text: "did crop" }],
      structuredContent: { ok: true },
    });
    expect(upstream.calls).toEqual(['crop {"asset":"asset://img-1","w":10}']);
  });

  test("tools_call rejects bad arguments with the schema, unknown tools with a hint", async () => {
    const { gateway, upstream } = await opened();
    const bad = await gateway.call("tools_call", {
      name: "photo-editor.crop",
      args: { asset: 3 },
    });
    expect(bad.isError).toBe(true);
    expect(resultText(bad)).toContain("Invalid arguments for photo-editor.crop");
    expect(resultText(bad)).toContain("Input schema:");
    expect(upstream.calls).toEqual([]);

    const unknown = await gateway.call("tools_call", { name: "photo-editor.sharpen" });
    expect(resultText(unknown)).toContain("tools_search");
    expect((await gateway.call("tools_call", { name: "nonsense" })).isError).toBe(true);
  });

  test("closed apps answer with the design's error", async () => {
    const { gateway, registry, owner } = await opened();
    await registry.disconnect(owner);
    const result = await gateway.call("tools_call", { name: "photo-editor.export" });
    expect(result).toEqual({
      isError: true,
      content: [
        {
          type: "text",
          text: "Photo editor isn't open. Ask the user to open it, or call apps_launch('photo-editor').",
        },
      ],
    });
  });

  test("apps_launch starts the app and waits until it registers", async () => {
    const { gateway, registry, owner } = await opened();
    await registry.disconnect(owner);
    const launched = gateway.call("apps_launch", { app: "photo-editor" });
    await Bun.sleep(20);
    await registry.presence({}, photoPresence()); // what the launched app would do
    expect(resultText(await launched)).toBe("Photo editor is open. Tools: crop, export.");
    expect(resultText(await gateway.call("apps_launch", { app: "photo-editor" }))).toContain(
      "already open",
    );
  });

  test("apps_launch times out when the app never registers, and needs a launch command", async () => {
    const { gateway, registry, owner } = await opened();
    await registry.disconnect(owner);
    const result = await gateway.call("apps_launch", { app: "photo-editor" });
    expect(result.isError).toBe(true);
    expect(resultText(result)).toContain("didn't connect");

    await registry.presence({}, { app: "lsp", kind: "service", open: true });
    await registry.disconnect({});
    expect(resultText(await gateway.call("apps_launch", { app: "nope" }))).toContain(
      "No connector",
    );
  });

  test("apps_focus is not available before Phase 3", async () => {
    const { gateway } = await opened();
    expect((await gateway.call("apps_focus", { app: "photo-editor" })).isError).toBe(true);
  });
});

describe("session announcements", () => {
  test("each turn starts with what changed since the session was last told", async () => {
    const { store, registry, upstreams } = setup();
    const photo = fakeUpstream(photoTools);
    upstreams.set("mem://photo", photo);
    const agent = new FakeAgent();
    const sessions = new SessionManager(store, agent, {
      defaultCwd: "/",
      log: silentLogger,
      connectors: () => registry.snapshot(),
    });
    const context = (n: number): SessionEvent[] => agent.prompts[n]?.pendingEvents ?? [];
    const turn = async (n: number): Promise<void> => {
      sessions.input(id, "cli", `turn ${n}`);
      await agent.started(n + 1);
      agent.release();
      await sessions.idle(id);
    };
    const { id } = sessions.create({});
    const owner = {};
    await registry.presence(owner, photoPresence());
    await turn(0);
    expect(context(0)).toEqual([
      { type: "app_connected", app: "photo-editor", kind: "app", v: 1, tools: ["crop", "export"] },
    ]);

    await turn(1); // nothing changed
    expect(context(1)).toEqual([]);

    await registry.disconnect(owner); // closed and reopened between prompts: nothing to say
    await registry.presence(owner, photoPresence());
    await turn(2);
    expect(context(2)).toEqual([]);

    photo.setTools([tool("crop", "Crop it differently")]);
    await Bun.sleep(1);
    await turn(3);
    expect(context(3)).toEqual([
      { type: "app_connected", app: "photo-editor", kind: "app", v: 2, tools: ["crop"] },
    ]);

    await registry.disconnect(owner);
    await turn(4);
    expect(context(4)).toEqual([{ type: "app_disconnected", app: "photo-editor" }]);
    // The announcements are part of the log, just before the user message they precede.
    expect(
      store
        .read(id)
        .map((r) => r.event.type)
        .slice(-4),
    ).toEqual(["app_disconnected", "user_message", "agent_message_chunk", "turn_ended"]);
  });
});

describe("over real MCP", () => {
  async function daemonWithApp(tools: AppTool[]): Promise<{ daemon: TestDaemon; app: McpApp }> {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const app = startMcpApp({ tools, instructions: "Greets people." });
    cleanups.push(() => app.close());
    await daemon.connectors.presence(
      {},
      {
        app: "greeter",
        kind: "app",
        open: true,
        mcp: app.endpoint,
      },
    );
    return { daemon, app };
  }

  const greet: AppTool = {
    name: "greet",
    description: "Say hello to someone",
    inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
    handler: (args) => ({
      content: [{ type: "text", text: `Hello, ${String(args.name)}!` }],
      structuredContent: { greeted: args.name },
    }),
  };

  test("the gateway lists, calls and follows list_changed on an app's MCP server", async () => {
    const { daemon, app } = await daemonWithApp([greet]);
    expect(daemon.connectors.snapshot()).toEqual([
      { app: "greeter", kind: "app", v: 1, tools: ["greet"] },
    ]);
    const result = await daemon.gateway.call("tools_call", {
      name: "greeter.greet",
      args: { name: "Ada" },
    });
    expect(result.structuredContent).toEqual({ greeted: "Ada" });
    expect(app.calls).toEqual([{ name: "greet", args: { name: "Ada" } }]);

    await app.setTools([greet, { ...greet, name: "wave", description: "Wave" }]);
    for (let i = 0; i < 100 && daemon.connectors.snapshot()[0]?.v !== 2; i++) await Bun.sleep(10);
    expect(daemon.connectors.snapshot()[0]).toMatchObject({ v: 2, tools: ["greet", "wave"] });
  });

  test("the daemon's /mcp endpoint serves the fixed core tools with token auth", async () => {
    const { daemon } = await daemonWithApp([greet]);
    const client = new Client({ name: "test", version: "1" });
    await client.connect(
      new StreamableHTTPClientTransport(new URL(`${daemon.info.url}/mcp`), {
        requestInit: { headers: { authorization: `Bearer ${daemon.info.token}` } },
      }),
    );
    cleanups.push(() => client.close());
    const { tools } = await client.listTools();
    expect(tools.map((t) => t.name)).toEqual([
      "tools_search",
      "tools_call",
      "apps_launch",
      "apps_focus",
    ]);
    const found = await client.callTool({ name: "tools_search", arguments: { query: "hello" } });
    expect(JSON.stringify(found.content)).toContain("greeter.greet");

    const denied = await fetch(`${daemon.info.url}/mcp`, { method: "POST", body: "{}" });
    expect(denied.status).toBe(401);
  });

  test("the pi harness reaches an app's tool through the gateway's MCP endpoint", async () => {
    const model = fakeModel([
      reply(
        [
          fauxToolCall(
            "tools_call",
            { name: "greeter.greet", args: { name: "Ada" } },
            { id: "c1" },
          ),
        ],
        "toolUse",
      ),
      reply("Greeted."),
    ]);
    const agent = createPiAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
    const daemon = await startTestDaemon({ agent });
    cleanups.push(() => daemon.stop());
    const app = startMcpApp({ tools: [greet] });
    cleanups.push(() => app.close());
    await daemon.connectors.presence(
      {},
      { app: "greeter", kind: "app", open: true, mcp: app.endpoint },
    );

    const { id } = daemon.sessions.create({});
    daemon.sessions.input(id, "cli", "greet Ada");
    await daemon.sessions.idle(id);

    const events = daemon.store.read(id).map((r) => r.event);
    expect(events[0]).toEqual({
      type: "app_connected",
      app: "greeter",
      kind: "app",
      v: 1,
      tools: ["greet"],
    });
    expect(events).toContainEqual({
      type: "tool_call_update",
      toolCallId: "c1",
      status: "completed",
      content: [{ type: "text", text: "Hello, Ada!" }],
      rawOutput: { greeted: "Ada" },
    });
    expect(events.at(-1)).toEqual({ type: "turn_ended", stopReason: "end_turn" });
    // The model saw the connect event in the user message, then the tool result.
    expect(JSON.stringify(model.requests[0])).toContain('<app connected=\\"greeter\\"');
    expect(JSON.stringify(model.requests[1])).toContain("Hello, Ada!");
    expect(app.calls).toEqual([{ name: "greet", args: { name: "Ada" } }]);
  });

  test("unreachable app servers are reported, not fatal", async () => {
    const dir = mkdtempSync(join(tmpdir(), "suite-gw-"));
    const store = new SessionStore(join(dir, "s.db"));
    const registry = new ConnectorRegistry(store, connectMcp, silentLogger);
    await registry.presence(
      {},
      {
        app: "ghost",
        kind: "app",
        open: true,
        mcp: { url: "http://127.0.0.1:9/mcp" },
      },
    );
    expect(registry.list()[0]?.error).toContain("can't reach its MCP server");
    await registry.close();
  });
});
