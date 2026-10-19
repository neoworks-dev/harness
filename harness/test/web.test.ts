import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import type { Engine, Fetch, Hit } from "@neoworks/websearch";
import type { PermissionAsk, RequestPermission } from "../src/agent.ts";
import { createCoreTools } from "../src/tools/core.ts";
import { createWebTools, fileNameFor, type WebToolsOptions } from "../src/tools/web.ts";
import { errorMessage, resultText, tempDir } from "./helpers.ts";

/** An engine whose answer is the hits as JSON; its request URL says what it was asked. */
function engine(name: string): Engine {
  return {
    name,
    weight: 1,
    safeSearch: "supported",
    request: (query, options) => ({
      url: `https://${name}.example/?q=${encodeURIComponent(query)}&safe=${options.safeSearch}`,
    }),
    parse: (response) => {
      const hits: Hit[] = JSON.parse(response.body);
      return hits;
    },
  };
}

/** Serves `pages` by URL prefix, recording what was asked. */
function site(pages: Record<string, string | Response>): Fetch & { asked: string[] } {
  const asked: string[] = [];
  const fake = (url: string): Promise<Response> => {
    asked.push(url);
    const prefix = Object.keys(pages).find((candidate) => url.startsWith(candidate));
    const page = prefix === undefined ? undefined : pages[prefix];
    if (page === undefined) return Promise.resolve(new Response("[]"));
    if (typeof page !== "string") return Promise.resolve(page.clone());
    return Promise.resolve(new Response(page, { headers: { "content-type": "text/html" } }));
  };
  return Object.assign(fake, { asked });
}

async function tools(
  fetch: Fetch,
  overrides: Partial<WebToolsOptions> = {},
  ask?: RequestPermission,
): Promise<{
  search: ReturnType<typeof createWebTools>[number];
  fetchTool: ReturnType<typeof createWebTools>[number];
  dir: string;
}> {
  const dir = join(await tempDir(), "web");
  const [search, fetchTool] = createWebTools(
    {
      config: { safeSearch: "moderate" },
      dir,
      fetch,
      engines: { web: [engine("one"), engine("two")] },
      cooldown: false,
      ...overrides,
    },
    ask,
  );
  if (!search || !fetchTool) throw new Error("expected two tools");
  return { search, fetchTool, dir };
}

function hits(...urls: string[]): string {
  return JSON.stringify(
    urls.map((url) => ({ url, title: `Title of ${url}`, snippet: "About it." })),
  );
}

test("web_search lists the merged results and says which engines answered", async () => {
  const fetch = site({
    "https://one.example/": hits("https://a.example/", "https://b.example/"),
    "https://two.example/": new Response("", { status: 429 }),
  });
  const { search } = await tools(fetch);
  const result = await search.execute("t", { query: "rust" });
  expect(resultText(result)).toBe(
    [
      "1. Title of https://a.example/",
      "   https://a.example/",
      "   About it.",
      "   (one)",
      "",
      "2. Title of https://b.example/",
      "   https://b.example/",
      "   About it.",
      "   (one)",
      "",
      "Engines: one; not: two blocked (HTTP 429)",
    ].join("\n"),
  );
});

test("web_search uses the user's safe search, which the agent can't set", async () => {
  const fetch = site({});
  const { search } = await tools(fetch, { config: { safeSearch: "strict" } });
  await search.execute("t", { query: "q" });
  expect(fetch.asked).toEqual([
    "https://one.example/?q=q&safe=strict",
    "https://two.example/?q=q&safe=strict",
  ]);
});

test("web_search takes the user's choice in the daemon over the configured safe search", async () => {
  const fetch = site({});
  let chosen: "off" | undefined;
  const { search } = await tools(fetch, { safeSearch: () => chosen });
  await search.execute("t", { query: "q" });
  chosen = "off";
  await search.execute("t", { query: "q" });
  expect(fetch.asked.map((url) => new URL(url).searchParams.get("safe"))).toEqual([
    "moderate",
    "moderate",
    "off",
    "off",
  ]);
});

test("web_search adds the user's SearXNG and TinyFish to web searches", async () => {
  const fetch = site({});
  const { search } = await tools(fetch, {
    config: { safeSearch: "moderate", searxngUrl: "http://searx.lan" },
    tinyfishKey: () => Promise.resolve("tf-key"),
  });
  const result = await search.execute("t", { query: "q" });
  expect(fetch.asked.map((url) => new URL(url).host)).toEqual([
    "one.example",
    "two.example",
    "searx.lan",
    "api.search.tinyfish.ai",
  ]);
  expect(resultText(result)).not.toContain("tf-key");
});

