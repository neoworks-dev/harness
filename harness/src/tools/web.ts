// web_search and web_fetch, on @neoworks/websearch. web_search returns the engines' results as
// they are: no model summarizes them. web_fetch saves a page as markdown in a file and returns
// its path and outline, so the agent greps and reads the parts it needs instead of taking the
// whole page into its context.
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  Cooldown,
  fetchPage,
  FetchPageError,
  imageEngines,
  newsEngines,
  search,
  searchImages,
  searchNews,
  searxng,
  tinyfish,
  webEngines,
  type Engine,
  type EngineReport,
  type FetchPageOptions,
  type Fetch,
  type Hit,
  type ImageHit,
  type NewsHit,
  type NewsSearchOptions,
  type Result,
  type SafeSearch,
  type SearchOptions,
} from "@neoworks/websearch";
import { Type } from "typebox";
import type { RequestPermission } from "../agent.ts";
import type { WebConfig } from "../config.ts";
import type { AgentState, AgentTool } from "../loop/index.ts";
import { askToProceed, withNote } from "./permission.ts";

export interface WebToolsOptions {
  config: WebConfig;
  /** The user's safe-search choice, read per search; undefined: the configured one. */
  safeSearch?: () => SafeSearch | undefined;
  /** Where web_fetch saves pages; created on the first fetch. One per session. */
  dir: string;
  /** TinyFish's API key, looked up per search; with one, TinyFish is searched as well. */
  tinyfishKey?: () => Promise<string | undefined>;
  /** For tests: the fetch the searches and web_fetch call. */
  fetch?: Fetch;
  /**
   * Where blocked engines are remembered. By default the library's, shared by every session in
   * the process, so one session's block spares the others from asking; tests pass false.
   */
  cooldown?: Cooldown | false;
  /** For tests: the engines to search instead of the library's. */
  engines?: { web?: Engine[]; news?: Engine<NewsHit>[]; images?: Engine<ImageHit>[] };
}

/** How many results web_search returns unless asked for another number. */
const DEFAULT_RESULTS = 8;
const MAX_RESULTS = 20;

/** A fetched page this short comes back whole, as well as in its file. */
const INLINE_LINES = 200;
const INLINE_CHARS = 12_000;

/** At most this many headings in a fetched page's outline. */
const OUTLINE_HEADINGS = 60;

// ---- web_search ----

const searchParameters = Type.Object({
  query: Type.String({
    description: "What to search for, as you would type it into a search engine.",
  }),
  kind: Type.Optional(
    Type.Union([Type.Literal("web"), Type.Literal("news"), Type.Literal("images")], {
      description: 'What to search: "web" (the default), "news" (dated articles) or "images".',
    }),
  ),
  since: Type.Optional(
    Type.Union(
      [Type.Literal("hour"), Type.Literal("day"), Type.Literal("week"), Type.Literal("month")],
      { description: "News only: articles published within the last hour, day, week or month." },
    ),
  ),
  limit: Type.Optional(
    Type.Number({
      description: `How many results, 1 to ${MAX_RESULTS}. Default: ${DEFAULT_RESULTS}.`,
    }),
  ),
});

export interface WebSearchDetails {
  kind: "web" | "news" | "images";
  results: Result[];
  engines: EngineReport[];
}

/** One result as the model reads it: the title, the URL, the snippet, then what else is known. */
function formatResult(result: Result<Hit & Partial<NewsHit & ImageHit>>, index: number): string {
  const lines = [`${index + 1}. ${result.title}`, `   ${result.url}`];
  if (result.image) lines.push(`   image: ${result.image}`);
  if (result.snippet) lines.push(`   ${result.snippet}`);
  const facts = [result.engines.join(", ")];
  if (result.published) facts.push(result.published.slice(0, 10));
  if (result.source) facts.push(result.source);
  if (result.width && result.height) facts.push(`${result.width}×${result.height}`);
  lines.push(`   (${facts.join(" · ")})`);
  return lines.join("\n");
}

/** The engines that didn't answer, and why, so the model knows what it didn't hear from. */
function formatEngines(engines: EngineReport[]): string {
  const answered = engines
    .filter((engine) => engine.status === "ok")
    .map((engine) => engine.engine);
  const missing = engines
    .filter((engine) => engine.status !== "ok")
    .map((engine) => {
      const said = `${engine.engine} ${engine.status}`;
      return engine.detail ? `${said} (${engine.detail})` : said;
    });
  let text = `Engines: ${answered.join(", ") || "none answered"}`;
  if (missing.length > 0) text += `; not: ${missing.join(", ")}`;
  return text;
}

function createWebSearchTool(
  options: WebToolsOptions,
): AgentTool<typeof searchParameters, WebSearchDetails> {
  return {
    name: "web_search",
    label: "web_search",
    description:
      "Search the web, news or images with several search engines at once. Returns the merged " +
      "results as they are: title, URL and snippet, and for news the date and publication. " +
      "Results found by more engines come first. Read a result's page with web_fetch.",
    parameters: searchParameters,
    async execute(_toolCallId, { query, kind = "web", since, limit }, signal) {
      const count = Math.min(MAX_RESULTS, Math.max(1, Math.floor(limit ?? DEFAULT_RESULTS)));
      // Safe search is the user's setting; the agent can't change it.
      const shared: Omit<SearchOptions, "engines"> = {
        limit: count,
        safeSearch: options.safeSearch?.() ?? options.config.safeSearch,
      };
      if (signal) shared.signal = signal;
      if (options.fetch) shared.fetch = options.fetch;
      if (options.cooldown !== undefined) shared.cooldown = options.cooldown;
      let response: { results: Result[]; engines: EngineReport[] };
      if (kind === "news") {
        const news: NewsSearchOptions = {
          ...shared,
          engines: options.engines?.news ?? newsEngines,
        };
        if (since) news.since = since;
        response = await searchNews(query, news);
      } else if (kind === "images") {
        const engines = options.engines?.images ?? imageEngines;
        response = await searchImages(query, { ...shared, engines });
      } else {
        const engines = [...(options.engines?.web ?? webEngines), ...(await extraEngines(options))];
        response = await search(query, { ...shared, engines });
      }
      const body =
        response.results.length === 0
          ? `No results for "${query}".`
          : response.results.map(formatResult).join("\n\n");
      return {
        content: [{ type: "text", text: `${body}\n\n${formatEngines(response.engines)}` }],
        details: { kind, results: response.results, engines: response.engines },
      };
    },
  };
}

