# Shared Agent Sessions Across the App Suite

One agent session that follows the user across every app (overlay, browser, file explorer, code editor, photo editor) instead of a separate chat per app. Apps are built with different toolkits (web, GPUI, TUI), so the chat is defined once as data and rendered natively by each toolkit.

## Goals

- One conversation history shared by all apps.
- Move the agent between apps; the target app opens its own chat UI with the same session.
- Apps can open and close at any time without breaking the session.
- Prompt caching stays intact: the cached prefix (tools + system prompt) never changes.
- The chat looks and behaves consistently in web, GPUI and terminal apps.
- Standard MCP throughout, so external harnesses can join as guests.

---

## 1. Overview

```
                ┌──────────────────────── daemon (always running) ────────────────────────┐
                │                                                                          │
 apps ◄─ bus ─► │  session store ── harness (pi) ── MCP gateway ── asset store ── services │
 (web/GPUI/TUI) │        │                               │                       (LSP…)    │
                │        └──── ACP bridge ── guests (Claude Code, Codex) ──────────┘         │
                └──────────────────────────────────────────────────────────────────────────┘
```

| Layer       | Role                                                                    |
| ----------- | ----------------------------------------------------------------------- |
| ACP         | Daemon ↔ agents (your harness and guests): sessions, prompts, streaming |
| MCP         | Agents ↔ connectors: tools, typed results                               |
| Bus         | Session sharing, focus, presence, file sync, UI actions                 |
| View format | How everything in the chat is described and rendered                    |

---

## 2. Repository structure

```
suite/
  protocol/         Source of truth: JSON Schemas for bus topics, events,
                    view format and theme tokens. Generates TS, Go and Rust types.
  daemon/           TS on Bun. Session store, bus hub, MCP gateway, asset store,
                    service manager, ACP bridge, view resolver.
  harness/          TS on Bun. pi-ai + pi-agent-core, core tools, static system
                    prompt, event injection, ACP-style interface.
  sdk-ts/           TS client: bus connection, presence, tool registration,
                    session attach.
  sdk-go/           Go client: same API, for TUI apps and the CLI.
  sdk-rs/           Rust client: same API, kept minimal, only for GPUI apps.
  renderers/
    web/            TS. Draws resolved components with DOM/CSS.
    gpui/           Rust. Draws resolved components with GPUI.
    tui/            Go. Draws resolved components in the terminal.
  cli/              Go. suite status / sessions / attach / logs / tools.
                    attach uses the TUI renderer.
  fixtures/         Example view JSON + resolved output + snapshot tests
                    for every renderer.

apps/ (existing)    overlay, browser, file explorer, code editor, photo editor
```

**Language split:** the daemon and harness are TypeScript on Bun because pi is TypeScript. The CLI, TUI renderer and Go SDK are Go. GPUI is a Rust-only framework, so GPUI apps need a Rust client and renderer; both stay thin because all view logic runs in the daemon (section 8). `protocol/` generates matching types for all three languages.

**Tooling:** Bun workspaces for the TS packages, a Go module (or `go.work`) for the Go side, a small Cargo workspace for the GPUI pieces.

---

## 3. Daemon

A long-running user service, started at login (launchd / systemd / Windows service). One process to start with; parts can be split out later if one needs isolation (LSP is the most likely).

| Part              | What it is                                                                                    |
| ----------------- | --------------------------------------------------------------------------------------------- |
| Session store     | SQLite event log with sequence numbers (`bun:sqlite`)                                         |
| Bus hub           | WebSocket over a Unix socket (or localhost plus an auth token)                                |
| View resolver     | Fills templates, applies patches, merges streaming text; sends resolved components to clients |
| MCP gateway       | Streamable HTTP endpoint; every connector's tools, tool search, errors for closed apps        |
| Asset store       | Folder of files named by content hash, served as `asset://` handles                           |
| Service manager   | Starts and supervises background services (LSP, indexes)                                      |
| ACP bridge        | Starts guest harnesses and connects them to sessions                                          |
| Template registry | View templates registered by apps for their result types                                      |

The overlay is **not** hosted in the daemon. It's an app like the others, with an always-on flag, so a UI crash never takes sessions down.

---

## 4. Harness

A general-purpose agent with a minimal core. Its own package, running in-process behind an ACP-style interface, so the daemon treats it the same as Claude Code or Codex. It can be tested standalone and used by other ACP clients.

**Built on pi (as dependencies, not a fork)**

- `@mariozechner/pi-ai`: model calls across providers
- `@mariozechner/pi-agent-core`: agent loop, tool calling, state
- `bash`/`read`/`write`/`edit` implementations from pi's coding agent if they import cleanly; otherwise rewritten (they're small)
- Check early: the session log must own the history, so pi-agent-core's state handling has to allow that.

**Fixed tool list** (full native definitions, never changes)

| Tool                                     | Purpose                                                  |
| ---------------------------------------- | -------------------------------------------------------- |
| `bash`                                   | Run shell commands                                       |
| `read`                                   | Read files                                               |
| `write`                                  | Write files                                              |
| `edit`                                   | Edit files                                               |
| `todo_write(todos)`                      | Keep a todo list (replaces the whole list)               |
| `tools_search(query, app?)`              | Find connector tools; returns full JSON schemas          |
| `tools_call(name, args)`                 | Run a connector tool (validated by the gateway)          |
| `apps_focus(app)`                        | Move the agent into an app                               |
| `delegate(agent, task, model?, effort?)` | Hand a task to a subagent: pi, Claude Code or Codex (§9) |

