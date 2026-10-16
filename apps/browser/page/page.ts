// Browser entry of the page (bundled by server.ts): reads the config the server put in the page,
// connects to the daemon and hands over to startBrowserPage.
import { assetUrl, BrowserClient, type DaemonAddress } from "@suite/sdk/browser";
import { createApi } from "./api.ts";
import { startBrowserPage } from "./app.ts";

function byId<T extends HTMLElement>(id: string, kind: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof kind)) throw new Error(`the page has no #${id}`);
  return found;
}

interface Config {
  daemon: DaemonAddress | null;
  problem: string;
  token: string;
}

function readConfig(): Config {
  const raw = byId("browser-config", HTMLScriptElement).textContent;
  const config: Config = JSON.parse(raw);
  return config;
}

async function main(): Promise<void> {
  const status = byId("status", HTMLElement);
  const config = readConfig();
  const api = createApi(location.origin, config.token);
  let address = config.daemon;

  const els = {
    addressForm: byId("address", HTMLFormElement),
    addressInput: byId("url", HTMLInputElement),
    title: byId("title", HTMLElement),
    note: byId("note", HTMLElement),
    images: byId("images", HTMLElement),
    text: byId("text", HTMLElement),
    pane: byId("pane", HTMLElement),
    chat: byId("chat", HTMLElement),
    chatForm: byId("chat-form", HTMLFormElement),
    chatInput: byId("chat-input", HTMLInputElement),
    status,
  };
  if (!address) {
    // Browsing works without the daemon; only the chat needs it (reload once it is running).
    status.textContent = `chat unavailable: ${config.problem}`;
    startBrowserPage(undefined, api, els);
    return;
  }
  try {
    const client = await BrowserClient.connect({
      client: "browser-page",
      ...address,
      // The server re-reads daemon.json, so a restarted daemon (new port and token) is found.
      refreshAddress: async () => {
        address = await api.daemon();
        return address;
      },
      onError: (err) => {
        status.textContent = err.message;
      },
    });
    startBrowserPage(client, api, els, {
      resolveImage: (src) => (address ? assetUrl(address, src) : undefined),
    });
  } catch (err) {
    status.textContent = err instanceof Error ? err.message : String(err);
  }
}

await main();