/** SearXNG and TinyFish, when the user has set them up; web search only. */
async function extraEngines(options: WebToolsOptions): Promise<Engine[]> {
  const engines: Engine[] = [];
  if (options.config.searxngUrl) engines.push(searxng(options.config.searxngUrl));
  const key = await options.tinyfishKey?.();
  if (key) engines.push(tinyfish(key));
  return engines;
}

// ---- web_fetch ----

const fetchParameters = Type.Object({
  url: Type.String({ description: "The page's URL, http or https." }),
});

export interface WebFetchDetails {
  url: string;
  finalUrl?: string;
  /** Where the markdown was saved. */
  path?: string;
  title?: string;
  lines?: number;
}

/** A file name for a page: its host and path, safe on every file system. */
export function fileNameFor(url: string): string {
  const { hostname, pathname } = new URL(url);
  const path = pathname
    .replace(/\/+$/, "")
    .replace(/[^\w.-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
  if (!path) return `${hostname}.md`;
  return `${hostname}-${path}.md`;
}

/** The page's headings with their line numbers, for reading parts of the file. */
function outline(lines: string[]): string[] {
  const headings: string[] = [];
  lines.forEach((line, index) => {
    if (/^#{1,6} /.test(line)) headings.push(`${String(index + 1).padStart(5)}  ${line}`);
  });
  if (headings.length <= OUTLINE_HEADINGS) return headings;
  return [...headings.slice(0, OUTLINE_HEADINGS), `  … ${headings.length - OUTLINE_HEADINGS} more`];
}

/** What to tell the model when a page loaded but isn't the content. */
const PROBLEM_ADVICE = {
  blocked: "The site's bot protection answered instead of the page. Try another source.",
  "needs-javascript":
    "The page only renders in a browser. Try another source for the same content.",
  "http-error": "The site answered with an error.",
} as const;

function createWebFetchTool(
  options: WebToolsOptions,
  ask?: RequestPermission,
): AgentTool<typeof fetchParameters, WebFetchDetails> {
  return {
    name: "web_fetch",
    label: "web_fetch",
    description:
      "Fetch a web page and save it as markdown in a file. Returns the file's path and the " +
      "page's outline (headings with line numbers); short pages also come back whole. Use grep " +
      "and read on the file for the parts you need rather than reading all of it. Plain text " +
      "and JSON are saved as they are; PDFs and other binary files can't be fetched.",
    parameters: fetchParameters,
    async execute(toolCallId, { url }, signal) {
      // A URL the model picked can carry data out in it, so the user decides (kind "fetch").
      const { note } = await askToProceed(
        ask,
        { title: `fetch ${url}`, kind: "fetch", toolCallId },
        "The user declined to let you fetch this page.",
        signal,
      );
      const fetchOptions: FetchPageOptions = {};
      if (signal) fetchOptions.signal = signal;
      if (options.fetch) fetchOptions.fetch = options.fetch;
      let page;
      try {
        page = await fetchPage(url, fetchOptions);
      } catch (error) {
        if (error instanceof FetchPageError) throw new Error(`${error.message} (${error.code})`);
        throw error;
      }
      if (page.problem) {
        const advice = PROBLEM_ADVICE[page.problem.kind];
        throw new Error(`${page.finalUrl}: ${page.problem.detail}. ${advice}`);
      }

      const header = [`# ${page.title ?? page.finalUrl}`, "", `Source: ${page.finalUrl}`];
      if (page.description) header.push(`Description: ${page.description}`);
      const file = `${header.join("\n")}\n\n${page.content}\n`;
      await mkdir(options.dir, { recursive: true });
      const path = join(options.dir, fileNameFor(page.finalUrl));
      await writeFile(path, file);

      const lines = file.split("\n");
      const summary = [`Saved ${page.finalUrl} as markdown: ${path} (${lines.length} lines).`];
      if (page.truncated) summary.push("The page was larger than 5 MB; the file has its start.");
      if (lines.length <= INLINE_LINES && file.length <= INLINE_CHARS) {
        summary.push("", file);
      } else {
        const headings = outline(lines);
        summary.push(
          "",
          headings.length > 0 ? "Outline (line, heading):" : "The page has no headings.",
        );
        summary.push(...headings);
      }
      const details: WebFetchDetails = { url, finalUrl: page.finalUrl, path, lines: lines.length };
      if (page.title) details.title = page.title;
      return { content: [{ type: "text", text: withNote(summary.join("\n"), note) }], details };
    },
  };
}

/** web_search and web_fetch. `ask` is consulted before each fetch. */
export function createWebTools(
  options: WebToolsOptions,
  ask?: RequestPermission,
): AgentState["tools"] {
  return [createWebSearchTool(options), createWebFetchTool(options, ask)];
}
