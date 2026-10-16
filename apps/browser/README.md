# apps/browser

A rough stand-in browser (`@suite/browser`, TS on Bun) for trying the whole suite end to end: the agent can open pages, read them and download images into the asset store, and the user gets a page with an address bar, the extracted text, the page's images and the shared chat.

```sh
bun run daemon                      # or your usual daemon
bun run apps/browser/main.ts        # --port N to pick the port; waits for the daemon if it isn't up
```

The page URL (`http://127.0.0.1:PORT`) goes to stderr; open it in a browser yourself, nothing is launched. The app registers as `browser` (title Browser, `launch` set so `apps_launch` can start it, `context` `{url, title}` kept current) with an MCP tool server on loopback.

## Tools

Fetching happens in the app with `fetch` (redirects followed, http(s) only, pages cut at 2 MB, downloads refused above 10 MB).

| Tool       | Args  | Result (`structuredContent`)                                                            |
| ---------- | ----- | --------------------------------------------------------------------------------------- |
| `navigate` | `url` | `{url, title, text (4000 chars), excerpt, images [{src, alt}] (20), imageCount, links}` |
| `read`     |       | `{url, title, text (50,000 chars), chars}`                                              |
| `images`   |       | `{url, title, images [{src, alt}], count}`                                              |
| `download` | `url` | `{handle, name, mime, size, url, path, isImage, isOther}` (stored with `asset.put`)     |

`navigate` accepts a bare host (`example.com` gets https) or a URL relative to the current page; `download` accepts relative URLs too. Images come from `<img src>`, the best `srcset` candidate, `<picture><source>`, `og:image`/`twitter:image` (first) and icons (last); tracking pixels, `data:` URLs and duplicates are dropped. Text is extracted with Bun's `HTMLRewriter` (scripts, styles, `<noscript>` skipped, one line per block element).

## Templates

Registered with the presence, drawn in every pane (`src/tools.ts`):

- `navigate`: card (title, link, excerpt, counts) with up to three thumbnails, each with a `Download n` button (keys 1 to 3, action `browser.download` with the image URL).
- `download`: card with the `asset://` handle as the `image` (or a file chip for non-images), a key/value list and `Copy handle` (c) / `Copy source URL` (u).
- `read`: one muted line. `images`: a table of alt and URL.

## The page

`src/server.ts` serves it on `127.0.0.1` with the daemon's address and a per-start token embedded (requests must be addressed to the server's own `127.0.0.1:PORT`/`localhost:PORT`; the JSON endpoints and the WebSocket need the token; the WebSocket checks the origin too). Address bar Enter calls `POST /api/navigate`, a tile's Download calls `POST /api/download`; page changes (also the agent's) arrive over `/ws`. The chat pane is `mountChat` on the daemon's current session (it follows `agent.focus`), input goes out as `session.input` from app `browser`, `asset://` images load from the daemon's `/assets` endpoint, and the pane gets a `focused` class (accent border) while the agent is in the browser.

Files: `main.ts` (CLI), `src/app.ts` (wiring and presence), `src/browser.ts` (navigate/download logic), `src/extract.ts`, `src/fetch.ts`, `src/tools.ts` (tools and templates), `src/server.ts`; `page/` is the browser code (`page.ts` entry, `app.ts` logic also run by the tests under happy-dom, `api.ts`).

## Tests

`bun test apps/browser`: extraction and fetch logic against a local fixture server, the whole app against a real in-process daemon (tools run through `ui.action` like button presses, assets checked byte for byte in the daemon's store), template resolution, and the page logic under happy-dom. The page bundle is built in a child process (`page/build-script.ts`) because `Bun.build` misbehaves in a process that already ran daemons.

## Rough edges

- No JavaScript, CSS or layout: pages are read as served, so script-rendered sites come out empty. Text is one line per block element, tables and `<pre>` lose their shape.
- No screenshot tool, back/forward, tabs, cookies or history; `data:` images and non-http(s) links are ignored.
- Fetches whatever URL it is given, local addresses included. It is a stand-in for your own machine, not a hardened browser.
- The chat in the page needs the daemon at page load; if it was down, browsing works and the chat says so (reload when it is up). A daemon restart is followed automatically.
- Not checked in a real browser (tests use happy-dom): layout, the page's Content-Security-Policy and the WebSocket from a real origin are only verified with `fetch`/`WebSocket` clients.
