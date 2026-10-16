// Wires the browser together: the Browser core, its MCP tools, the page's server, and the bus
// connection that registers it as the "browser" app (and keeps its context current).
import type { Asset, AssetPut, AppPresence } from "@suite/protocol/types";
import { SuiteClient } from "@suite/sdk";
import { createToolServer, type ToolServer } from "@suite/sdk/tools";
import { Browser } from "./browser.ts";
import { startBrowserServer, type BrowserServer } from "./server.ts";
import { browserTemplates, browserTools } from "./tools.ts";

const MAIN_PATH = new URL("../main.ts", import.meta.url).pathname;

export interface BrowserAppOptions {
  /** Port for the page, on 127.0.0.1; 0 picks a free one. */
  port?: number;
  /** Where to look for the daemon (SUITE_SOCKET etc.); defaults to process.env. */
  env?: Record<string, string | undefined>;
  /** Keep retrying if the daemon isn't running yet (default false: reject). */
  waitForDaemon?: boolean;
  /** The bundled page script, if already built (default: built at start). */
  script?: string;
  /** Told once the page is being served, before connecting to the daemon. */
  onListening?: (url: string) => void;
}

export interface BrowserApp {
  /** The page. */
  url: string;
  browser: Browser;
  suite: SuiteClient;
  tools: ToolServer;
  server: BrowserServer;
  /** Unregisters the app and shuts both servers down. */
  stop(): Promise<void>;
}

const INSTRUCTIONS =
  "A rough web browser: navigate loads a page and returns its text, images and link count; read returns the page's text; images lists its images; download stores any file in the asset store and returns an asset:// handle for other tools.";

function presenceFor(tools: ToolServer, context: Record<string, string>): AppPresence {
  return {
    app: "browser",
    kind: "app",
    title: "Browser",
    description:
      "Browser: open pages, read their text, list and download their images (downloads become asset:// handles).",
    open: true,
    mcp: tools.endpoint,
    templates: browserTemplates,
    launch: { command: ["bun", "run", MAIN_PATH] },
    context,
  };
}

export async function startBrowserApp(options: BrowserAppOptions = {}): Promise<BrowserApp> {
  let suite: SuiteClient | undefined;
  const browser = new Browser({
    putAsset(asset: AssetPut): Promise<Asset> {
      if (!suite) return Promise.reject(new Error("not connected to the suite daemon yet"));
      return suite.putAsset(asset);
    },
  });
  const tools = createToolServer({
    name: "browser",
    instructions: INSTRUCTIONS,
    tools: browserTools(browser),
  });
  let server: BrowserServer;
  try {
    server = await startBrowserServer({
      browser,
      port: options.port,
      env: options.env,
      script: options.script,
    });
  } catch (err) {
    await tools.close();
    throw err;
  }
  options.onListening?.(server.url);

  try {
    suite = await SuiteClient.connect({
      client: "browser",
      env: options.env,
      waitForDaemon: options.waitForDaemon,
      presence: presenceFor(tools, { url: "about:blank", title: "New tab" }),
    });
  } catch (err) {
    await Promise.all([tools.close(), server.stop()]);
    throw err;
  }
  const connected = suite;

  browser.onEvent((event) => {
    if (event.type !== "page" || event.page === null) return;
    connected.setContext({ url: event.page.url, title: event.page.title }).catch(() => {});
  });

  return {
    url: server.url,
    browser,
    suite: connected,
    tools,
    server,
    async stop() {
      await connected.close();
      await Promise.all([tools.close(), server.stop()]);
    },
  };
}
