import { afterEach, expect, test } from "bun:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { ToolListChangedNotificationSchema } from "@modelcontextprotocol/sdk/types.js";
import { createToolServer, type ToolServer, type ToolSpec } from "../src/index.ts";

async function until(check: () => boolean, what: string): Promise<void> {
  const deadline = Date.now() + 5000;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(10);
  }
}

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const add: ToolSpec = {
  name: "add",
  description: "Adds two numbers",
  inputSchema: {
    type: "object",
    properties: { a: { type: "number" }, b: { type: "number" } },
    required: ["a", "b"],
  },
  outputSchema: { type: "object", properties: { sum: { type: "number" } } },
  handler(args) {
    const sum = Number(args.a) + Number(args.b);
    return Promise.resolve({
      content: [{ type: "text", text: String(sum) }],
      structuredContent: { sum },
    });
  },
};

const boom: ToolSpec = {
  name: "boom",
  description: "Always fails",
  inputSchema: { type: "object", properties: {} },
  handler: () => Promise.reject(new Error("kaboom")),
};

const echo: ToolSpec = {
  name: "echo",
  description: "Echoes text",
  inputSchema: { type: "object", properties: { text: { type: "string" } } },
  handler: (args) => Promise.resolve(String(args.text)),
};

async function mcpClient(server: ToolServer, onChanged?: () => void): Promise<Client> {
  const client = new Client({ name: "test", version: "0" });
  if (onChanged) client.setNotificationHandler(ToolListChangedNotificationSchema, onChanged);
  const transport = new StreamableHTTPClientTransport(new URL(server.url), {
    requestInit: { headers: server.headers },
  });
  await client.connect(transport);
  cleanups.push(() => client.close());
  return client;
}

async function serve(tools: ToolSpec[], instructions?: string): Promise<ToolServer> {
  const server = await createToolServer({ name: "demo", instructions, tools });
  cleanups.push(() => server.close());
  return server;
}

test("lists tools and calls them", async () => {
  const server = await serve([add, boom, echo], "Use add for sums.");
  expect(server.endpoint).toEqual({ url: server.url, headers: server.headers });
  const client = await mcpClient(server);
  expect(client.getInstructions()).toBe("Use add for sums.");

  const { tools } = await client.listTools();
  expect(tools.map((t) => t.name)).toEqual(["add", "boom", "echo"]);
  expect(tools[0]?.outputSchema).toBeDefined();

  const sum = await client.callTool({ name: "add", arguments: { a: 2, b: 3 } });
  expect(sum.structuredContent).toEqual({ sum: 5 });
  expect(sum.content).toEqual([{ type: "text", text: "5" }]);

  const text = await client.callTool({ name: "echo", arguments: { text: "hi" } });
  expect(text.content).toEqual([{ type: "text", text: "hi" }]);
});

test("skills are prompts without arguments", async () => {
  const layers = { name: "layers", description: "Working with layers", text: "Use layers." };
  const server = await createToolServer({ name: "demo", tools: [echo], skills: [layers] });
  cleanups.push(() => server.close());
  const client = await mcpClient(server);
  const { prompts } = await client.listPrompts();
  expect(prompts).toEqual([{ name: "layers", description: "Working with layers" }]);
  const got = await client.getPrompt({ name: "layers" });
  expect(got.messages).toEqual([{ role: "user", content: { type: "text", text: "Use layers." } }]);
  const unknown = await client.getPrompt({ name: "letter" }).catch((err: unknown) => err);
  expect(String(unknown)).toContain("no skill letter");
});

test("without skills the server offers no prompts", async () => {
  const client = await mcpClient(await serve([echo]));
  expect(client.getServerCapabilities()?.prompts).toBeUndefined();
});

test("a thrown error becomes an isError result", async () => {
  const client = await mcpClient(await serve([boom]));
  const result = await client.callTool({ name: "boom", arguments: {} });
  expect(result.isError).toBe(true);
  expect(result.content).toEqual([{ type: "text", text: "kaboom" }]);
});

test("a wrong or missing token gets 401", async () => {
  const server = await serve([echo]);
  const init = { method: "POST", body: "{}", headers: { "content-type": "application/json" } };
  expect((await fetch(server.url, init)).status).toBe(401);
  const wrong = { ...init, headers: { ...init.headers, authorization: "Bearer nope" } };
  expect((await fetch(server.url, wrong)).status).toBe(401);
  const right = { ...init, headers: { ...init.headers, ...server.headers } };
  expect((await fetch(server.url, right)).status).not.toBe(401);
});

test("setTools changes the list and sends list_changed", async () => {
  const server = await serve([echo]);
  let changes = 0;
  const client = await mcpClient(server, () => {
    changes += 1;
  });
  // The notification travels on the standalone stream, which opens just after initialize.
  await Bun.sleep(100);
  server.setTools([echo, add]);
  await until(() => changes === 1, "list_changed");
  const { tools } = await client.listTools();
  expect(tools.map((t) => t.name)).toEqual(["echo", "add"]);
});
