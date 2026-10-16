// Browser entry of the demo (bundled by serve.ts): connects to the daemon with the address the
// server put in the page, then hands over to startDemo.
import { assetUrl, BrowserClient, type DaemonAddress } from "@suite/sdk/browser";
import { startDemo } from "./app.ts";

function byId<T extends HTMLElement>(id: string, kind: new () => T): T {
  const found = document.getElementById(id);
  if (!(found instanceof kind)) throw new Error(`the page has no #${id}`);
  return found;
}

function readAddress(): DaemonAddress {
  const raw = byId("suite-config", HTMLScriptElement).textContent;
  const config: { url?: unknown; token?: unknown } = JSON.parse(raw);
  if (typeof config.url !== "string" || typeof config.token !== "string") {
    throw new Error("the page has no daemon address");
  }
  return { url: config.url, token: config.token };
}

/** The daemon's current address; asset:// images load from it. */
let address: DaemonAddress | undefined;

/** The server re-reads daemon.json, so a restarted daemon (new port and token) is found again. */
async function refreshAddress(): Promise<DaemonAddress> {
  const response = await fetch("/daemon");
  if (!response.ok) throw new Error(`the daemon is not running (${response.status})`);
  const next: DaemonAddress = await response.json();
  address = next;
  return next;
}

async function main(): Promise<void> {
  const status = byId("status", HTMLElement);
  try {
    address = readAddress();
    const client = await BrowserClient.connect({
      client: "web",
      ...address,
      refreshAddress,
      onError: (err) => {
        status.textContent = err.message;
      },
    });
    startDemo(
      client,
      {
        chat: byId("chat", HTMLElement),
        status,
        form: byId("form", HTMLFormElement),
        input: byId("input", HTMLInputElement),
      },
      { resolveImage: (src) => (address ? assetUrl(address, src) : undefined) },
    );
  } catch (err) {
    status.textContent = err instanceof Error ? err.message : String(err);
  }
}

await main();
