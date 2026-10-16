// The whole app against a real daemon (echo agent): presence, tools through the gateway (a button
// press runs the tool like the agent would), templates in the resulting view, and assets that
// land in the daemon's store with the right bytes.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { Asset, Component, ConnectorInfo, ViewItem } from "@suite/protocol/types";
import { SuiteClient } from "@suite/sdk";
import { startTestDaemon, type TestDaemon } from "../../../daemon/src/testing.ts";
import { startBrowserApp, type BrowserApp } from "../src/app.ts";
import { PNG, startFixture, type Fixture } from "./fixture.ts";

let daemon: TestDaemon;
let fixture: Fixture;
let app: BrowserApp;
let user: SuiteClient;
let session: string;

beforeEach(async () => {
  daemon = await startTestDaemon();
  fixture = startFixture();
  app = await startBrowserApp({ env: daemon.env, script: "// not built in this test" });
  user = await SuiteClient.connect({ client: "tester", env: daemon.env });
  session = daemon.focus.current().session;
});
afterEach(async () => {
  await user.close();
  await app.stop();
  await fixture.stop();
  await daemon.stop(200);
});

async function connector(): Promise<ConnectorInfo | undefined> {
  return (await user.listConnectors()).find((c) => c.app === "browser");
}

/** Presses a button the way a renderer would; resolves with the tool item it added. */
async function press(action: string, args: Record<string, unknown>): Promise<ViewItem> {
  const before = daemon.views.get(session).snapshot().items.length;
  await user.action(session, "x", action, args);
  const items = daemon.views.get(session).snapshot().items;
  const item = items.at(-1);
  expect(items.length).toBeGreaterThan(before);
  if (!item) throw new Error("no item");
  return item;
}

function all(component: Component, type: string): Component[] {
  const found: Component[] = [];
  if (component.type === type) found.push(component);
  if ("children" in component) for (const c of component.children) found.push(...all(c, type));
  return found;
}

describe("registration", () => {
  test("announces itself with tools, templates, launch command and context", async () => {
    const info = await connector();
    expect(info).toMatchObject({
      app: "browser",
      kind: "app",
      title: "Browser",
      connected: true,
      context: { url: "about:blank" },
    });
    expect(info?.tools.map((t) => t.name).sort()).toEqual([
      "browser.download",
      "browser.images",
      "browser.navigate",
      "browser.read",
    ]);
    for (const tool of ["navigate", "read", "images", "download"]) {
      expect(daemon.connectors.template(`browser.${tool}`)).toBeDefined();
    }
    const launch = daemon.connectors.connector("browser")?.launch;
    expect(launch?.command.slice(0, 2)).toEqual(["bun", "run"]);
    expect(launch?.command[2]?.endsWith("/apps/browser/main.ts")).toBe(true);
  });
});

describe("tools through the gateway", () => {
  test("navigate renders a card with thumbnails and download buttons, and updates the context", async () => {
    const item = await press("browser.navigate", { url: fixture.url });
    expect(item).toMatchObject({
      kind: "tool",
      label: "browser.navigate",
      state: "done",
      component: { type: "card", title: "Fixture & Co" },
    });
    const images = all(item.component, "image");
    expect(images.map((i) => i.type === "image" && i.src)).toEqual([
      `${fixture.url}/og.png`,
      `${fixture.url}/logo.png`,
      `${fixture.url}/large.png`,
    ]);
    const buttons = all(item.component, "button");
    expect(buttons.map((b) => b.type === "button" && [b.label, b.key, b.action])).toEqual([
      ["Download 1", "1", "browser.download"],
      ["Download 2", "2", "browser.download"],
      ["Download 3", "3", "browser.download"],
    ]);
    expect(buttons[1]).toMatchObject({ args: { url: `${fixture.url}/logo.png` } });

    const context = (): Promise<Record<string, string> | undefined> =>
      connector().then((c) => c?.context);
    const expected = { url: `${fixture.url}/`, title: "Fixture & Co" };
    for (let i = 0; i < 100 && JSON.stringify(await context()) !== JSON.stringify(expected); i++) {
      await Bun.sleep(20);
    }
    expect(await context()).toEqual(expected);
  });

  test("a thumbnail's button downloads into the daemon's asset store", async () => {
    const nav = await press("browser.navigate", { url: fixture.url });
    const button = all(nav.component, "button")[1];
    if (button?.type !== "button") throw new Error("no button");
    const item = await press(button.action, button.args as Record<string, unknown>);
    expect(item).toMatchObject({ label: "browser.download", state: "done" });

    const [image] = all(item.component, "image");
    if (image?.type !== "image") throw new Error("no image");
    expect(image.src).toMatch(/^asset:\/\/img-[0-9a-f]{12}$/);
    const copy = all(item.component, "button").map((b) => b.type === "button" && b.action);
    expect(copy).toEqual([`copy:${image.src}`, `copy:${fixture.url}/logo.png`]);

    const asset = daemon.assets.get(image.src);
    expect(asset).toMatchObject({ name: "logo.png", mime: "image/png", size: PNG.length });
    expect(asset?.app).toBe("browser");
    expect(new Uint8Array(readFileSync(asset?.path ?? ""))).toEqual(PNG);
    expect(asset).toBeDefined();
    expect(await user.getAsset(image.src)).toEqual(asset as Asset);
  });

  test("download of a non-image is a file card; read and images have templates too", async () => {
    await press("browser.navigate", { url: fixture.url });
    const file = await press("browser.download", { url: "/notes.txt" });
    expect(file.component).toMatchObject({ type: "card", title: "Downloaded notes.txt" });
    expect(all(file.component, "image")).toHaveLength(0);
    expect(all(file.component, "file")).toHaveLength(1);

    const read = await press("browser.read", {});
    expect(read.component).toMatchObject({ type: "text" });
    const list = await press("browser.images", {});
    expect(all(list.component, "table")[0]).toMatchObject({ columns: ["alt", "src"] });
  });

  test("errors come back as failed tool calls", async () => {
    const item = await press("browser.navigate", { url: "ftp://example.com/" });
    expect(item.state).toBe("failed");
    expect(JSON.stringify(item.component)).toContain("only http(s)");
  });
});

describe("page changes are pushed", () => {
  test("the app's event stream sees what the agent's tools do", async () => {
    const seen: string[] = [];
    app.browser.onEvent((event) => seen.push(event.type));
    await press("browser.navigate", { url: fixture.url });
    await press("browser.download", { url: `${fixture.url}/logo.png` });
    expect(seen).toEqual(["page", "download"]);
  });
});
