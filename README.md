# suite

One agent session shared across every app in the suite. The design is in [docs/design.md](docs/design.md).

| Path                 | What                                                             | Phase |
| -------------------- | ---------------------------------------------------------------- | ----- |
| `protocol/`          | JSON Schemas (source of truth) + generated TS, Go, Rust types    | 0     |
| `daemon/`            | TS on Bun: session store, bus hub, turn queue                    | 1     |
| `harness/`           | TS on Bun: pi-based agent, core tools, system prompt             | 1     |
| `cli/`               | Go: `suite status / sessions / attach / tools / focus / move`    | 1-3   |
| `sdk-ts/`, `sdk-go/` | Bus clients (sessions, presence; TS: tool server helper)         | 2     |
| `sdk-rs/`            | Rust bus client for GPUI apps                                    | 3     |
| `apps/pad/`          | GPUI code editor stand-in (files, diagnostics, tools, chat)      | 3, 5  |
| `apps/browser/`      | Stand-in web browser: navigate/read/images/download tools + page | 5     |
| `apps/photo/`        | Stand-in photo editor (Go, terminal): image tools, preview, chat | 5     |
| `services/lsp/`      | Language servers as a connector (started by the daemon)          | 5     |
| `renderers/`         | web (TS), gpui (Rust), tui (Go)                                  | 4     |
| `fixtures/`          | View fixtures + renderer snapshot tests                          | 4     |

## Requirements

Bun ≥ 1.3, Go ≥ 1.27, Rust ≥ 1.85 (edition 2024; Pad needs GPUI's Linux build deps: Wayland/X11, xkbcommon, Vulkan).

## Running it

```sh
make build                 # bin/suite, bin/photo
bun run daemon             # starts the daemon in the foreground (Ctrl-C stops it)
bin/suite attach --new     # new session in the current directory; type to chat
bin/suite status           # daemon info + session counts + where the agent is
bin/suite tools [query]    # connectors (open or closed) and their tools
bin/suite sessions         # sessions by last activity
bin/suite attach [id]      # id or unique prefix (default: the agent's current session); draws the chat, then follows live (--raw: event dump)
bin/suite focus            # where the agent is: its session and app (or no app)
bin/suite move [app]       # move the agent to an open app; no app = home. --session ID also makes that session current
```

In `attach`: Ctrl-C cancels the running turn (exits when idle), Ctrl-D exits, `/<key>` presses a button on the latest result (`/help` lists them). `/clear` (in any pane: attach, Pad, the photo editor, the browser page) starts a new conversation in the same directory; every app switches to it and the agent stays where it is. Piped stdin sends one input per line and exits once the session is idle.

| Env                                       | Default                                                          |
| ----------------------------------------- | ---------------------------------------------------------------- |
| `SUITE_SOCKET`                            | `$XDG_RUNTIME_DIR/suite/bus.sock` (else `/tmp/suite-$UID/`)      |
| `SUITE_DATA_DIR`                          | `$XDG_DATA_HOME/suite` (else `~/.local/share/suite`)             |
| `SUITE_MODEL`                             | `deepseek/deepseek-flash` (`provider/model-id`)                  |
| `SUITE_IMAGES`                            | `1`/`0`: the model takes images (default: pi's registry)         |
| `SUITE_THINKING`                          | `off` (`minimal`…`xhigh`)                                        |
| `<PROVIDER>_API_KEY`, `SUITE_API_KEY`     | falls back to API-key entries in `~/.pi/agent/auth.json`         |
| `SUITE_AGENT`                             | `pi`; `echo` is an offline scripted agent for tests              |
| `SUITE_PORT`                              | 127.0.0.1 port for `/bus` + `/mcp` (default: any free port)      |
| `SUITE_GUESTS`                            | `claude,codex`; `none` turns guest agents off                    |
| `SUITE_CLAUDE_MODEL`, `SUITE_CODEX_MODEL` | the guest's model, e.g. `haiku`, `gpt-6-luna` (default: its own) |
| `SUITE_GUEST_PERMISSIONS`                 | `ask` (buttons in the chat) or `allow`                           |

`model` and `thinkingLevel` can also be set in `~/.config/suite/config.json`; env wins.

## Guest agents

Claude Code and Codex can work in a session as guests, through their ACP adapters (fetched by `bunx` on first use; they use your existing `claude` / `codex` logins). The agent hands them coding tasks with its `delegate` tool, or you address one directly in any pane: `/claude fix the failing test`, `/codex …`. Their messages and tool calls stream into the same chat, labelled; the agent hears what they did. When a guest asks before running something, the chat shows its options as buttons. Guests reach the same apps through the daemon's MCP gateway.

## Moving the agent between apps

The daemon owns where the agent is: one current session and the app whose pane shows it. The overlay is the home app. Try it with Pad (the code editor stand-in, app id `code-editor`; [apps/pad/README.md](apps/pad/README.md)):

```sh
bun run daemon                              # terminal 1
cargo run --release -p suite-pad            # terminal 2: a window; the agent is still home
bin/suite move code-editor                  # Pad's chat pane opens on the conversation
bin/suite attach                            # follow the same session from the terminal
bin/suite move                              # back home (the overlay, if it's running)
```

The agent can move itself with `apps_focus`, and Pad's buttons move it too. Closing the focused app sends the agent home.

## Connecting an app

Apps register with `app.presence` and bring their own MCP server; the daemon's gateway lists and calls its tools, and the agent finds them with `tools_search` / `tools_call`. Clients that can't use the Unix socket (Bun, browsers, MCP clients) read `daemon.json` next to the socket for the loopback URL and token. A complete example app: `bun run sdk-ts/examples/notes-app.ts`, then ask the agent to use the notes app. See [sdk-ts](sdk-ts/README.md) and [sdk-go](sdk-go/README.md).

## Cross-app work (the stand-ins)

The browser, photo editor and code editor are rough stand-ins for the real apps, enough to run the design's logo flow end to end. Downloads and edits become `asset://` handles in the daemon's asset store, which any app and the agent's bash/read tools can use; the lsp service (started with the daemon) gives diagnostics with or without an editor.

```sh
bun run daemon                         # terminal 1 (also starts the lsp service)
bun run apps/browser/main.ts           # terminal 2: prints a page URL with the chat in it
bin/photo                              # terminal 3: photo editor (make build), preview + chat
cargo run -p suite-pad                 # a window: code editor + chat
cd ~/site && bin/suite attach --new    # or chat from any pane
```

Then e.g. "download the logo from https://example.com, crop it to a square, put it into this project as public/logo.png and add it to src/Header.tsx". Details: [apps/browser](apps/browser/README.md), [apps/photo](apps/photo/README.md), [apps/pad](apps/pad/README.md), [services/lsp](services/lsp/README.md).

## The chat view

Every pane draws the same thing: the daemon turns session events into view items (markdown parsed, diffs computed, tool results filled into the app's template) and streams them as `view.snapshot` + `view.patch`. Three renderers draw them — `renderers/tui` (Go, used by `suite attach`), `renderers/web` (TS/DOM; try `bun run renderers/web/demo/serve.ts`) and `renderers/gpui` (Rust, used by Pad). Apps register templates for their tools in `app.presence.templates`; see [protocol/README.md](protocol/README.md) and [renderers/README.md](renderers/README.md). Every fixture in `fixtures/` is snapshot-tested by all three.

## Commands

```sh
make check      # install, codegen check, format check, lint, typecheck, test, build — everything
make codegen    # regenerate protocol types after editing protocol/schemas
make build      # builds bin/suite
```
