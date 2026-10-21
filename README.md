# suite

One agent session shared across every app in the suite. The design is in [docs/design.md](docs/design.md).

| Path                 | What                                                              | Phase |
| -------------------- | ----------------------------------------------------------------- | ----- |
| `protocol/`          | JSON Schemas (source of truth) + generated TS, Go, Rust types     | 0     |
| `daemon/`            | TS on Bun: session store, bus hub, turn queue                     | 1     |
| `harness/`           | TS on Bun: neo, our agent (loop, core tools, system prompt)       | 1     |
| `websearch/`         | Web, news and image search across engines; pages as markdown      | 6     |
| `cli/`               | Go: `suite status / sessions / attach / tools / focus / move`     | 1-3   |
| `sdk-ts/`, `sdk-go/` | Bus clients (sessions, presence; TS tool server: `sdk-ts-tools/`) | 2     |
| `sdk-rs/`            | Rust bus client for GPUI apps                                     | 3     |
| `apps/term/`         | `suite-term`: full-screen terminal app (chat, plan, files, usage) | 6     |
| `apps/gui/`          | `suite-gui`: desktop app (Svelte on svelte-gpui, native window)   | 6     |
| `crates/github/`     | Read-only GitHub (gh allowlist) for the term and gui apps         | 6     |
| `apps/pad/`          | GPUI code editor stand-in (files, diagnostics, tools, chat)       | 3, 5  |
| `apps/browser/`      | Stand-in web browser: navigate/read/images/download tools + page  | 5     |
| `apps/photo/`        | Stand-in photo editor (Go, terminal): image tools, preview, chat  | 5     |
| `services/lsp/`      | Language servers as a connector (started by the daemon)           | 5     |
| `renderers/`         | web (TS), gpui (Rust), tui (Go), ratatui (Rust)                   | 4     |
| `fixtures/`          | View fixtures + renderer snapshot tests                           | 4     |

## Requirements