**Other responsibilities**

- Add connect, disconnect and focus events to each prompt.
- Publish `file.changed {path}` whenever a core tool changes a file.
- Resolve `asset://` handles to file paths.
- Act as an MCP client of the gateway; read `structuredContent` directly.

### System prompt (static, identical for every session)

```
You're an AI agent working across a suite of apps and services on the user's system.

Connectors (apps and services) announce themselves with <app connected=…/>
and <app disconnected=…/> events. Each event lists tool names only; use
tools_search to get a connector's tool schemas and tools_call to run them.

The most recent <focus> event says where the user is now.

When a connector disconnects, its tools stop working until it reconnects;
earlier schemas and results in this conversation stay valid as history.
If you need a disconnected app, ask the user to open it or use apps_launch.
After a reconnect, earlier schemas still apply unless the event shows a new version.

Tools that produce files or media return handles like asset://img-7f3.
Pass these handles to other tools to move results between apps.
```

---

## 5. Connectors

Everything beyond the core tools is a connector. Two kinds, one registration mechanism:

- **Apps** (have a UI, come and go): browser, photo editor, code editor, file explorer, overlay
- **Background services** (no UI, connected when the daemon starts): LSP, asset store, search indexes

Only `apps_focus` cares about the difference.

### MCP gateway

- Exposes every connector's tools all the time, so no agent's tool list ever changes.
- Forwards each call to the connected app or service.
- If the target isn't connected, returns a normal tool result:

```json
{
  "isError": true,
  "content": [
    {
      "type": "text",
      "text": "Photo editor isn't open. Ask the user to open it, or call apps_launch('photo-editor')."
    }
  ]
}
```

- Validates arguments; on a bad call returns `isError` with the schema so the model can correct itself.
- Tools declare `outputSchema` and return `structuredContent` next to the text.
- Optionally auto-launches an app, waits for it to register, then runs the call.

### Tool search

- The index updates live from `app.presence`.
- Matching on names + descriptions is enough to start with; add embeddings only if the tool count gets large.
- Schemas arrive as tool results, so they're appended to the history (no cache invalidation).

### Asset store

- Holds anything apps produce (images, downloads, exports).
- Handles stay valid after the producing app disconnects.
- Needed because the model can see an image but can't hand the pixels to another tool.

### LSP

- Language servers run as a background service, not inside the code editor.
- Diagnostics and definitions work with the editor closed; the editor attaches to the same servers.

---

## 6. Transcript

Events go at the start of the next user message. Mid-conversation system messages aren't portable across providers (Anthropic only accepts a system prompt at the start).

```
[system: static prompt above]

[user]
<app connected="lsp" kind="service" v="1" tools="diagnostics, definition, references, rename"/>
<app connected="photo-editor" kind="app" v="3" tools="crop, resize, layers, filters, export"/>
<app connected="browser" kind="app" v="2" tools="navigate, read, screenshot, download"/>
<focus app="browser" url="https://…"/>
Can you pull this image and edit it?

[assistant]
→ tools_search("download image")         ← schemas appended to history
→ tools_call("browser.download", …)      → asset://img-7f2
→ tools_call("photo.crop", { asset: "asset://img-7f2", … }) → asset://img-7f3

[user]
<app connected="code-editor" kind="app" v="1" tools="open, reveal, selection"/>
<app disconnected="photo-editor"/>
<focus app="code-editor" project="~/site"/>
Add that image as a logo to my project

[assistant]
→ bash: cp <resolved asset://img-7f3> ~/site/public/logo.png
→ edit: ~/site/src/Header.tsx
```

**Rules**

- The system prompt never contains session state, not even the apps open at startup.
- Events carry tool names only; schemas come from `tools_search`.
- Send `<focus>` only when focus changes.
- Log disconnects so later "app isn't open" errors make sense.
- Version connect events (`v=`); the model re-searches only when the version changes.

---

## 7. Bus

The session lives in the daemon; every chat UI is a view of it.

### Session

- Event log with sequence numbers. Late subscribers replay, then follow live events.
- Turns are queued one at a time; each input is tagged with the app it came from.

### Topics

| Topic            | Payload                                                                                | Purpose                                                                 |
| ---------------- | -------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| `session.events` | ACP-style `session/update` events + seq                                                | Stream to every view                                                    |
| `session.input`  | `{session, app, text}`                                                                 | Any pane can submit input                                               |
| `agent.focus`    | `{session, app}`                                                                       | Where the agent lives; matching app opens its pane                      |
| `app.presence`   | `{app, kind, title, open, context, mcp, launch}` (Phase 4 adds `templates`, `actions`) | Registration; the gateway reads tools and `v` from the app's MCP server |
| `tool.result`    | `{ref, type, data}`                                                                    | Typed results for views                                                 |
| `view.patch`     | `{ref, patch}`                                                                         | Live updates to a rendered view                                         |
| `ui.action`      | `{ref, action, args}`                                                                  | Button presses, routed to an app or the agent                           |
| `file.changed`   | `{path}`                                                                               | Open apps reload or show a diff                                         |