test("web_search says when nothing was found", async () => {
  const { search } = await tools(site({}));
  const result = await search.execute("t", { query: "nothing" });
  expect(resultText(result)).toStartWith('No results for "nothing".');
});

test("web_fetch asks first, saves the page as markdown and returns a short page whole", async () => {
  const asked: PermissionAsk[] = [];
  const ask: RequestPermission = (request) => {
    asked.push(request);
    return Promise.resolve({ allowed: true });
  };
  const fetch = site({
    "https://1.1.1.1/docs/intro": "<title>Intro</title><h1>Intro</h1><p>Hello <b>there</b>.</p>",
  });
  const { fetchTool, dir } = await tools(fetch, {}, ask);
  const result = await fetchTool.execute("t1", { url: "https://1.1.1.1/docs/intro" });
  expect(asked).toEqual([
    { title: "fetch https://1.1.1.1/docs/intro", kind: "fetch", toolCallId: "t1" },
  ]);
  const path = join(dir, "1.1.1.1-docs-intro.md");
  const file = "# Intro\n\nSource: https://1.1.1.1/docs/intro\n\n# Intro\n\nHello there.\n";
  expect(await readFile(path, "utf8")).toBe(file);
  expect(resultText(result)).toBe(
    `Saved https://1.1.1.1/docs/intro as markdown: ${path} (8 lines).\n\n${file}`,
  );
});

test("web_fetch returns an outline with line numbers for a long page", async () => {
  const sections = Array.from(
    { length: 30 },
    (_, index) => `<h2>Part ${index + 1}</h2>${"<p>Some text.</p>".repeat(8)}`,
  ).join("");
  const { fetchTool } = await tools(site({ "https://1.1.1.1/long": `<h1>Long</h1>${sections}` }));
  const text = resultText(await fetchTool.execute("t", { url: "https://1.1.1.1/long" }));
  expect(text).toContain("Outline (line, heading):");
  expect(text).toContain("    5  # Long");
  expect(text).toContain("## Part 30");
  expect(text).not.toContain("Some text.");
});

test("web_fetch fails with what was wrong: refused, blocked, unreachable, private", async () => {
  const refuse: RequestPermission = () =>
    Promise.resolve({ allowed: false, note: "not that site" });
  const refused = await tools(site({}), {}, refuse);
  expect(await errorMessage(refused.fetchTool.execute("t", { url: "https://1.1.1.1/" }))).toBe(
    "The user declined to let you fetch this page.\n\nThe user added: not that site",
  );

  const blocked = await tools(
    site({
      "https://1.1.1.1/": new Response("<title>Just a moment...</title><div id=cf-chl-widget>", {
        status: 403,
        headers: { "content-type": "text/html" },
      }),
    }),
  );
  expect(await errorMessage(blocked.fetchTool.execute("t", { url: "https://1.1.1.1/" }))).toBe(
    "https://1.1.1.1/: a Cloudflare challenge (HTTP 403). The site's bot protection answered " +
      "instead of the page. Try another source.",
  );

  const down = await tools(() => Promise.reject(new TypeError("Connection refused")));
  expect(await errorMessage(down.fetchTool.execute("t", { url: "https://1.1.1.1/" }))).toBe(
    "1.1.1.1: Connection refused (unreachable)",
  );

  const local = await tools(site({}));
  expect(await errorMessage(local.fetchTool.execute("t", { url: "http://127.0.0.1:8080/" }))).toBe(
    "127.0.0.1 is on a private network (127.0.0.1) (private-address)",
  );
});

test("pages are named after their host and path", () => {
  expect(fileNameFor("https://en.wikipedia.org/wiki/TypeScript")).toBe(
    "en.wikipedia.org-wiki-TypeScript.md",
  );
  expect(fileNameFor("https://example.com/")).toBe("example.com.md");
  expect(fileNameFor("https://example.com/a b/c?x=1")).toBe("example.com-a-20b-c.md");
});

test("the core tools include the web tools when given their settings", async () => {
  const cwd = await tempDir();
  const names = (web?: WebToolsOptions): string[] =>
    createCoreTools(cwd, undefined, undefined, undefined, web).map((tool) => tool.name);
  expect(names()).not.toContain("web_search");
  const web = { config: { safeSearch: "moderate" as const }, dir: join(cwd, "web") };
  // The gateway tools follow, connected or not.
  expect(names(web).slice(0, 7)).toEqual([
    "bash",
    "read",
    "write",
    "edit",
    "todo_write",
    "web_search",
    "web_fetch",
  ]);
});
