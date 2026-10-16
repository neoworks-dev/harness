// The Browser core and its tool handlers against the fixture web server, with a fake asset store.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Asset, AssetPut } from "@suite/protocol/types";
import { Browser } from "../src/browser.ts";
import { MAX_PAGE_BYTES } from "../src/fetch.ts";
import { browserTools } from "../src/tools.ts";
import type { ServerEvent } from "../src/types.ts";
import { failure, PNG, startFixture, type Fixture } from "./fixture.ts";

let fixture: Fixture;
let stored: AssetPut[];
let browser: Browser;
let events: ServerEvent[];

beforeEach(() => {
  fixture = startFixture();
  stored = [];
  events = [];
  browser = new Browser({
    putAsset(asset: AssetPut): Promise<Asset> {
      stored.push(asset);
      return Promise.resolve({
        handle: `asset://img-${String(stored.length).padStart(12, "0")}`,
        path: "/nowhere",
        name: asset.name ?? "?",
        mime: asset.mime ?? "image/png",
        size: Buffer.from(asset.data ?? "", "base64").length,
        createdAt: 0,
      });
    },
  });
  browser.onEvent((event) => events.push(event));
});
afterEach(async () => {
  await fixture.stop();
});

function tool(name: string): (args: Record<string, unknown>) => Promise<unknown> {
  const spec = browserTools(browser).find((t) => t.name === name);
  if (!spec) throw new Error(`no tool ${name}`);
  return (args) => spec.handler(args);
}

describe("navigate", () => {
  test("loads a page, follows redirects, and tells the UI", async () => {
    const page = await browser.navigate(`${fixture.url}/redirect`);
    expect(page.url).toBe(`${fixture.url}/`);
    expect(page.title).toBe("Fixture & Co");
    expect(page.images).toHaveLength(5);
    expect(browser.page).toBe(page);
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({
      type: "page",
      page: { url: `${fixture.url}/`, title: "Fixture & Co", links: 2 },
    });
  });

  test("the tool returns structured content for the templates", async () => {
    const result = await tool("navigate")({ url: `${fixture.url}/` });
    expect(result).toMatchObject({
      structuredContent: {
        url: `${fixture.url}/`,
        title: "Fixture & Co",
        imageCount: 5,
        links: 2,
      },
    });
    const text = JSON.stringify(result);
    expect(text).toContain(`${fixture.url}/og.png`);
    expect(text).toContain("Hello page");
    expect(text).toContain("First paragraph with bold and a link.");
  });

  test("accepts bare hosts, relative paths and rejects other schemes", async () => {
    await browser.navigate(`${fixture.url.replace("http://", "")}/notes.txt`).catch(() => {});
    // A bare host gets https, which the plain-http fixture can't answer.
    expect(browser.page).toBeUndefined();
    await browser.navigate(`${fixture.url}/`);
    const next = await browser.navigate("/notes.txt");
    expect(next.url).toBe(`${fixture.url}/notes.txt`);
    expect(next.text).toBe("plain words");
    for (const bad of ["file:///etc/passwd", "ftp://example.com/", "javascript:alert(1)", ""]) {
      expect(await failure(browser.navigate(bad))).not.toBe("(did not reject)");
    }
    expect(browser.page).toBe(next);
  });

  test("non-HTML: images show themselves, binaries are refused, 404s are errors", async () => {
    const image = await browser.navigate(`${fixture.url}/logo.png`);
    expect(image.images).toEqual([{ src: `${fixture.url}/logo.png`, alt: "logo.png" }]);
    expect(await failure(browser.navigate(`${fixture.url}/archive.zip`))).toMatch(
      /application\/zip.*download/,
    );
    expect(await failure(browser.navigate(`${fixture.url}/missing`))).toMatch(/404/);
    expect(browser.page).toBe(image);
  });

  test("pages are cut at 2 MB", async () => {
    const page = await browser.navigate(`${fixture.url}/bigpage`);
    expect(page.text.length).toBeLessThan(MAX_PAGE_BYTES + 100);
    expect(page.text).toContain("[cut at");
  });
});

describe("read and images", () => {
  test("need a page first", async () => {
    expect(await failure(tool("read")({}))).toMatch(/navigate first/);
    expect(await failure(tool("images")({}))).toMatch(/navigate first/);
  });

  test("return the current page's text and images", async () => {
    await browser.navigate(`${fixture.url}/`);
    const read = await tool("read")({});
    expect(read).toMatchObject({ structuredContent: { title: "Fixture & Co" } });
    expect(JSON.stringify(read)).toContain("Second paragraph <escaped>.");
    const images = await tool("images")({});
    expect(images).toMatchObject({ structuredContent: { count: 5 } });
    expect(JSON.stringify(images)).toContain("The logo");
  });
});

describe("download", () => {
  test("stores the bytes (relative urls resolve against the page) and tells the UI", async () => {
    await browser.navigate(`${fixture.url}/`);
    const result = await tool("download")({ url: "/logo.png" });
    expect(result).toMatchObject({
      structuredContent: {
        handle: "asset://img-000000000001",
        name: "logo.png",
        mime: "image/png",
        size: PNG.length,
        url: `${fixture.url}/logo.png`,
        isImage: true,
        isOther: false,
      },
    });
    expect(Buffer.from(stored[0]?.data ?? "", "base64")).toEqual(Buffer.from(PNG));
    expect(stored[0]).toMatchObject({ name: "logo.png", mime: "image/png" });
    expect(events.at(-1)).toMatchObject({ type: "download", download: { name: "logo.png" } });
  });

  test("lets the store sniff generic content types, and names files after the host if need be", async () => {
    await browser.download(`${fixture.url}/download`);
    expect(stored[0]?.mime).toBeUndefined();
    expect(stored[0]?.name).toBe("download");
    await browser.download(`${fixture.url}/notes.txt`);
    expect(stored[1]).toMatchObject({ name: "notes.txt" });
    expect(stored[1]?.mime).toBeUndefined();
  });

  test("refuses more than 10 MB, relative urls without a page, and other schemes", async () => {
    expect(await failure(browser.download(`${fixture.url}/big.bin`))).toMatch(/too large/);
    expect(await failure(browser.download("logo.png"))).toMatch(/not a valid URL/);
    expect(await failure(browser.download("file:///etc/passwd"))).toMatch(/http/);
    expect(stored).toHaveLength(0);
  });
});
