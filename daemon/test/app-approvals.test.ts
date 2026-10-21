// Approvals for the agents' calls to apps' tools (issue #34), from MCP's annotations: a read never
// asks, an edit the app can undo asks in ask mode only, any other tool asks in every mode but auto.
import { afterEach, describe, expect, test } from "bun:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { SessionEvent } from "@neoworks-dev/neo-protocol/types";
import type { Approvals } from "../src/settings.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { FakeAgent } from "./fake-agent.ts";
import { type AppTool, type McpApp, startMcpApp } from "./mcp-app.ts";

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const photoSchema = {
  type: "object",
  properties: { photo: { type: "string" } },
  required: ["photo"],
};

interface Photos {
  daemon: TestDaemon;
  app: McpApp;
  session: string;
}

/** A stand-in for Latent: a read, an edit it can undo, and an export it can't take back. */
async function setup(mode: Approvals): Promise<Photos> {
  const daemon = await startTestDaemon({ agent: new FakeAgent("neo") });
  cleanups.push(() => daemon.stop());
  const tools: AppTool[] = [
    {
      name: "list",
      description: "List photos",
      inputSchema: { type: "object", properties: {} },
      annotations: { readOnlyHint: true },
      handler: () => ({ content: [{ type: "text", text: "1, 2" }] }),
    },
    {
      name: "adjust",
      description: "Adjust a photo",
      inputSchema: photoSchema,
      annotations: { destructiveHint: false },
      handler: (args) => ({ content: [{ type: "text", text: `adjusted ${String(args.photo)}` }] }),
    },
    {
      name: "export",
      description: "Write the photo out",
      inputSchema: photoSchema,
      handler: () => ({ content: [{ type: "text", text: "exported" }] }),
    },
  ];
  const app = startMcpApp({ tools });
  cleanups.push(() => app.close());
  await daemon.connectors.presence(
    {},
    { app: "photos", kind: "app", title: "Photos", open: true, mcp: app.endpoint },
  );
  const session = daemon.sessions.create({}).id;
  daemon.settings.setApprovals(session, mode);
  return { daemon, app, session };
}

function questions(p: Photos): SessionEvent[] {
  return p.daemon.store
    .read(p.session)
    .map((r) => r.event)
    .filter((e) => e.type === "permission_request");
}

async function until(check: () => boolean, what: string): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (check()) return;
    await Bun.sleep(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** The agent's call, as neo's tools_call reaches the gateway. */
function call(
  p: Photos,
  name: string,
  args: Record<string, unknown> = {},
): Promise<CallToolResult> {
  return p.daemon.gateway.call("tools_call", { name: `photos.${name}`, args }, undefined, {
    session: p.session,
  });
}

/** Waits for the nth question and answers it. */
async function answer(p: Photos, n: number, option: string, note?: string): Promise<SessionEvent> {
  await until(() => questions(p).length > n, `question ${n}`);
  const question = questions(p)[n];
  if (question?.type !== "permission_request") throw new Error("not a permission request");
  const args: Record<string, string> = { id: question.id, option };
  if (note !== undefined) args.note = note;
  await p.daemon.actions.run({ session: p.session, ref: "x", action: "suite.permit", args }, "t");
  return question;
}

function text(result: CallToolResult): string {
  const lines: string[] = [];
  for (const block of result.content) if (block.type === "text") lines.push(block.text);
  return lines.join("\n");
}

test("the gateway keeps an app's annotations", async () => {
  const p = await setup("auto");
  const tools = p.daemon.connectors.connector("photos")?.tools ?? [];
  expect(tools.find((t) => t.name === "list")?.annotations).toEqual({ readOnlyHint: true });
  expect(tools.find((t) => t.name === "adjust")?.annotations).toEqual({ destructiveHint: false });
  expect(tools.find((t) => t.name === "export")?.annotations).toBeUndefined();
});

describe("ask", () => {
  test("a read runs unasked; an undoable edit and an export ask first", async () => {
    const p = await setup("ask");
    expect(text(await call(p, "list"))).toBe("1, 2");
    expect(questions(p)).toEqual([]);

    const declined = call(p, "adjust", { photo: "1" });
    const asked = await answer(p, 0, "reject", "not that one");
    expect(asked).toMatchObject({ agent: "neo", kind: "edit", title: "Call photos.adjust?" });
    const result = await declined;
    expect(result.isError).toBe(true);
    expect(text(result)).toBe("The user declined this call.\n\nThe user added: not that one");
    expect(p.app.calls.map((c) => c.name)).toEqual(["list"]);

    const allowed = call(p, "adjust", { photo: "1" });
    await answer(p, 1, "allow");
    expect(text(await allowed)).toBe("adjusted 1");

    const exported = call(p, "export", { photo: "1" });
    expect(await answer(p, 2, "allow")).toMatchObject({ kind: "other" });
    expect(text(await exported)).toBe("exported");
  });

  test("allow for this session holds per tool", async () => {
    const p = await setup("ask");
    const first = call(p, "export", { photo: "1" });
    await answer(p, 0, "always");
    await first;
    await call(p, "export", { photo: "2" });
    expect(questions(p)).toHaveLength(1);
    const other = call(p, "adjust", { photo: "1" });
    await answer(p, 1, "allow");
    await other;
  });

  test("a user's action in a pane has no session and is not asked about", async () => {
    const p = await setup("ask");
    const result = await p.daemon.gateway.call("tools_call", {
      name: "photos.export",
      args: { photo: "1" },
    });
    expect(text(result)).toBe("exported");
    expect(questions(p)).toEqual([]);
  });
});

for (const mode of ["review", "edits"] as const) {
  test(`${mode}: an undoable edit runs unasked, an export asks first`, async () => {
    const p = await setup(mode);
    expect(text(await call(p, "adjust", { photo: "1" }))).toBe("adjusted 1");
    expect(questions(p)).toEqual([]);
    const exported = call(p, "export", { photo: "1" });
    await answer(p, 0, "allow");
    expect(text(await exported)).toBe("exported");
  });
}

test("auto: nothing asks", async () => {
  const p = await setup("auto");
  await call(p, "adjust", { photo: "1" });
  await call(p, "export", { photo: "1" });
  expect(questions(p)).toEqual([]);
});

test("a guest's call over /mcp goes by the same rules, with the guest named", async () => {
  const p = await setup("edits");
  const client = new Client({ name: "test", version: "1" });
  await client.connect(
    new StreamableHTTPClientTransport(new URL(`${p.daemon.info.url}/mcp`), {
      requestInit: {
        headers: {
          authorization: `Bearer ${p.daemon.info.token}`,
          "x-suite-session": p.session,
          "x-suite-guest": "codex",
        },
      },
    }),
  );
  cleanups.push(() => client.close());
  const callTool = (name: string): Promise<unknown> =>
    client.callTool({ name: "tools_call", arguments: { name, args: { photo: "9" } } });

  await callTool("photos.adjust");
  expect(questions(p)).toEqual([]);
  const exported = callTool("photos.export");
  expect(await answer(p, 0, "reject")).toMatchObject({ agent: "codex", kind: "other" });
  expect(((await exported) as CallToolResult).isError).toBe(true);
  expect(p.app.calls.map((c) => c.name)).toEqual(["adjust"]);
});