### Moving the agent between apps

1. User drags the agent onto another app, or the agent calls `apps_focus`.
2. Daemon publishes `agent.focus`.
3. The target app opens its chat pane, replays the history, then follows live events. A handoff mid-turn keeps streaming into the new pane.
4. The previous pane collapses, or stays open as a second view.
5. The next prompt carries the new `<focus>`.

If the focused app closes, focus falls back to the overlay. The session never closes.

**As built (Phase 3).** The daemon owns the agent's location: the _current session_ (persisted; created at startup if none exists) and the app whose pane hosts it. `agent.focus {session, app?, from?, reason}` goes to every client after `bus.welcome` and on every change; clients ask with `agent.move {app?, session?}` (no app = home). The overlay registers with `home: true`: it takes the agent when no app has it and gets it back when the agent's app closes. `apps_focus` moves the calling session's agent and makes that session current. `<focus>` means where the agent is (app plus that app's presence `context`); it's added at the start of the current session's turns when either changed.

---

## 8. View format

The whole chat is described as data and rendered natively by each toolkit. The goal is consistent structure and behavior, not pixel-identical output.

### Everything in the chat is a component

Session events map to standard components: user message, agent message, tool-call card, plan, diff, result view. Rich tool results use the same components.

### Components (fixed set; apps combine them, never add new ones)

| Component              | Web / GPUI        | TUI fallback (required)                                        |
| ---------------------- | ----------------- | -------------------------------------------------------------- |
| `card`, `row`, `stack` | Layout containers | Boxes / indentation                                            |
| `text` (markdown)      | Styled text       | Styled text                                                    |
| `image`                | Image             | Kitty/Sixel if supported, else `[image 1920×1080 · asset://…]` |
| `file`                 | File chip         | `📄 path`                                                      |
| `code`                 | Highlighted block | Highlighted block                                              |
| `diff`                 | Side-by-side      | Colored unified diff                                           |
| `table`                | Grid              | Box-drawn table, columns cut to fit                            |
| `kv`                   | Key–value list    | Aligned lines                                                  |
| `progress`             | Bar               | `▓▓▓░░ 60%`                                                    |
| `button`               | Button            | `[1] Label` + key binding                                      |
| `link`                 | Link              | Underlined text + URL                                          |

The text fallback also gives guests' plain-text results a consistent format.

### Templates

Data and view are separate. Each result type registers a template when its app announces itself; the daemon stores it. Results carry only data, so they render correctly in any pane, even after the producing app has closed.

```json
{
  "type": "card",
  "title": "Cropped image",
  "children": [
    {
      "type": "image",
      "src": "{{data.asset}}",
      "caption": "{{data.w}}×{{data.h}}"
    },
    {
      "type": "row",
      "children": [
        {
          "type": "button",
          "label": "Open as layer",
          "action": "photo.open_layer",
          "key": "o"
        },
        {
          "type": "button",
          "label": "Copy handle",
          "action": "copy:{{data.asset}}",
          "key": "c"
        }
      ]
    }
  ]
}
```

Results with no template fall back to a generic card built from ACP's base data (`kind`, text, diff, terminal output).

### Behavior

- **Actions send events, not code.** A button publishes `ui.action`; the daemon routes it to the right app, or to the agent as input if the app isn't open.
- **Keyboard first.** Every action has a key hint; focus moves through actions in a defined order.
- **Live updates.** Views are patched by `ref` via `view.patch`.

### As built (Phase 4)