Bun ≥ 1.3, Go ≥ 1.27, Rust ≥ 1.85 (edition 2024; Pad needs GPUI's Linux build deps: Wayland/X11, xkbcommon, Vulkan).

The UI language, [neoui](docs/neoui.md), is a package next to this repo: a fresh clone needs `cd ../neoui && bun link` before `bun install`.

## Running it

```sh
make build                 # bin/suite, bin/photo, bin/suite-term, bin/suite-gui
bin/suite                  # in a terminal: starts the daemon if needed, then opens the terminal app (suite-term)
bin/suite gui              # the desktop app (apps/gui) in its own window, terminal stays free: [--session ID | --new]
bin/suite up               # make sure the daemon runs; starts it in the background (log: <data dir>/daemon.log)
bin/suite down             # stop the daemon (SIGTERM; waits for the socket to close)
bun run daemon             # alternatively: the daemon in the foreground (Ctrl-C stops it)
# suite and suite up restart a daemon whose code (daemon/, harness/, protocol/) changed since it
# started, unless a turn is running (then they say so and keep it).
bin/suite attach --new     # new session in the current directory; type to chat
bin/suite status           # daemon info + session counts + where the agent is
bin/suite tools [query]    # connectors (open or closed) and their tools
bin/suite sessions         # sessions by last activity
bin/suite attach [id]      # id or unique prefix (default: the agent's current session); draws the chat, then follows live (--raw: event dump)
bin/suite focus            # where the agent is: its session and app (or no app)
bin/suite move [app]       # move the agent to an open app; no app = home. --session ID also makes that session current
```

`suite` opens the terminal app ([apps/term](apps/term/README.md)): the session's chat in a full-screen pane next to a prompt editor, with the git branch, model settings, turn state and usage under it, and plan / changed files at the side when there are any. It shows the agent's session if it works in the current directory (else the latest one here, else a new one), `--new` and `--session ID` choose another, and the agent moves into it. Closing it never stops work. Keys: Enter sends, Alt-Enter newline, Up/Down history, PgUp/PgDn, mouse wheel or Shift-Up/Down scroll, Ctrl-O full tool results, Ctrl-T thoughts, Esc closes / cancels the turn, Ctrl-C twice quits, Shift-Tab approvals ask/review/edits/auto, Tab on a prompt adds a note, Ctrl-S sessions, `/` for commands (`/help` lists everything).

In `attach`: Ctrl-C cancels the running turn (exits when idle), Ctrl-D exits, `/<key>` presses a button on the latest result (`/help` lists them). `/clear` (in any pane: attach, Pad, the photo editor, the browser page) starts a new conversation in the same directory; every app switches to it and the agent stays where it is. Piped stdin sends one input per line and exits once the session is idle.

| Env                                   | Default                                                     |
| ------------------------------------- | ----------------------------------------------------------- |
| `SUITE_SOCKET`                        | `$XDG_RUNTIME_DIR/suite/bus.sock` (else `/tmp/suite-$UID/`) |
| `SUITE_DATA_DIR`                      | `$XDG_DATA_HOME/suite` (else `~/.local/share/suite`)        |
| `SUITE_DAEMON_CMD`                    | `bun run <repo>/daemon/src/main.ts` (what `suite up` runs)  |
| `SUITE_ROOT`                          | repo found above the `suite` binary (for `suite up`)        |
| `SUITE_TERM`                          | `suite-term` next to `suite`, else on `PATH`                |
| `SUITE_GUI`                           | `suite-gui` next to `suite`, else on `PATH`                 |
| `SUITE_MODEL`                         | `deepseek/deepseek-flash` (`provider/model-id`)             |
| `SUITE_IMAGES`                        | `1`/`0`: the model takes images (default: pi's registry)    |
| `SUITE_THINKING`                      | `off` (`minimal`…`xhigh`)                                   |
| `SUITE_SAFE_SEARCH`                   | `moderate` (`strict`, `off`) until `/safesearch` picks one  |
| `SUITE_SEARXNG_URL`                   | a SearXNG instance neo's web search asks too                |
| `TINYFISH_API_KEY`                    | TinyFish's search API, asked too (or `/login tinyfish`)     |
| `<PROVIDER>_API_KEY`, `SUITE_API_KEY` | else `SUITE_AUTH`, then pi's `~/.pi/agent/auth.json`        |
| `SUITE_AUTH`                          | `auth.json` in `SUITE_DATA_DIR`: what `/login` writes       |
| `SUITE_AGENT`                         | `neo`; `echo` is an offline scripted agent for tests        |
| `SUITE_PORT`                          | 127.0.0.1 port for `/bus` + `/mcp` (default: any free port) |
| `SUITE_GUESTS`                        | `claude,codex`; `none` turns guest agents off               |
| `SUITE_GUEST_PERMISSIONS`             | `ask` (buttons in the chat) or `allow`                      |
| `SUITE_TITLES`                        | `off`: no model writes session titles (`/title` still does) |

`model`, `thinkingLevel` and `web` (`{"safeSearch": "strict", "searxng": "http://…"}`) can also be set in `~/.config/suite/config.json`; env wins.

neo searches the web itself: `web_search` asks several engines at once (web, news or images; no key needed) and lists what they found, and `web_fetch` saves a page as markdown in a file that the agent greps and reads in parts. You're asked before each fetch unless approvals is `auto`. Safe search is your setting, not the agent's: `/safesearch strict|moderate|off` sets it for every session (no value: a card to pick from). TinyFish's free search API is asked too once you add its key with `/login tinyfish`. The search engines are in [websearch](websearch/README.md).

Skills are instructions neo loads when the work calls for them: a folder with a `SKILL.md` (frontmatter `name` and `description`, then the text) in `~/.config/suite/skills/` or in a project's `.suite/skills/`. neo sees each skill's description and loads the text with its `skill` tool.

UI in the chat is not a skill: every agent (neo, Claude Code, Codex) is told it can write a ```neoui block, and the gateway's `ui_library` tool teaches the language, the library and the components the connected apps add.

Keys and accounts: `/login` adds an API key or an account (ChatGPT, GitHub Copilot, Claude Pro/Max, which neo only may use), `/logout` removes one, `/accounts` lists them. Keys are typed into a hidden field, never into the chat.

## Session titles

A session is named after the start of its first message at once, and then a model writes a short title for it (3 to 8 words: what the session is about and what it should achieve, not a cut-off copy of the message). `/title` writes it again from the conversation so far (the GUI's session menu has it as "Regenerate title"); it also replaces a title you gave the session yourself, which nothing else does.

Which model writes titles is your choice, for every session: `/title-model <harness> <model>` (`/title-model claude haiku`; a harness alone uses its own default model; no arguments pick from a list; `/title-model default` goes back). Until you choose, the suite asks Claude Code's `haiku` when that guest is configured, else the daemon's own agent on the model it is configured with. Each title is one call outside the session, logged nowhere and with no tools. A model that cannot be reached leaves the start of the message as the title, and `/title` says why. Claude Code and Codex write theirs in a conversation of their own in a temporary directory, which they may keep in their own history. `SUITE_TITLES=off` stops the automatic title.

## Guest agents

Claude Code and Codex can work in a session as the session's harness or as guests, through their ACP adapters (fetched by `bunx` on first use; they use your existing `claude` / `codex` logins). The agent hands them coding tasks with its `delegate` tool, or you address one directly in any pane: `/claude fix the failing test`, `/codex …`. Their messages and tool calls stream into the same chat, labelled; the agent hears what they did. When the agent wants to start a guest, or a guest asks before running something, the chat shows the options as buttons. A delegation shows as one tool item (`neo · first line of the task`, running, then done or failed, with the subagent's reply as its result); its thoughts and tool calls are in the log and can be opened per agent (in the terminal app: the agents list under the prompt). Guests reach the same apps through the daemon's MCP gateway.

How much asks first is the session's approvals mode: `/approvals ask|review|edits|auto` (no value: the next one; Shift-Tab in the terminal app). In `ask` (the default for every new session) the agent's file edits show their diff before anything is written, and its commands, starting a guest and guests' requests ask too. In `review` each edit is written right away and a card with its diff asks Keep or Revert for that one edit (the agent is told when you revert); everything else still asks. In `edits` edits are written right away and, when the turn ends, a card offers Keep, Revert all or reverting single files (the agent is told what you reverted); everything else still asks. In `auto` nothing asks. Reading files never asks, in any mode, except sensitive ones (`.env` files, private keys, `~/.ssh`, credential and token files), which always ask, as does a command that mentions one. An answer can carry a note for the agent (in the terminal app, Tab on a prompt: "use tabs instead").

Each session runs on one harness that you pick: neo (the default), Claude Code or Codex. `/harness claude` switches (no name: a list; your last choice is the default for new sessions), and the next turn goes to the new one, which first gets a short summary of what was said while another harness ran. The agent can hand work to any harness, also its own, with `delegate`, optionally naming the model and effort; you are asked first (unless approvals is auto) with one card, "Start a subagent?": the harness, model, effort and directory that will run, highlighted, and the task. A model or effort the harness lists and doesn't know is refused before anything is asked, with the valid names. `delegate` can also name a `cwd` or a git `worktree` (created as `.worktrees/<name>` on a branch of that name): the subagent then runs as a session of its own in that directory, which the terminal app's sidebar groups under its worktree.

You choose the models: `/model` (the session's harness) or `/model claude` shows what that harness offers (for neo: the models of every provider you are logged in to or have a key for, as `provider/model-id`; OpenRouter's list is its live one), `/model claude sonnet` sets it; `/effort` and `/mode` work the same way. Choices hold for the session and become the default for new ones; until you choose, each harness uses its own defaults. After you pick an OpenRouter model for neo, a second card asks which host runs it (DeepInfra fp8, say, with price and context); "Any" lets OpenRouter choose. You can also type the host: `/model openrouter/deepseek/deepseek-v4.1-flash-1017@deepinfra/fp8`. `/resume` lists earlier Claude Code / Codex conversations in the session's directory (also ones started outside the suite) and continues the one you pick.

Claude Code and Codex use their own logins. To run one on an API key from `/login`, pick a model of that provider: `/model codex openrouter/openai/gpt-5-mini` (OpenAI, OpenRouter, Groq, xAI, Fireworks and Vercel AI Gateway work, as Codex needs the Responses API), `/model claude deepseek/deepseek-v4-pro` (Anthropic, DeepSeek, OpenRouter, Moonshot, Z.ai and MiniMax work). `/model codex` and `/model claude` list them. A model without a provider goes back to the harness's own login. A Claude Pro/Max login is for neo only.

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

Apps register with `app.presence` and bring their own MCP server; the daemon's gateway lists and calls its tools, and the agent finds them with `tools_search` / `tools_call`. Clients that can't use the Unix socket (Bun, browsers, MCP clients) read `daemon.json` next to the socket for the loopback URL and token. A complete example app: `bun run sdk-ts/examples/notes-app.ts`, then ask the agent to use the notes app. See [sdk-ts](sdk-ts/README.md) (tool hosting: `sdk-ts-tools`) and [sdk-go](sdk-go/README.md).

## Cross-app work (the stand-ins)

The browser, photo editor and code editor are rough stand-ins for the real apps, enough to run the design's logo flow end to end. Downloads and edits become `asset://` handles in the daemon's asset store, which any app and the agent's bash/read tools can use; the lsp service (started with the daemon) gives diagnostics with or without an editor.

```sh
bun run daemon                         # terminal 1 (also starts the lsp service)
bun run apps/browser/main.ts           # terminal 2: prints a page URL with the chat in it
bin/photo                              # terminal 3: photo editor (make build), preview + chat
cargo run -p suite-pad                 # a window: code editor + chat
cd ~/site && bin/suite attach --new    # or chat from any pane
```

Then e.g. "download the logo from <https://example.com>, crop it to a square, put it into this project as public/logo.png and add it to src/Header.tsx". Details: [apps/browser](apps/browser/README.md), [apps/photo](apps/photo/README.md), [apps/pad](apps/pad/README.md), [services/lsp](services/lsp/README.md).

## The chat view

Every pane draws the same thing: the daemon turns session events into view items (markdown parsed, diffs computed, tool results filled into the app's template) and streams them as `view.snapshot` + `view.patch`. Three renderers draw them — `renderers/tui` (Go, used by `suite attach`), `renderers/web` (TS/DOM; try `bun run renderers/web/demo/serve.ts`) and `renderers/gpui` (Rust, used by Pad). Apps register templates for their tools in `app.presence.templates`; see [protocol/README.md](protocol/README.md) and [renderers/README.md](renderers/README.md). Every fixture in `fixtures/` is snapshot-tested by all three.

## Commands

```sh
make check      # install, codegen check, format check, lint, typecheck, test, build — everything
make codegen    # regenerate protocol types after editing protocol/schemas
make build      # builds bin/suite, bin/photo, bin/suite-term and bin/suite-gui
```
