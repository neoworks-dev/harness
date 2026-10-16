# apps/photo

A stand-in photo editor for the suite: a terminal app (Go, standard library image packages only) that registers as `photo-editor`, serves image tools to the agent over MCP and shows the current image next to the shared chat.

```sh
bun run daemon                      # the daemon must be running
go run ./apps/photo [image]         # or `make build` and bin/photo [image]
```

Without a terminal on stdin and stdout (or with `--headless`) it runs headless: presence and tools only, logging to stderr. That is also how `apps_launch` starts it (the launch command is the binary itself). `--socket` overrides the daemon socket.

## Tools

All tools work on the current image. Every edit stores its result as a new asset (`asset.put`), so handles handed out earlier stay valid, and returns `structuredContent` `{handle, name, width, height, mime}`.

| Tool        | Arguments                                              | Notes                                                    |
| ----------- | ------------------------------------------------------ | -------------------------------------------------------- |
| `open`      | `asset` (an `asset://` handle) or `path`               | PNG, JPEG, GIF (first frame). Starts a new undo history. |
| `info`      |                                                        | The current image.                                       |
| `crop`      | `x`, `y`, `width`, `height`                            | Pixels from the top-left; must lie inside the image.     |
| `resize`    | `width?`, `height?`                                    | One of them keeps the aspect ratio.                      |
| `rotate`    | `degrees`: 90, 180 or 270                              | Clockwise.                                               |
| `flip`      | `direction`: `horizontal` or `vertical`                |                                                          |
| `grayscale` |                                                        |                                                          |
| `export`    | `format`: `png` or `jpeg`, `quality?` (1-100, def. 90) | New asset; the current image stays.                      |
| `undo`      |                                                        | Back to the previous image, same handle as before.       |

Edits are written as PNG. WebP is not supported (it needs `golang.org/x/image`). Images are limited to 20000 px a side and 50 megapixels; undo keeps the last 20 states.

Each tool has a view template: a card with the image (`src` is the handle), its size, type and handle, and buttons: Undo `[u]`, Grayscale `[g]`, Rotate 90° `[r]` (these run the tool, from `suite attach` or this app) and Copy handle `[c]`.

The presence context is `{image: <name>, asset: <handle>}` and follows the open image, so the agent's `<focus>` says which image is open.

## The terminal UI

```
 Photo editor · logo.png · 640×480 · asset://img-… · agent is here   (status)
          ▀▀▀▀▀▀▀▀                                                   (preview, half blocks)
── notes ─────────────────────────────────────────────────────────
 … the shared chat, last lines of it …
 > input
```

The preview uses `▀` cells with the upper pixel as foreground and the lower as background, in 24-bit color (`COLORTERM=truecolor`), else 256 colors, else a gray character ramp. Images are shrunk to fit, never enlarged.

The chat follows the agent's current session (`agent.focus` + `view.subscribe`), drawn by `renderers/tui`. Typing sends `session.input` from app `photo-editor`; `//text` sends `/text`. Keys and commands:

- `/<key>` presses a button of the latest chat item that has any (`/help` lists them), like in `suite attach`; `copy:` buttons use the OSC 52 clipboard escape.
- `/open <path>` opens a local image.
- `q` + Enter, `/q`, Ctrl-C, or Ctrl-D on an empty line quits and unregisters.

The status line says `agent is here` when the agent was moved to this app (`bin/suite move photo-editor`).

## Tests

`go test ./...` in this directory: image operations on generated images, preview snapshots, the tools, the UI against the fake daemon with a fake terminal, and a headless run against the real daemon (needs `bun`; skipped without it) that calls `photo-editor.crop` through the gateway and checks the new asset and the rendered template.
