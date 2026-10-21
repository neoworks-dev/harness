// The page's server (HTTP, token, Host/Origin checks, WebSocket) and the page logic under
// happy-dom, against a real daemon and the fixture web server: what a real browser would run.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Window } from "happy-dom";
import { assetUrl, connect, fixedAddress, type Client } from "@neoworks-dev/neo-sdk";
import { startTestDaemon, type TestDaemon } from "../../../daemon/src/testing.ts";
import { startBrowserApp, type BrowserApp } from "../src/app.ts";
import { TOKEN_HEADER } from "../src/server.ts";
import type { ServerEvent } from "../src/types.ts";
import { createApi } from "../page/api.ts";
import { startBrowserPage, type BrowserPage, type PageElements } from "../page/app.ts";
import { failure, PNG, startFixture, type Fixture } from "./fixture.ts";

// Built in a child process: Bun's bundler misbehaves in a process that already ran daemons.
async function buildScript(): Promise<string> {
  const child = Bun.spawn(
    [process.execPath, "run", join(import.meta.dir, "../page/build-script.ts")],
    { stdout: "pipe", stderr: "inherit" },
  );
  const script = await new Response(child.stdout).text();
  expect(await child.exited).toBe(0);
  return script;
}

const script = await buildScript();

let daemon: TestDaemon;
let fixture: Fixture;
let app: BrowserApp;
const cleanups: (() => Promise<void> | void)[] = [];

beforeEach(async () => {
  daemon = await startTestDaemon();
  fixture = startFixture();
  app = await startBrowserApp({ env: daemon.env, script });
});
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  await app.stop();
  await fixture.stop();
  await daemon.stop(200);
});

async function until(check: () => boolean, what: string, timeoutMs = 5000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(10);
  }
}

describe("server", () => {
  test("serves the page with the daemon's address and a token, and the bundle", async () => {
    const page = await fetch(app.url);
    expect(page.status).toBe(200);
    expect(page.headers.get("content-security-policy")).toContain("script-src 'self'");
    const body = await page.text();
    for (const id of ["address", "url", "title", "images", "text", "pane", "chat", "chat-form"]) {
      expect(body).toContain(`id="${id}"`);
    }
    const config = body.match(
      /<script type="application\/json" id="browser-config">(.*)<\/script>/,
    );
    expect(JSON.parse(config?.[1] ?? "null")).toEqual({
      daemon: { url: daemon.info.url, token: daemon.info.token },
      problem: "",
      token: app.server.token,
    });

    const bundle = await (await fetch(`${app.url}/app.js`)).text();
    expect(bundle).toContain("view.subscribe");
    expect(bundle).toContain("/api/navigate");
    expect(bundle).not.toContain("node:");
    expect((await fetch(`${app.url}/nope`)).status).toBe(404);
  });

  test("refuses other hosts (DNS rebinding) and requests without the token", async () => {
    const evil = await fetch(`${app.url}/`, { headers: { host: "evil.example" } });
    expect(evil.status).toBe(403);
    const text = await evil.text();
    expect(text).not.toContain(daemon.info.token);
    expect(text).not.toContain(app.server.token);

    const post = (headers: Record<string, string>): Promise<Response> =>
      fetch(`${app.url}/api/navigate`, {
        method: "POST",
        headers: { "content-type": "application/json", ...headers },
        body: JSON.stringify({ url: fixture.url }),
      });
    expect((await post({})).status).toBe(403);
    expect((await post({ [TOKEN_HEADER]: "wrong" })).status).toBe(403);
    expect((await post({ host: "evil.example", [TOKEN_HEADER]: app.server.token })).status).toBe(
      403,
    );
    expect(fixture.hits).toEqual([]);

    expect((await fetch(`${app.url}/daemon`)).status).toBe(403);
    const daemonRes = await fetch(`${app.url}/daemon`, {
      headers: { [TOKEN_HEADER]: app.server.token },
    });
    expect(await daemonRes.json()).toEqual({ url: daemon.info.url, token: daemon.info.token });
  });

  test("the JSON endpoints navigate and download; errors are 400s with a message", async () => {
    const api = createApi(app.url, app.server.token);
    const view = await api.navigate(fixture.url);
    expect(view).toMatchObject({ title: "Fixture & Co", links: 2 });
    expect(view.images).toHaveLength(5);
    expect(app.browser.page?.url).toBe(`${fixture.url}/`);
    const done = await api.download(`${fixture.url}/logo.png`);
    expect(done).toMatchObject({ name: "logo.png", mime: "image/png", size: PNG.length });
    expect(new Uint8Array(readFileSync(daemon.assets.get(done.handle)?.path ?? ""))).toEqual(PNG);
    expect(await failure(api.navigate("ftp://example.com"))).toMatch(/only http/);
    expect(await failure(api.download(`${fixture.url}/missing`))).toMatch(/404/);
  });

  test("the WebSocket needs the token and a local origin, and pushes what the tools do", async () => {
    const wsUrl = app.url.replace("http", "ws");
    const refused = new WebSocket(`${wsUrl}/ws?token=nope`);
    const closed = await new Promise<string>((resolve) => {
      refused.onerror = () => resolve("error");
      refused.onclose = () => resolve("closed");
    });
    expect(["error", "closed"]).toContain(closed);

    await app.browser.navigate(fixture.url);
    const events: ServerEvent[] = [];
    const off = createApi(app.url, app.server.token).onEvent((event) => events.push(event));
    cleanups.push(off);
    // The current page is sent on connect...
    await until(() => events.length === 1, "initial page");
    expect(events[0]).toMatchObject({ type: "page", page: { title: "Fixture & Co" } });
    // ...and the agent's tool calls reach every connected page.
    await app.browser.navigate(`${fixture.url}/notes.txt`);
    await app.browser.download(`${fixture.url}/logo.png`);
    await until(() => events.length === 3, "pushed events");
    expect(events.map((e) => e.type)).toEqual(["page", "page", "download"]);
  });
});

