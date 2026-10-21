// The demo against a real daemon (echo agent): the HTTP side with fetch, and the page logic
// (startDemo) driven through happy-dom with the browser SDK, which together are what a browser
// would run.
import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { startTestDaemon, type TestDaemon } from "../../../daemon/src/testing.ts";
import { connect, fixedAddress } from "@neoworks-dev/neo-sdk";
import { startDemo, type Demo } from "../demo/app.ts";
import { startDemoServer, type DemoServer } from "../demo/server.ts";
import { newPage } from "./dom.ts";

// Built in a child process, see demo/build-script.ts.
async function buildScript(): Promise<string> {
  const child = Bun.spawn(
    [process.execPath, "run", join(import.meta.dir, "../demo/build-script.ts")],
    {
      stdout: "pipe",
      stderr: "inherit",
    },
  );
  const script = await new Response(child.stdout).text();
  expect(await child.exited).toBe(0);
  return script;
}

const script = await buildScript();

const cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function until(check: () => boolean, what: string, timeoutMs = 5000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(10);
  }
}

async function daemon(): Promise<TestDaemon> {
  const d = await startTestDaemon();
  cleanups.push(() => d.stop(200));
  return d;
}

async function server(env: Record<string, string>): Promise<DemoServer> {
  const s = await startDemoServer({ env, script });
  cleanups.push(() => s.stop());
  return s;
}

describe("demo server", () => {
  test("serves the page with the daemon's address, and the bundle", async () => {
    const d = await daemon();
    const s = await server(d.env);

    const page = await fetch(s.url);
    expect(page.status).toBe(200);
    const body = await page.text();
    expect(body).toContain('id="chat"');
    expect(body).toContain('<script type="module" src="/app.js">');
    const config = body.match(/<script type="application\/json" id="suite-config">(.*)<\/script>/);
    expect(JSON.parse(config?.[1] ?? "null")).toEqual({ url: d.info.url, token: d.info.token });

    const bundle = await fetch(`${s.url}/app.js`);
    expect(bundle.status).toBe(200);
    expect(bundle.headers.get("content-type")).toContain("javascript");
    const code = await bundle.text();
    expect(code).toContain("view.subscribe");
    expect(code).toContain("suite-chat");
    expect(code).not.toContain("node:");

    const address = await fetch(`${s.url}/daemon`);
    expect(await address.json()).toEqual({ url: d.info.url, token: d.info.token });
    expect((await fetch(`${s.url}/nope`)).status).toBe(404);
  });

  test("refuses requests for other hosts (DNS rebinding)", async () => {
    const d = await daemon();
    const s = await server(d.env);
    const res = await fetch(`${s.url}/`, { headers: { host: "evil.example" } });
    expect(res.status).toBe(403);
    expect(await res.text()).not.toContain(d.info.token);
  });

  test("says so when no daemon is running", async () => {
    const s = await server({ SUITE_SOCKET: "/nonexistent-dir/bus.sock" });
    const page = await fetch(s.url);
    expect(page.status).toBe(503);
    expect(await page.text()).toContain("not running");
    expect((await fetch(`${s.url}/daemon`)).status).toBe(503);
  });
});

describe("demo page logic against a real daemon", () => {
  test("follows the current session, shows the echo reply, streams and handles reconnects", async () => {
    const d = await daemon();
    const client = await connect({ client: "web", address: fixedAddress(d.info) });
    cleanups.push(() => client.close());

    const { window, container } = newPage();
    const doc = window.document;
    const status = doc.createElement("div");
    const form = doc.createElement("form");
    const input = doc.createElement("input");
    form.append(input);
    doc.body.append(status, form);
    const demo: Demo = startDemo(client, {
      chat: container,
      status: status as unknown as HTMLElement,
      form: form as unknown as HTMLFormElement,
      input: input as unknown as HTMLInputElement,
    });
    cleanups.push(() => demo.stop());

    await until(() => demo.session !== undefined, "focus");
    expect(demo.session).toBe(client.focus.value?.session);
    // The daemon's theme arrived and was applied.
    await until(() => container.style.getPropertyValue("--suite-accent") !== "", "theme");

    // Typing in the box sends session.input from app "web".
    input.value = "hello";
    form.dispatchEvent(new window.Event("submit", { cancelable: true }));
    await until(() => container.textContent.includes("echo #1: hello"), "echo reply");
    const user = container.querySelector('[data-kind="user"]');
    expect(user?.querySelector("header")?.textContent).toBe("» web");
    expect(input.value).toBe("");

    // A second turn arrives as patches on top of the first.
    const first = container.querySelector('[data-kind="agent"]');
    await demo.send("again");
    await until(() => container.textContent.includes("echo #2: again"), "second reply");
    expect(container.querySelector('[data-kind="agent"]') === first).toBe(true);

    // Moving the agent to another session switches the view.
    const other = await client.createSession({ title: "other" });
    await client.moveAgent({ session: other.id });
    await until(() => demo.session === other.id, "focus change");
    await until(() => !container.textContent.includes("hello"), "old session cleared");
    await demo.send("elsewhere");
    await until(() => container.textContent.includes("echo #1: elsewhere"), "reply in new session");
  });
});