- **Wire format.** `view.subscribe {session}` → `view.snapshot {seq, items}`, then `view.patch {seq, ops}` with ops `set` (replace or append an item by `ref`), `blocks` (replace the tail of a text component's blocks — streaming) and `remove`. Items have a `kind` (user, agent, thought, tool, notice), an optional `label` and tool `state`, and one component. `session.events` stays for raw consumers.
- **The daemon resolves more than templates.** Markdown is parsed into blocks and spans, and diffs into numbered lines, before anything is sent, so no renderer parses markdown or diffs text. Streaming text is re-parsed per chunk; only blocks from the first changed one are sent.
- **Templates** are registered in `app.presence.templates`, keyed by tool name, and kept after the app closes. A result's data is the tool's `structuredContent`. Template language: `{{path}}` in strings, whole-value substitution when the string is just `{{path}}`, `if`, and `each`/`as` (protocol/README.md). A template that doesn't resolve to valid components falls back to the generic card instead of failing the view.
- **Actions.** An `app.tool` action runs that tool through the gateway when the app is open; the result appears in the chat as a tool call rendered with the app's template. Otherwise the agent gets it as input ("The user pressed …"). `copy:` stays in the client.
- **Theme** comes from `~/.config/suite/theme.json` (or `SUITE_THEME`), else `protocol/themes/default.json`, and is sent after `bus.welcome`.
- Renderer conventions (item headers, tool state icons, per-component rules, keys) are in `renderers/README.md`; every fixture in `fixtures/` is snapshot-tested by every renderer.

### Theme tokens

Tokens describe meaning, not exact colors: `accent`, `muted`, `success`, `danger`; spacing `s`/`m`/`l`; text roles `title`/`body`/`code`. Web maps them to CSS variables, GPUI to its theme, TUIs to the closest 16-color, 256-color or true-color value. The daemon serves the active theme.

### Rendering pipeline

```
templates + data ─► daemon view resolver ─► resolved components (over the bus) ─┬─► web renderer  (TS)
                                                                                ├─► GPUI renderer (Rust)
                                                                                └─► TUI renderer  (Go)
```

The daemon's view resolver does the shared logic once: filling templates, applying `view.patch`, merging streaming text. Clients receive fully resolved component trees and only draw them. Local UI state (expanded/collapsed, scroll, focused action) stays in each client, since it's per-view and simple.

This keeps all three renderers thin without a shared library across three languages. Every component is added to `fixtures/` first (template + data + expected resolved output), with snapshot tests in each renderer (screenshots for web and GPUI, text output for the TUI), so the three can't drift apart.

---

### As built (Phase 5)

- **Asset store.** Content-addressed files under `<data>/assets` (`asset://img-<12 hex>`, `file-…` for non-images), so the same bytes always get the same handle; a JSON sidecar keeps the name, type and producing app. Bus: `asset.put {path|data}` / `asset.get {handle}` → `asset.info`; HTTP `/assets` for browsers. The agent's bash and read take handles as paths (bash commands get them substituted); write and edit refuse them.
- **`file.changed`** is derived from the diffs of the agent's write/edit results and passed on when an app sends it after saving (not echoed to the sender).
- **The lsp service** is a separate process the daemon starts (`kind: service`), driving gopls / typescript-language-server / rust-analyzer per project root; files are re-read before each request. Editors call it through the gateway like the agent does.
- **The working directory** is told to the model once per session (`<session cwd=…/>` in the first prompt), since the system prompt carries no session state.
- **Stand-ins**: browser (TS/web), photo editor (Go/terminal), code editor (Pad, GPUI). Go and Rust apps serve their tools with small hand-written MCP servers (`sdk-go/tools`, `sdk_rs::tools`), the TS one with the MCP SDK.

### The terminal app (after Phase 6)

- **`suite`** (no arguments) starts the daemon if it isn't running (`suite up`: detached, log in the data dir; `suite down` stops it; a daemon older than its code is restarted when no turn is running) and opens `suite-term`, a full-screen ratatui app (`apps/term`) for people who live in the terminal. The desktop overlay stays the home app; the terminal registers as an ordinary app (`terminal`, `terminal-2`… for more than one), so typing there pulls the agent in and other apps show the same conversation.
- **Layout:** the chat scrolls inside its pane (not the terminal's scrollback, which tmux breaks), drawn by `renderers/ratatui` in Claude Code's compact style (`●` tool lines with `⎿` summaries, Ctrl-O for full results); the editor; three status lines under it (directory, branch, harness settings, where the agent is; usage and turn state; the approvals mode, shift+tab to cycle ask, edits, auto); a sessions sidebar on the left (every session with its running state, title, last message and changed lines, live from `session.changed`; Ctrl-G to focus it); a side column with the plan and the files (git status plus what the agent and guests changed, Enter for a diff) when there is something to show.
- **Keyboard only:** prompts and choices are answered with arrows/digits/Enter, `/` opens a command menu (daemon commands are sent as input; /sessions, /help and the like are local), Ctrl-S switches sessions, Esc cancels a turn.

## 9. Guest agents

Claude Code, Codex and other ACP harnesses can join for heavy coding.

- Started by the ACP bridge as a sub-session (from the code editor, or by the main agent via a `delegate` tool); output streams onto the same bus.
- Connect to the same MCP gateway and see the same tools.
- **System prompt: add to it, don't replace it.** Their built-in prompts explain their own tools and are what make them good at coding. Append a short block covering the gateway, `asset://` handles and the event tags (`--append-system-prompt` for the Claude Code CLI, the `claude_code` preset + `append` in the Agent SDK, Codex's own custom-instruction config).
- The daemon adds the same connect, disconnect and focus events to their prompts.
- Guests usually reduce tool results to text, so the gateway publishes `tool.result {ref, …}` on the bus and returns text with the ref (`"Cropped → [result:a1f3]"`). Panes match the ref to the stored result and render it with its template; with no match they show the text.

### As built (Phase 6)

- **The bridge** is part of the daemon (`daemon/src/guests/`): a small hand-written ACP client (newline-delimited JSON-RPC over stdio) driving Zed's adapters, `@agentclientprotocol/claude-agent-acp` and `@agentclientprotocol/codex-acp`, started with `bunx` at pinned versions (not repo dependencies). Claude Code's adapter uses the user's own `claude` when it's on PATH. One guest process and ACP session per suite session and guest, kept while the daemon runs; follow-up tasks continue its conversation.
- **Two ways in.** The session agent's `delegate {agent, task, model?, effort?}` core tool (hidden from and refused for subagents) runs the guest inside its own tool call and returns its last message. Starting one is the user's call: every delegation first asks (`permission_request` from the session's harness: start it, allow it for the rest of the session, use another harness instead, or reject) unless the approvals mode is auto; the question names the harness, model and effort that will run, e.g. "Hand a task to Claude Code (model sonnet, effort high): …" (the call's own model and effort win over the user's settings, which fill the rest). "Allow for this session" holds for exactly that harness, model and effort. The user types `/claude task` or `/codex task` in any pane: the guest runs the turn instead of the agent, and a `guest_turn {agent, task, result}` event tells the agent with its next prompt (`<guest agent=… task=… stop=…>reply</guest>`).
- **Same session.** Guest messages, thoughts and tool calls are appended to the session log with `agent: "claude"` (tool call ids prefixed `claude:`); views label them. Diffs in guest results announce `file.changed` like the agent's.
- **Appended prompt.** Both adapters accept the same `_meta.jetbrains.air.customInstructions` on `session/new`: appended to Claude Code's preset, Codex's developer instructions. It explains the gateway, the event tags and `asset://` handles.
- **Same context.** Each guest keeps its own "told" state: its prompts start with the `<app …/>` and `<focus>` changes since its last prompt (the full list on the first).
- **Results.** The gateway knows an ACP client by `x-suite-text` (guests and the session's own ACP harness) and gives it text only (Claude Code shows `structuredContent` instead of the text when both are present): connector results get `[result:ref]` plus the file behind each `asset://` handle, and the structured result is stored as a `tool_result {ref, tool, rawOutput}` session event. The daemon's view (not the panes: they get rendered items) matches the ref in a guest's tool call text and draws the result with the app's template. `x-suite-guest` is the other meaning: a subagent, which may not delegate (guests send both headers, a pi subagent only that one). Adapters differ: Claude Code streams a call's input in later updates (so `tool_call_update` gained `title`, `tool`, `rawInput`), Codex puts MCP results in `rawOutput`.
- **Permissions.** A guest's `session/request_permission` becomes `permission_request {id, agent, title, kind?, content?, options}` (the tool call's kind and its diff or text content, so the card previews what it would change); panes draw one button per option (action `suite.permit {id, option}`, handled by the daemon); `permission_resolved {id, option?, by?}` answers it, with no option when the turn was cancelled. `permissions: "allow"` (config) picks the allow option without asking, and so does the session's approvals mode (below).
- **The user decides model, effort, mode and resuming.** Nothing is chosen for them: a harness keeps its own defaults (and its own assumptions, like Claude Code's effort for Sonnet) until the user picks. `/model`, `/effort`, `/mode [harness] [value]` (any pane; the daemon answers, not the agent) set it for the session, logged as `setting_changed`; the last choice per harness is the default for new sessions. Without a value they show the harness's own options as a `choice` (buttons, `suite.choose`). The options come from the harness: ACP config options for guests (categories `model`, `thought_level`, `mode`, set with `session/set_config_option`), and the same shape from pi (`configOptions()` / `setConfigOption()` on `AgentSession`: its provider's models and thinking levels). `/resume [guest]` lists the guests' stored conversations for the session's directory (`session/list`, including ones started outside the suite) and continues the picked one (`session/resume`); nothing resumes on its own.
- **Plans and usage.** Two session events feed status widgets, from the main agent and from guests alike; neither is a context event and pi's restore ignores both. `plan {agent?, entries: [{content, status, priority?}]}` is an agent's whole todo list (ACP's plan update; pi's `todo_write` core tool, emitted when the model makes the call): each one replaces that agent's previous list, and the view draws one checklist item per agent (`plan`, `plan-<agent>`), removed when the list is empty. `usage {agent?, contextUsed?, contextSize?, cost?, inputTokens?, outputTokens?}` carries an agent's latest numbers (consumers keep the newest per agent; the chat doesn't show it). pi sends it after each assistant message: `contextUsed` is that request's input + cache + output tokens, `contextSize` the model's window, and cost and token counts are summed over every message in the log, so they survive restarts. Guests send it for ACP `usage_update` (`used`, `size`, and `cost` only when the currency is USD). A plan update still ends the guest's current message; a usage update does not.
- **Approvals mode.** `/approvals [ask|edits|auto]` per session (no value: the next mode, ask → edits → auto → ask; shift+tab in the terminal app), `setting_changed {harness: "suite", option: "approvals"}`. A new session always starts in ask. The change is state for the panes' status line only: the view draws nothing for it. What each mode does:
  - **ask**: file edits are previewed before anything is written (the permission card shows the diff), and bash, starting a guest and guests' requests ask too.
  - **edits**: edits are written right away, so editors and the files panel show them at once; afterwards one Keep / Revert card (below) lets the user undo them. Everything that isn't an edit still asks (bash, starting a guest, a guest's non-edit requests). A guest's request counts as an edit when its tool call has kind `edit` or carries a diff.
  - **auto**: nothing asks.
    Config `permissions: "allow"` for a guest still allows everything in every mode.
- **The agent's own actions.** `NewSessionParams.requestPermission(request, signal) → Promise<boolean>` is the harness's hook for asking. pi's `bash`, `write` and `edit` call it before they act (`read`, `todo_write` and the gateway tools never do); without the hook (tests, other embedders) nothing changes. `write` and `edit` compute the new content first and ask with the diff (`path`, `oldText`, `newText`) before writing; `bash` asks with the command as the title and kind `execute`. A refusal makes the tool fail with "The user declined this edit." / "The user declined to run this command.", which the model reads as a tool error. The daemon builds the hook per session (`permissions.ts`) from the mode: auto allows, edits allows edits and asks for the rest, ask asks. The question is a `permission_request {agent, title, kind, toolCallId, content, options}` (Allow, Allow for this session, Reject); "for this session" is remembered per session and kind, in memory. The tool's abort signal drops the question when the turn is cancelled. `permission_request` gained `kind` (the ToolKind asked about) and `content` (diffs or text to preview, shown above the buttons; the answered item collapses to one line).
- **Review after the turn (edits mode).** `review.ts` tracks the edits applied without asking during a turn, from completed tool calls' diff content in the log (path, `oldText`, absent for a created file; `newText`), for every agent including guests and `/claude` direct turns. When `turn_ended` arrives with edits, one `choice` asks, titled like "Edited 3 files: src/a.ts, src/b.ts, notes.md", with Keep, Revert all and (for 2 to 10 files) "Revert <file>". Reverting restores each file to the `oldText` of its first edit in the turn (a created file is deleted), but only when the file still has the turn's last `newText`; otherwise the file is left alone and reported. Line endings and a BOM the file has are kept. A per-file revert asks again about the rest. The result is an `edits_reverted {paths, skipped?}` session event: a context event, so the agent's next prompt starts with `<reverted paths="a.ts, b.ts"/>` (a `skipped` attribute lists the left-alone files; nothing when nothing was restored), and the chat shows a notice ("reverted a.ts"). Open cards don't survive a daemon restart; the edits stay on disk.
- **The session's harness.** Each session runs on one harness: pi (the default until the user chooses), `claude` (Claude Code) or `codex` (Codex), switched with `/harness [name]` (no name: a `choice` titled "Harness (now: pi)"). Stored as `setting_changed {harness: "suite", option: "harness", value}`; the chat shows a notice ("harness: claude"), unlike the approvals mode. The last choice is the default for new sessions: a new session whose default isn't the daemon's own agent gets the event at creation, so it keeps that harness whatever is chosen elsewhere later. The valid names are the daemon's agent and the configured guests; an unknown stored one falls back to the agent. `/model`, `/effort` and `/mode` default to this harness. Each turn goes to the session's harness: pi through the in-process agent, an ACP harness through `GuestManager.agent(name)`, an `Agent` like pi whose session is an ACP conversation in "main" mode: its updates carry no `agent` label (messages, tool calls, plan, usage: the status widgets' main-agent slot gets them), tool call and message ids are prefixed `<name>-main` to stay unique next to a `/claude` guest of the same name (a separate process), and its permission requests take the guests' per-mode mapping above, so the edits-mode review card covers its edits too. It reaches the gateway with `x-suite-text` only, so it may delegate. The conversation's id is logged as `harness_message {harness: "claude", message: {acpSession}}`; after a daemon restart the harness is resumed with `session/resume`, or started anew (told the whole conversation, below) when that fails. Connectors and focus are told per harness process, from its own `ToldState`; the session's `guest_turn` and `edits_reverted` events come from the session's pending events. The direct `/claude task` turns still run a labelled guest whatever the session's harness is. Cancel, queueing and idle work as for pi.
- **Switching harness.** The harness that runs next never saw the turns another ran, so its prompt gets a `<conversation-so-far>` block before the pending events (`handoff.ts`): for each such turn the user's message and the agent's final message (tool calls left out; each text cut at 1500 characters, the whole at about 6000, older turns dropped first). A turn belongs to the harness chosen before its `user_message`; a harness is told the turns after its own last one (pi keeps its history in `harness_message` events, an ACP harness in its own conversation), and all turns when its conversation is new or lost. Turns the user gave a guest directly aren't part of it (the agent gets them as `guest_turn` events).
- **Subagents.** `delegate {agent}` takes `pi`, `claude` or `codex`, also the harness the session runs on (a fresh subagent; a `claude` guest is a separate process from the session's own Claude Code). `model` and `effort` are for that task only: they are set on the subagent's session with `session/set_config_option` by category (pi: `setConfigOption`), put back when it finishes, never logged as the user's settings, and the harness's refusal (an unknown model) is the tool result. A pi subagent (`guests/subagent.ts`) is a new session of the pi harness (the daemon's agent when that is pi, else `DaemonOptions.pi`), kept per session like a guest, reaching the gateway with `x-suite-guest` only; its updates are labelled `agent: "pi"` with prefixed ids, and its `harness_message` events are not logged (they aren't pi's history of the session). Its bash, write and edit ask by the approvals mode like pi's own.
- **Config.** `config.json` `guests: {claude: {title, command, permissions}, …}`; env `SUITE_GUESTS` (`none` or a list), `SUITE_GUEST_PERMISSIONS`.

---

## 10. Build plan

Each phase ends with something that works on its own.

| Phase             | Build                                                                                                   | Done when                                                                          |
| ----------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| 0. Protocol       | Schemas for bus topics, events, view format, tokens; TS + Go codegen (Rust added in Phase 3)            | Both sides compile against the same types                                          |
| 1. Core loop      | Daemon (session store, bus hub), harness with core tools only, Go CLI with a text-only `attach`         | You can chat with the agent from a terminal, and sessions survive a daemon restart |
| 2. Connectors     | MCP gateway, presence, tool search, closed-app errors; `sdk-ts` and `sdk-go`; overlay as the first app  | An app's tools appear in search and work from the overlay                          |
| 3. Handoff        | `agent.focus`, replay + live follow, event injection; `sdk-rs` and a second app (one GPUI app)          | One conversation moves between the overlay and another app — the core goal         |
| 4. View format    | Daemon view resolver, the three renderers, fixtures, templates, `ui.action`, `view.patch`, theme tokens | The same tool result looks consistent in web, GPUI and TUI                         |
| 5. Cross-app work | Asset store, LSP service, `file.changed` sync                                                           | The browser → photo editor → code editor logo flow works end to end                |
| 6. Guests         | ACP bridge, appended prompts, `[result:ref]` matching                                                   | A coding task handed to Claude Code streams into the same session                  |
| 7. Hardening      | See open questions                                                                                      | Safe to leave running all day                                                      |

---

## 11. Open questions

- **Approvals (built for bash, files and guests).** The permission flow exists: `permission_request` cards in every pane, three modes (§9 As built). What remains is connector tools (`tools_call` into apps), which the modes don't gate yet.
- **Long sessions.** A shared session grows fast. Decide how compaction works and who owns it (harness vs daemon), and how guests' histories relate to the main log.
- **Concurrent sessions.** Can two sessions be active at once, and how does focus work then?
- **Local security.** Auth on the bus socket and the MCP endpoint, and which apps may register tools. (Phase 1: the bus socket is mode 0600 in a 0700 directory, so only the user's own processes can connect. Phase 2 adds a 127.0.0.1 listener for `/bus` and `/mcp`, because Bun and browsers can't open WebSockets on Unix sockets and MCP clients need HTTP; it takes a random per-start bearer token published in `daemon.json` (0600) next to the socket. App MCP servers are expected to check a token too. Any local process of the user can still register any app name.)
- **pi-agent-core state.** Confirm the session log can own the history without forking.
- **pi on Bun.** Confirm pi-ai and pi-agent-core run cleanly under Bun (provider SDKs, streaming).
- **Tool names (resolved in Phase 1).** Anthropic and OpenAI only accept `[a-zA-Z0-9_-]` in tool names, so the core tools are `tools_search`, `tools_call`, `apps_focus` (and the system prompt says `apps_launch`). Connector tool names like `browser.download` are unaffected: they are arguments to `tools_call`, not tool definitions.
- **`apps_launch`.** The system prompt and the closed-app error tell the model to call `apps_launch`, but it isn't in the fixed tool list. Either add it as a core tool or make it a connector tool reached through `tools_call` (and reword the prompt), before Phase 2.
- **Tool list grows after Phase 1.** Phase 1 ships exactly four tools; Phase 2 adds `tools_search`/`tools_call`/`apps_focus`. Sessions created before that get one prompt-cache miss when the list changes. Harmless during development, but the "never changes" rule only holds from Phase 2 on.
- **What the log stores.** Phase 1 persists every streaming chunk as its own row (so a pane attaching mid-turn replays exactly what others saw), plus a `harness_message` event per complete model message, which is what the harness rebuilds its context from (provider fields like thinking signatures can't be rebuilt from chunks). That roughly doubles the text stored per turn. Coalescing chunks after a turn ends belongs with the compaction question above.
- **Crash mid-turn.** On restart the daemon appends `turn_ended {stopReason: "error"}` to any turn without one, and the harness answers interrupted tool calls with a synthetic error result. A bash command that was running is not re-run, and its side effects are unknown to the model beyond "interrupted".
- **Queued input visibility.** `user_message` is logged when its turn starts, not when it's queued, so other panes don't see a queued input until it runs. A `session.input_queued` event (or queue state in `SessionInfo` updates) may be wanted once several apps share a session.
- **Model credentials.** pi-ai's built-in model list lags new releases (the harness clones a known model of the same provider for unknown ids). API keys come from env or pi's `~/.pi/agent/auth.json`; OAuth logins (Codex, Claude subscriptions) stored there aren't used, because refreshing rotates the token underneath pi. Needs either our own login flow or a shared lock with pi.
- **Presence carries an MCP endpoint, not tool names (Phase 2).** Apps bring their own MCP server and announce its URL; the gateway lists the tools, caches the schemas (SQLite) so search and "isn't open" errors work while the app is closed, and owns `v`: it bumps it whenever the schemas or the server's `instructions` change. Apps never need to manage a version.
- **When connect/disconnect events are written.** They are computed per session at the start of each turn, as the difference between what is open now and what that session's log last said. Idle sessions get no writes, and an app that closes and reopens between two prompts produces nothing. The consequence: an app that opens mid-turn is only announced on the next prompt.
- **Connector instructions.** An MCP server's `instructions` are shown with that connector's tools in `tools_search` results (the overlay uses this for its ramble skill). They live in the history as tool results, so the cache is untouched, but they are repeated on every search that hits that connector.
- **Approvals are now urgent.** Without a permission flow the overlay confirms `click` and `type_text` with Return in its own UI before running them. Any other app with side effects has to do the same until the suite has approvals.
- **Which session do apps follow?** There is no "current session" yet: the overlay follows the most recently active one, and its reset creates a new session that other apps don't know to follow. Phase 3's `agent.focus {session, app}` needs a daemon-owned answer.
- **Missing for the overlay's UI.** Usage/context-size events, daemon settings (model, effort, cwd) and a list of models, and subagent parent ids on tool calls. All were shown by the overlay under the old companion daemon.
- **Daemon restarts are invisible to clients.** Clients resume subscriptions from the last seq they saw, which is correct as long as the log is append-only. Once compaction or log truncation exists, `bus.welcome` should carry a daemon/log instance id so clients can tell they need a full replay.
- **One current session.** Phase 3 answers "concurrent sessions" with one current session that every app follows; other sessions remain reachable from the CLI (`suite attach <id>`), get no `<focus>`, and an `apps_focus` call from one of them makes it current. Several agents active in different apps at once would need per-app sessions.
- **Focus is the agent's location, not the user's.** `<focus>` follows the agent (moved by the user or by itself). The user switching windows without moving the agent isn't reported; if the model should know where the user is looking, apps need to report window focus separately.
- **Pad is a stand-in.** No suite app is GPUI yet (Grove will be), so Phase 3's GPUI app is `apps/pad`. In Phase 5 it became the code editor stand-in (app id `code-editor`): a rough editor with diagnostics from the lsp service and `open`/`reveal`/`selection`/`files` tools, served by `suite_sdk::tools::ToolServer` (a small MCP server) instead of `rmcp`; to be replaced when Grove is ported.
- **Actions the agent doesn't see.** A button that runs an open app's tool directly shows its result in the chat, but the result is not in the agent's history (only in the session log). If the agent should know, actions need to reach the harness as context (like connector events) at the next prompt.
- **App-side live updates.** The design's `view.patch {ref, patch}` from apps (e.g. an export's progress) isn't built: apps can't address a result they returned, because MCP gives them no id for the call. Options: the gateway passes the tool call id in `_meta`, or apps return a progress token the daemon maps to the item.
- **Pixel snapshots.** Renderer snapshots are text (TUI), HTML (web) and a layout model (GPUI). Screenshot tests need a headless browser and offscreen GPUI rendering; not set up yet.
- **The view has no turn state.** Nothing in a view says whether the last agent message is still streaming, so `suite attach` also follows `session.events` to know when a turn runs and waits 150 ms after the last change before treating an item as settled. Giving agent and thought items `state: running` until `turn_ended` (one more `set` per turn) would make the view self-sufficient.
- **Emphasis without color.** With no color (`NO_COLOR`, pipes) the TUI drops bold, italic and inline code silently, so a highlighted phrase reads like plain text. Textual markers (`*…*`, backticks) in that mode are an option if it matters.
- **Templates can't slice lists (resolved).** `each` takes a `limit`, and `{{index}}` is the 1-based position; the browser's thumbnails use both.
- **Stand-ins fetch anything.** The browser stand-in fetches whatever URL the agent gives it, local addresses included, and the lsp service writes files on `rename`. Fine for testing; real apps need the approvals story (see above) before the agent can reach the local network or write outside a project.
- **The bash tool's file changes aren't announced.** `file.changed` comes from write/edit diffs (and apps saving); `cp asset://… ~/site/logo.png` through bash changes a file nobody hears about. Watching the session's cwd, or diffing mtimes around a bash call, would close the gap.
- **Guest sessions end with the daemon.** After a restart a delegation starts a fresh guest conversation; the user can `/resume` the old one.
- **Approvals cover the main agent's file edits and bash, not connector calls.** `bash`, `write` and `edit` are gated by the session's approvals mode (§9 As built), as are guests and starting a guest. `tools_call` into an app's tools isn't: apps with side effects still confirm in their own UI. Bash commands that change files aren't part of the edit review (see the bash note above). "Allow for this session" answers live in memory only (a restart asks again).
- **The session's harness (resolved).** The user picks it per session (`/harness`, §9 As built). Left open: an ACP harness resumed after a restart is told the connectors again (its told state is in memory), a switch back to a harness doesn't tell it about `guest_turn`s the other harness had already consumed, and the summary is the final messages only, not what the tools did.
- **Open questions don't survive a restart.** Pending choices and permission requests are answered with "none" when the daemon stops; their buttons then report that they're no longer open.
- **Guests' histories and the main log.** The main agent sees a guest's work only as the `delegate` result or a `guest_turn` summary, not its tool calls; the log has everything. Whether (and how) the main agent should see more is part of the compaction question.
- **Permission noise.** Claude Code in its default mode asks before every suite tool call (each `tools_search` and `tools_call`), Codex in its default mode reviews them itself. Pre-allowing the suite's MCP tools for Claude Code (its `allowedTools`), or a per-guest `mode`, would cut the prompts; the real answer belongs with approvals (Phase 7).
- **Pending events across a guest turn.** After a restart, the agent's pending connector events are recomputed from the last `user_message`; a `/claude` turn's message counts, so events from before it that the agent hadn't been told yet are lost (they reappear only if something changes again).
- **Presence is the only way to update context (resolved).** `app.context {app, context}` updates just the context: memory only, no store write, no `connectors.changed`. The SDKs keep it for reconnects (`setContext` / `SetContext` / `set_context`); Pad, the photo editor and the browser use it.