interface Rig {
  page: BrowserPage;
  els: PageElements;
  window: Window;
  user: Client;
}

/** The page logic on a happy-dom document, connected like the bundled page would be. */
async function rig(): Promise<Rig> {
  const window = new Window({ url: app.url });
  const doc: Document = window.document as unknown as Document;
  const make = <T extends HTMLElement>(tag: string, ...children: HTMLElement[]): T => {
    const el = doc.createElement(tag);
    el.append(...children);
    doc.body.append(el);
    return el as T;
  };
  const addressInput = doc.createElement("input");
  const addressForm = make<HTMLFormElement>("form", addressInput);
  const chatInput = doc.createElement("input");
  const chatForm = make<HTMLFormElement>("form", chatInput);
  const els: PageElements = {
    addressForm,
    addressInput,
    title: make("h1"),
    note: make("div"),
    images: make("section"),
    text: make("pre"),
    pane: make("aside"),
    chat: make("div"),
    chatForm,
    chatInput,
    status: make("div"),
  };
  const client = await connect({ client: "browser-page", address: fixedAddress(daemon.info) });
  cleanups.push(() => client.close());
  const user = await connect({ client: "tester", address: fixedAddress(daemon.info) });
  cleanups.push(() => user.close());
  const api = createApi(app.url, app.server.token);
  const page = startBrowserPage(client, api, els, {
    resolveImage: (src) => assetUrl(daemon.info, src),
  });
  cleanups.push(() => page.stop());
  return { page, els, window, user };
}

function submit(window: Window, form: HTMLFormElement): void {
  form.dispatchEvent(new window.Event("submit", { cancelable: true }) as unknown as Event);
}

function click(window: Window, el: Element | null | undefined): void {
  el?.dispatchEvent(new window.Event("click", { bubbles: true }) as unknown as Event);
}

