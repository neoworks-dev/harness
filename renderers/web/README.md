# renderers/web

TS renderer (`@suite/renderer-web`): draws resolved view items with DOM and CSS, no framework. See ../README.md for the conventions every renderer follows.

```ts
import { mountChat } from "@suite/renderer-web";
import { connect, fixedAddress } from "@neoworks-dev/neo-sdk";

const client = await connect({ client: "web", address: fixedAddress({ url, token }) });
const chat = mountChat(document.querySelector("#chat"), {
  theme: client.theme.value, // or chat.setTheme(theme) when client.theme.on fires
  onAction: (ref, action, args) => void client.action(session, ref, action, args),
});
client.subscribeView(session, {
  onSnapshot: (snapshot) => chat.applySnapshot(snapshot),
  onPatch: (patch) => chat.applyPatch(patch),
});
```

## `mountChat(container, options)`

`options`: `theme?`, `onAction?(ref, action, args)`, `clipboard?` (default `navigator.clipboard`), `onError?`, `resolveImage?(src)`, `injectStyles?` (default true: one `<style data-suite-chat-styles>` in the document head). It owns the container's children: give it an empty element (it becomes a scrollable flex column and stays pinned to the bottom while you are there).

Returns `{ setItems(items), applySnapshot(snapshot), applyPatch(patch), setTheme(theme), items, destroy() }`.

- `setItems` / `applySnapshot` replace the list but reuse the DOM of items that did not change (so a re-snapshot after a reconnect keeps focus and scroll).
- `applyPatch` goes through `ViewState` (`@neoworks-dev/neo-sdk`), which ignores patches for another session or older than the snapshot, then updates the DOM per op: `set` replaces just that item's element (appends if new; keeps focus and a thought's open state), `remove` drops it, and a streaming `blocks` op removes the elements of `blocks[from:]` of the text component at `path` and draws the new tail, leaving the earlier blocks untouched. If the DOM can't follow (it should never happen) the item is redrawn.
- `setTheme` writes CSS custom properties on the container (`--suite-fg/bg/surface/border/accent/muted/success/danger`, `--suite-space-s/m/l`, `--suite-{title,body,code}-{size,weight,font}`); the stylesheet (`chatStyles` from `@suite/renderer-web/styles`) uses only those for colors, spacing and type, so a new theme restyles the chat without new CSS.

Keyboard: items with buttons are focusable (`tabindex=0`; Tab, or Arrow Up/Down between them). While an item (or a button inside it) has focus, pressing a button's `key` triggers it; a button without `key` answers to its 1-based position among the item's buttons, as in the TUI. Each button shows its key as a `<kbd>` hint. `copy:<text>` actions write `<text>` to the clipboard (the button gets `data-copied` for a moment); everything else goes to `onAction`.

Markup is semantic (`article`/`details`, `section`+`h3` for cards, `table`, `dl`, `figure`, `progressbar`, real `button`s). Links only render as `<a>` for http(s) and mailto. Images: `data:image/*` and `http(s):` render as `<img>` (falling back to the text box if they fail to load); `asset://` handles and bare paths show the fallback box `[image W×H · src]` unless the host passes `resolveImage`, which maps an `src` to a URL (e.g. `asset://img-0123456789ab` to `${daemonUrl}/assets/img-0123456789ab?token=…`, the daemon's asset endpoint). Only `http(s):` and `data:image/` results are used; anything else the resolver returns is ignored.

## Tests and snapshots

`bun test renderers/web` (happy-dom). Every `fixtures/<name>/resolved.json` is drawn with `protocol/themes/default.json` and compared with `fixtures/<name>/web.html` (one element per line); `UPDATE_SNAPSHOTS=1 bun test renderers/web` rewrites them. Other tests cover patches, keyboard, copy vs action routing, the theme, and the demo against a real daemon.

## Demo

A page that follows the daemon's current session in a real browser:

```sh
SUITE_AGENT=echo bun run daemon        # or your usual daemon
bun run renderers/web/demo/serve.ts    # prints http://127.0.0.1:PORT; --port N to pick one
```

Open the printed URL. `serve.ts` reads `daemon.json` (honouring `SUITE_SOCKET`) server-side and puts the daemon's loopback URL and token in the page; the page connects from the browser with `@neoworks-dev/neo-sdk`, follows `agent.focus.session` (re-subscribing when the agent moves), draws it with `mountChat`, and sends the input box as `session.input` from app `web`. If the daemon restarts, the page reconnects and asks `/daemon` for the new address. The bundle is built with `Bun.build` at startup. The server only answers requests addressed to its own `127.0.0.1:PORT` / `localhost:PORT` (the page carries the daemon token, so DNS rebinding must not reach it).

Files: `demo/serve.ts` (CLI), `demo/server.ts` (HTTP side), `demo/page.ts` (browser entry), `demo/app.ts` (page behavior, also run by the tests under happy-dom).