describe("page logic", () => {
  test("the address bar loads a page: title, text, and a tile with a Download button per image", async () => {
    const { els, window } = await rig();
    els.addressInput.value = fixture.url;
    submit(window, els.addressForm);
    await until(() => els.title.textContent === "Fixture & Co", "page");
    expect(els.text.textContent).toContain("First paragraph with bold and a link.");
    expect(els.note.textContent).toBe("5 images · 2 links");
    expect(els.addressInput.value).toBe(`${fixture.url}/`);
    const tiles = Array.from(els.images.querySelectorAll(".tile"));
    expect(tiles).toHaveLength(5);
    expect(tiles[1]?.querySelector("img")?.getAttribute("src")).toBe(`${fixture.url}/logo.png`);
    expect(tiles[1]?.querySelector(".alt")?.textContent).toBe("The logo");

    // Download stores the image as an asset and shows its handle.
    click(window, tiles[1]?.querySelector("button"));
    const handle = tiles[1]?.querySelector(".handle");
    await until(() => handle?.textContent?.startsWith("asset://img-") === true, "handle");
    const asset = daemon.assets.get(handle?.textContent ?? "");
    expect(asset).toMatchObject({ name: "logo.png", mime: "image/png", app: "browser" });
    expect(new Uint8Array(readFileSync(asset?.path ?? ""))).toEqual(PNG);
  });

  test("a bad address shows the error and keeps the page", async () => {
    const { els, window } = await rig();
    await app.browser.navigate(fixture.url);
    await until(() => els.title.textContent === "Fixture & Co", "page");
    els.addressInput.value = `${fixture.url}/missing`;
    submit(window, els.addressForm);
    await until(() => els.note.textContent?.includes("404") === true, "error");
    expect(els.title.textContent).toBe("Fixture & Co");
  });

  test("the agent's navigate and download show up live", async () => {
    const { els } = await rig();
    await until(() => els.status.textContent?.startsWith("session") === true, "connected");
    await Bun.sleep(100); // the page's WebSocket
    await app.browser.navigate(fixture.url);
    await until(() => els.title.textContent === "Fixture & Co", "live page");
    expect(els.images.querySelectorAll(".tile")).toHaveLength(5);

    await app.browser.download(`${fixture.url}/logo.png`);
    const handle = els.images.querySelectorAll(".tile")[1]?.querySelector(".handle");
    await until(() => handle?.textContent?.startsWith("asset://img-") === true, "live handle");
    expect(els.status.textContent).toContain("downloaded logo.png as asset://");
  });

  test("the chat pane follows the current session, sends as app browser, lights up on focus", async () => {
    const { page, els, window, user } = await rig();
    await until(() => page.session !== undefined, "focus");
    expect(page.session).toBe(daemon.focus.current().session);
    expect(els.pane.classList.contains("focused")).toBe(false);

    els.chatInput.value = "hello there";
    submit(window, els.chatForm);
    await until(() => els.chat.textContent?.includes("echo #1: hello there") === true, "echo");
    expect(els.chat.querySelector('[data-kind="user"] header')?.textContent).toBe("» browser");
    expect(els.chatInput.value).toBe("");

    // The agent moves into the browser: same session, pane highlighted.
    await user.moveAgent({ app: "browser" });
    await until(() => els.pane.classList.contains("focused"), "highlight");
    expect(els.chat.textContent).toContain("echo #1: hello there");

    // A new current session replaces the pane's content.
    const other = await user.createSession({ title: "other" });
    await user.moveAgent({ app: "browser", session: other.id });
    await until(() => page.session === other.id, "session switch");
    await until(() => els.chat.textContent?.includes("hello there") === false, "cleared");
    expect(els.pane.classList.contains("focused")).toBe(true);

    // The agent leaves: the pane stays on the session but loses the highlight.
    await user.moveAgent({});
    await until(() => !els.pane.classList.contains("focused"), "highlight off");
    expect(page.session).toBe(other.id);
  });

  test("asset:// images in the chat load from the daemon's asset endpoint", async () => {
    const { els, user } = await rig();
    await until(() => els.status.textContent?.startsWith("session") === true, "connected");
    const session = daemon.focus.current().session;
    await app.browser.navigate(fixture.url);
    await user.action(session, "x", "browser.download", { url: `${fixture.url}/logo.png` });

    await until(() => els.chat.querySelector("figure img") !== null, "image in chat");
    const imgs = Array.from(els.chat.querySelectorAll("figure img"));
    const src = imgs.at(-1)?.getAttribute("src") ?? "";
    expect(src).toStartWith(`${daemon.info.url}/assets/img-`);
    expect(src).toContain(`?token=${daemon.info.token}`);
    const res = await fetch(src);
    expect(res.status).toBe(200);
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(PNG);
  });
});
