# neo: Shared Agent Sessions Across Apps

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
 apps ◄─ bus ─► │  session store ── harness (neo) ── MCP gateway ── asset store ── services │
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
neo/
  protocol/         Source of truth: JSON Schemas for bus topics, events,
                    view format and theme tokens. Generates TS, Go and Rust types.
  daemon/           TS on Bun. Session store, bus hub, MCP gateway, asset store,
                    service manager, ACP bridge, view resolver.
  neo/              TS on Bun. pi-ai, our agent loop, core tools, static system
                    prompt, event injection, ACP-style interface.
  sdk-ts/           TS client: bus connection, presence, tool registration,
                    session attach.
  sdk-go/           Go client: same API, for TUI apps and the CLI.
  sdk-rs/           Rust client: same API, kept minimal, only for GPUI apps.
  renderers/
    web/            TS. Draws resolved components with DOM/CSS.
    gpui/           Rust. Draws resolved components with GPUI.
    tui/            Go. Draws resolved components in the terminal.
  cli/              Go. neo status / sessions / attach / logs / tools.
                    attach uses the TUI renderer.
  fixtures/         Example view JSON + resolved output + snapshot tests
                    for every renderer.

apps/ (existing)    overlay, browser, file explorer, code editor, photo editor
```

**Language split:** the daemon and harness are TypeScript on Bun because pi-ai is TypeScript. The CLI, TUI renderer and Go SDK are Go. GPUI is a Rust-only framework, so GPUI apps need a Rust client and renderer; both stay thin because all view logic runs in the daemon (section 8). `protocol/` generates matching types for all three languages.

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

**Built on pi**

- `@mariozechner/pi-ai`: model calls across providers (a dependency)
- `neo/src/loop/`: agent loop, tool calling, state. Copied from `@mariozechner/pi-agent-core` 0.73.1 (MIT, see its LICENSE) so we can change it; no longer a dependency.
- `bash`/`read`/`write`/`edit` implementations from pi's coding agent if they import cleanly; otherwise rewritten (they're small)
- Check early: the session log must own the history, so the loop's state handling has to allow that.

**Fixed tool list** (full native definitions, never changes)

| Tool                                                                   | Purpose                                                                                                                                       |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `bash`                                                                 | Run shell commands                                                                                                                            |
| `read`                                                                 | Read files; each text line tagged `12:k3\|…` (number and a two-character hash, after hashline)                                                |
| `write`                                                                | Write files                                                                                                                                   |
| `edit`                                                                 | Edit files by line tags from read: replace lines `start`..`end` or insert `after` a line (see below)                                          |
| `todo_write(todos)`                                                    | Keep a todo list (replaces the whole list)                                                                                                    |
| `skill(name)`                                                          | Load a skill: instructions for a kind of work, listed in the system prompt (below)                                                            |
| `web_search(query, kind?, since?, limit?)`                             | Search the web, news or images with several engines at once; the results as they are (`@neoworks/websearch`, its own repository)              |
| `web_fetch(url)`                                                       | Save a page as markdown in a file; returns its path and outline (short pages whole)                                                           |
| `tools_search(query, app?)`                                            | Find connector tools; returns full JSON schemas                                                                                               |
| `tools_call(name, args)`                                               | Run a connector tool (validated by the gateway)                                                                                               |
| `ui_library()`                                                         | How to show UI: the neoui guidance, language and library, and the components the connected apps add (`docs/neoui.md`)                         |
| `apps_focus(app)`                                                      | Move the agent into an app                                                                                                                    |
| `delegate(agent, task, model?, effort?, cwd?, worktree?, background?)` | Hand a task to a subagent: neo, Claude Code or Codex, optionally in another directory or git worktree, optionally without waiting for it (§9) |
| `send_message(to, text)`                                               | Write to a running subagent, a peer, or `main`, the session's agent (§9)                                                                      |
| `wait(for?, timeoutSeconds?)`                                          | Block until a message arrives or a named subagent ends; returns what came in (§9)                                                             |

**Line tags** (`neo/src/tools/hashline.ts`, after [hashline](https://github.com/kebbbnnn/hashline)). `read` prefixes each line with `N:hh|`: its number and two characters (base 36) of the md5 of the line without its outer whitespace, so indentation changes keep the tag. `edit({path, edits: [{start, end?, newText} | {after, newText}]})` names lines by those tags instead of repeating their text; `after: "0"` inserts at the top, an empty `newText` with `start` deletes. All tags in a call refer to the file before it, edits may not overlap, and they are applied bottom-up. The number says where and the hash checks the line is still what was read. Two characters are 1296 values, so hashes repeat in a file: a line that no longer matches is only followed when its hash appears exactly once within 30 lines of the number (the result says where it moved; a range's two ends must have moved by the same amount); otherwise the edit fails with the current lines there, tagged, so the model can retry without reading again. The result shows the changed regions with two lines of context and their new tags. The permission request and the details still carry the whole old and new text, so diffs, review and revert are unchanged.

**Other responsibilities**

- Add connect, disconnect and focus events to each prompt.
- Publish `file.changed {path}` whenever a core tool changes a file.
- Resolve `asset://` handles to file paths.
- Act as an MCP client of the gateway; read `structuredContent` directly.

### Skills

A skill is a directory with a `SKILL.md`: frontmatter `name` (default: the directory's) and `description` (one line, what the work is and when to load it; a skill without one is skipped), then the instructions. neo finds them when a session starts, in three places, a later one replacing an earlier one of the same name:

1. `neo/skills/*/SKILL.md`, shipped with neo (none right now: neoui is not a skill but the gateway's `ui_library`, which every harness has);
2. `$XDG_CONFIG_HOME/neo/skills/*/SKILL.md` (`~/.config/neo/skills/…`), the user's;
3. `<session cwd>/.neo/skills/*/SKILL.md`, the project's.

The skills of enabled plugins (below) come first, so these three replace them by name; two plugins with the same skill name are both written `plugin:name`. The system prompt lists each skill as `- name: description`, with a line telling the model to load one before work it covers. The `skill {name}` core tool returns the text (the chat shows `N lines`, not the text); an unknown name is a tool error that lists the names. It reads only what the session found, so it never asks permission; it is in the tool list of every session, with skills or without. The list is read once per session, so a new skill shows from the next session on, and the prompt (and its cache) changes only when the skills do. `HarnessConfig.skills` holds the two fixed directories (none in tests that don't set it).

### Plugins

neo loads Claude Code plugins, installed by the harness itself so it doesn't depend on Claude Code (it never reads `~/.claude/plugins`). Guests (Claude Code, Codex) load their own.

```
~/.config/neo/plugins/
  installed.json         { version: 1, plugins: { "name@marketplace": { root, version, enabled, installedAt, entry? } } }
  marketplaces.json      the marketplaces added, and where from
  marketplaces/<name>/   a marketplace's repository (or the copy of a folder)
  cache/<marketplace>/<name>/<version>/   an installed plugin
```

A marketplace is a git repo or folder with `.claude-plugin/marketplace.json`; `/plugin marketplace add anthropics/claude-plugins-official` (also a git URL or a path) copies it. An entry's `source` is a path in the marketplace (`./plugins/x`) or `{source: "github" | "url" | "git-subdir", …}` with an optional `ref` and a pinned `sha`; npm sources are refused. `/plugin install name@marketplace` copies the plugin into the cache and records it in `installed.json` with its marketplace `entry` (that is where `lspServers` lives). `/plugin` lists, `search [text]` browses the marketplaces, `uninstall`, `enable`, `disable`, `marketplace remove|update`. Removing a marketplace uninstalls its plugins. The reply says what the plugin brings and what takes a new session.

What a plugin brings, for sessions on neo:

| In the plugin                                             | Becomes                                                                                                                                                                                         |
| --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `skills/<name>/SKILL.md` (and `skills` paths in manifest) | neo's skills, read when a session starts; the skill's text ends with its base directory                                                                                                         |
| `commands/<name>.md`                                      | `/name` in the menu (`/plugin:name` when the name is taken by a daemon command or another plugin; always valid). `$ARGUMENTS`, `$1`…`$9` are filled in; the chat shows what was typed (`shown`) |
| `.mcp.json`, `mcpServers`                                 | MCP servers the daemon runs (below), with `${CLAUDE_PLUGIN_ROOT}` expanded                                                                                                                      |
| `lspServers` in the entry or manifest, `.lsp.json`        | language servers of services/lsp, after the fixed ones; read on every file                                                                                                                      |
| `agents/`, `hooks/`                                       | not used by neo yet                                                                                                                                                                             |

Not done for commands: `` !`cmd` `` output and `@file` includes in the body (the text goes to neo as it is), and `allowed-tools`.

### MCP servers the daemon runs

Most MCP servers are stdio programs, which the gateway's Streamable HTTP client can't reach. The daemon starts them from `mcpServers` in Claude Code's shape: `{ command, args, env }`, or `{ type: "http", url, headers }`. They come from `mcpServers` in `~/.config/neo/config.json`, from enabled plugins' `.mcp.json`, and, if config.json has `"projectMcp": true`, from the `.mcp.json` of the session directories (off by default: a repository's file starts programs). `${VAR}`, `${VAR:-default}` and `${CLAUDE_PLUGIN_ROOT}` are expanded. Each server is a `service` connector named `mcp-<name>` (`mcp-<plugin>-<name>` for a plugin's, unless the server has the plugin's name): `tools_search` finds its tools, `tools_call` runs them, sessions are told with `<app connected=…/>`. A server that dies is started again after 1 s, 2 s, 4 s … up to 60 s (the pauses start over once one ran for a minute; after 8 failures in a row it is left alone until the configuration changes). Its stored tools stay searchable while it is down. They stop with the daemon. A server starts with the SDK's small default environment plus its `env`, not the daemon's.

### Hooks

neo runs Claude Code's hooks (`neo/src/hooks.ts`, `session-hooks.ts`), so plugins that ship a `hooks/hooks.json` work as they are. The configuration is Claude Code's settings shape, `{ "hooks": { "PreToolUse": [{ "matcher": "Bash|Edit", "hooks": [{ "type": "command", "command": "…", "timeout": 10 }] }] } }`, read from `hooks` in `~/.config/neo/config.json`, from `<cwd>/.neo/settings.json` and from `hooks/hooks.json` (or the manifest's `hooks`) of each enabled plugin, where `${CLAUDE_PLUGIN_ROOT}` is the plugin's folder. All of them run, side by side; identical commands run once. A hook gets the event as JSON on stdin (`session_id`, `transcript_path`, `cwd`, `hook_event_name`, and `tool_name`, `tool_input`, `tool_response`, `prompt`, … by event) and answers with an exit code and perhaps JSON. 0 goes on (`continue: false` ends the turn, `systemMessage` is shown to the user, `decision: "block"`, `hookSpecificOutput`: `permissionDecision`, `updatedInput`, `additionalContext`); 2 blocks and stderr is what the agent is told; anything else is an error shown in the chat and nothing more. Plain stdout of `UserPromptSubmit` and `SessionStart` is context for the agent.

| Event              | When                                                                                                                                                                                                                                                                                                                             |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PreToolUse`       | Before a tool call, before the permission question. `allow` and `deny` replace it, `ask` asks even where the approvals mode would not (`PermissionAsk.hook`); the daemon still asks about sensitive files, and review mode still lets the user take an edit back. `updatedInput` rewrites the arguments (bash, read, write, web) |
| `PostToolUse`      | After a call (`PostToolUseFailure` after a failed one). Exit 2, `decision: "block"` and `additionalContext` are added to the tool's result for the agent                                                                                                                                                                         |
| `UserPromptSubmit` | Before each prompt. Context joins the prompt as `<hook-context>`; exit 2 drops the prompt and tells the user                                                                                                                                                                                                                     |
| `Stop`             | When the agent would end its turn (`SubagentStop` in a subagent). Exit 2 or `decision: "block"` sends the agent back to work with the reason; the hook is told `stop_hook_active` the second time, and after 20 rounds the turn ends                                                                                             |
| `SessionStart`     | Before the session's first prompt (`source` startup or resume); its context joins that prompt                                                                                                                                                                                                                                    |
| `SessionEnd`       | When the daemon stops (`AgentSession.close`)                                                                                                                                                                                                                                                                                     |

Not run: `Notification` (the harness can't tell when the daemon really asks the user), `PreCompact` (neo has no compaction), hook types other than `command`, and the results of `async` hooks (they run in the background). Hooks with an `if` run only for calls that match it.

Matchers and `tool_input` use Claude Code's names (`neo/src/tool-names.ts`); `transcript_path` is a JSONL file in Claude Code's format, rewritten before every event but the tool ones.

| neo tool                                               | Claude Code                                    | arguments                                                                                                 |
| ------------------------------------------------------ | ---------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `bash`                                                 | `Bash`                                         | `command`, `description`, `timeout` (seconds → ms)                                                        |
| `read`                                                 | `Read`                                         | `path` → `file_path`, `offset`, `limit`                                                                   |
| `write`                                                | `Write`                                        | `path` → `file_path`, `content`                                                                           |
| `edit`, one edit                                       | `Edit`                                         | `file_path`, `old_string`, `new_string` (the lines the tags name, before and after), `replace_all: false` |
| `edit`, several                                        | `MultiEdit` (a matcher for `Edit` matches too) | `file_path`, `edits[]` of the same, and `old_string`/`new_string` joined                                  |
| `todo_write`, `skill`, `web_fetch`, `web_search`       | `TodoWrite`, `Skill`, `WebFetch`, `WebSearch`  | `todos`; `name` → `skill`; `url`; `query`                                                                 |
| `delegate`                                             | `Task`                                         | `agent` → `subagent_type`, `task` → `prompt`, `description`                                               |
| `tools_call`                                           | `mcp__<app>__<tool>`                           | the connector tool's own arguments                                                                        |
| the rest (`tools_search`, `models`, `send_message`, …) | neo's name                                     | neo's arguments                                                                                           |

### Agent definitions

`delegate` also reaches agents a user, a project or a plugin defines, in Claude Code's format (`neo/src/agents.ts`): a markdown file whose frontmatter has `name`, `description`, `tools` and `model`, and whose body is the system prompt. They are read from `<cwd>/.neo/agents/*.md`, `~/.config/neo/agents/*.md` and `agents/*.md` of each enabled plugin; when a name is defined twice the project's wins over the user's, which wins over a plugin's (a plugin's agent is also `plugin:name`). neo lists them in its system prompt, like skills, as `- name: description`. `delegate {agent: "reviewer", task}` runs neo as a subagent with the body under neo's own instructions, only the tools the list allows, and the model; the "Start a subagent?" question names the agent and the model, and `subagent_started` carries `name` for the lists (the terminal's and the GUI's) to show. A name that is also a harness (`neo`, `claude`, `codex`) is the harness.

- **Tools** are Claude Code's names, mapped as for hooks (§ Hooks): `Read`, `Write`, `Bash`, `TodoWrite`, `WebFetch`, `WebSearch`, `Skill` have a counterpart, `Edit` and `MultiEdit` bring `read` along (edit works on read's tags), `Grep`, `Glob` and `LS` mean `bash` (neo searches in the shell; an agent with only those is told to look and not change), `mcp__…` means `tools_search` and `tools_call`. Names neo has nothing for (`NotebookRead`, `KillShell`, `Agent(…)`) allow nothing. No `tools:` means all of them.
- **Model** is a neo model setting (`provider/id`, optionally `@host`). `sonnet`, `opus`, `haiku`, `inherit` and `claude-*` name Claude models, which neo has no account for: they stand for the session's model, i.e. the user's choice for neo's subagents, else the configured default. A `model` in the delegate call wins over the definition's. The approvals mode is the user's and applies to the subagent as to neo.
- Each agent keeps a conversation of its own per session (follow-up delegations continue it), next to the plain neo subagent's. Only one neo subagent, definition or not, runs at a time in a directory. An agent definition works in the session's directory (no `cwd` or `worktree` yet). Not read: `color`, `effort`, `maxTurns`, `skills`, `hooks` and the other fields Claude Code's agents can carry.

### Model proxy

`proxy` in `config.json` maps a provider to a base URL, `{"openrouter": "http://localhost:8787/v1"}`, and every request of that provider's models (turns, titles) goes there instead of to the model's own `baseUrl` (`neo/src/proxy.ts`, applied where neo calls pi-ai). A proxy such as headroom forwards to one upstream, so the setting is per provider and each needs its own proxy or port. pi-ai makes decisions from the base URL it is given, so a proxied model carries what the real URL would have caused: the completions compatibility flags (DeepSeek's API takes no `store` and no developer role; the reasoning format) are written into `model.compat` from the real URL, and the body parts that only a request to openrouter.ai (`provider: {only: […]}`, how a model is pinned to a host) or api.openai.com (`prompt_cache_key`) gets are put back by an `onPayload` hook. `proxy.test.ts` compares the bodies pi-ai builds for every provider's models with and without a proxy. Checked against headroom (`headroom proxy`, `OPENAI_TARGET_API_URL=https://openrouter.ai/api/v1`): cached tokens come back in the usage as they do without it (DeepSeek on OpenRouter, ~100% of the prefix on the second request), the host pin and `reasoning.effort` reach the upstream unchanged, and with `HEADROOM_OUTPUT_SHAPER=1` a chat-completions request only gets a steering paragraph appended to its system message (effort routing exists for Anthropic's `output_config.effort` alone). Run it with `HEADROOM_TELEMETRY=off` and also `HEADROOM_BEACON=off` (or `DO_NOT_TRACK=1`): the anonymous session-summary beacon is on by default. Not covered: providers whose URL pi-ai builds itself (Cloudflare), and guests (Claude Code would need `ANTHROPIC_BASE_URL` in the env the daemon starts it with).

### System prompt (static, identical for every session)

````
You're an AI agent working across a suite of apps and services on the user's system.

Connectors (apps and services) announce themselves with <app connected=…/>
and <app disconnected=…/> events. Each event lists tool names only; use
tools_search to get a connector's tool schemas and tools_call to run them.

You can show UI by writing a ```neoui block. Call ui_library before your first one.

The most recent <focus> event says where the user is now.

When a connector disconnects, its tools stop working until it reconnects;
earlier schemas and results in this conversation stay valid as history.
If you need a disconnected app, ask the user to open it or use apps_launch.
After a reconnect, earlier schemas still apply unless the event shows a new version.

Tools that produce files or media return handles like asset://img-7f3.
Pass these handles to other tools to move results between apps.
````

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

### Asking for a secret

A command that needs an API key or a pasted login code asks with `secret.request {id, session, title, placeholder?}`. The daemon sends it to the one client that typed the command, and that client answers with `secret.answer {id, value?}` (no value: cancelled). It is not a card. A card is a session event: it is stored in the log for good, shown to every app on the session, and the note on its answer is logged too. A secret must be in none of those places. So the question is kept in memory by the hub only, an answer from any other client gets `bus.error`, and the question is dropped when its client disconnects. The daemon never logs the value, and a command must not put it in an error message. Apps draw it as a masked field.

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

Results with no template fall back to a generic card built from ACP's base data (`kind`, text, diff, image, terminal output). Images a tool returns become `{type: "image", src}` content: neo's read of a picture points at the file, an adapter's base64 image block is stored as an asset first, so the bytes stay out of the log; the view shows them as `image` components.

### Behavior

- **Actions send events, not code.** A button publishes `ui.action`; the daemon routes it to the right app, or to the agent as input if the app isn't open.
- **Keyboard first.** Every action has a key hint; focus moves through actions in a defined order.
- **Live updates.** Views are patched by `ref` via `view.patch`.

### As built (Phase 4)

- **Wire format.** `view.subscribe {session, agent?}` → `view.snapshot {seq, items}`, then `view.patch {seq, ops}` with ops `set` (replace or append an item by `ref`), `blocks` (replace the tail of a text component's blocks — streaming) and `remove`. Items have a `kind` (user, agent, thought, tool, notice), an optional `label` and tool `state`, and one component. `session.events` stays for raw consumers. With `agent`, the subscription is a per-agent view (§9): only that agent's work, and its snapshot and patches carry `agent` so a client following both tells them apart. A `kv` item may have a `tone` that highlights its value.
- **The daemon resolves more than templates.** Markdown is parsed into blocks and spans, and diffs into numbered lines, before anything is sent, so no renderer parses markdown or diffs text. Streaming text is re-parsed per chunk; only blocks from the first changed one are sent.
- **Templates** are registered in `app.presence.templates`, keyed by tool name, and kept after the app closes. A result's data is the tool's `structuredContent`. Template language: `{{path}}` in strings, whole-value substitution when the string is just `{{path}}`, `if`, and `each`/`as` (protocol/README.md). A template that doesn't resolve to valid components falls back to the generic card instead of failing the view.
- **neoui.** Beyond the fixed components, the model and apps can write UI as neoui programs (HTML-style markup: a fenced `neoui` block in a message, or `{"neoui": …}` as a template). The daemon parses and evaluates them into `neoui` components and blocks (the wire names are unchanged); renderers draw the evaluated tree, send `neoui.input` / `neoui.press` actions, and keep no program state. [docs/neoui.md](neoui.md) has the format, the library and how the terminal draws it; the language is in neoui's own DESIGN.md.
- **Actions.** An `app.tool` action runs that tool through the gateway when the app is open; the result appears in the chat as a tool call rendered with the app's template. Otherwise the agent gets it as input ("The user pressed …"). `copy:` stays in the client.
- **Theme** comes from `~/.config/neo/theme.json` (or `NEO_THEME`), else `protocol/themes/default.json`, and is sent after `bus.welcome`.
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

- **Asset store.** Content-addressed files under `<data>/assets` (`asset://img-<12 hex>`, `file-…` for non-images), so the same bytes always get the same handle; a JSON sidecar keeps the name, type and producing app. Bus: `asset.put {path|data}` / `asset.get {handle}` / `asset.fetch {url}` → `asset.info`; `asset.fetch` downloads a web image so clients only ever read files (http(s) to public addresses only, redirects checked too, png / jpeg / gif / webp up to 20 MB, 8 s, each URL once while the daemon runs); HTTP `/assets` for browsers. The agent's bash and read take handles as paths (bash commands get them substituted); write and edit refuse them.
- **`file.changed`** is derived from the diffs of the agent's write/edit results and passed on when an app sends it after saving (not echoed to the sender).
- **The lsp service** is a separate process the daemon starts (`kind: service`), driving gopls / typescript-language-server / rust-analyzer per project root; files are re-read before each request. Editors call it through the gateway like the agent does.
- **Slash commands** a harness offers beyond the daemon's own are a `commands_available {harness, commands: [{name, description?, hint?}]}` event in the session log; the latest of a harness replaces the earlier ones. For Claude Code and Codex the daemon takes them from the adapter's `available_commands_update` (also outside a turn, and before `session/new` answers) and logs them when they change; for neo they are the enabled plugins' commands, logged when a session starts, when plugins change and on the next input. A client shows the latest list of the harness the session runs on after its own commands, which win a name clash (the menu says so). `/name args` is sent as the user's text; a guest runs its command, neo's plugin commands are expanded by the daemon.
- **The working directory** is told to the model once per session (`<session cwd=…/>` in the first prompt), since the system prompt carries no session state.
- **App tools**: Go and Rust apps serve their tools with small hand-written MCP servers (`sdk-go/tools`, `sdk_rs::tools`), TS ones with the MCP SDK. The browser, photo editor and code editor stand-ins that ran the logo flow in Phase 5 are gone; the real apps take their place.

### The terminal app (after Phase 6)

- **`neo`** (no arguments) starts the daemon if it isn't running (`neo up`: detached, log in the data dir; `neo down` stops it; a daemon older than its code is restarted when no turn is running) and opens `neo-term`, a full-screen ratatui app (`apps/term`) for people who live in the terminal. The desktop overlay stays the home app; the terminal registers as an ordinary app (`terminal`, `terminal-2`… for more than one), so typing there pulls the agent in and other apps show the same conversation.
- **Layout:** the chat scrolls inside its pane (not the terminal's scrollback, which tmux breaks), drawn by `renderers/ratatui` in a compact style (tool lines with `⎿` summaries, no bullets, a failed call red, Ctrl-O for full results); the editor; three status lines under it (directory, branch, harness settings, where the agent is; usage and turn state; the approvals mode, shift+tab to cycle ask, edits, auto); a list of the running subagents under the status lines (while a delegation runs and 5 s after: `● main`, then `○ neo · deepseek-v4-flash  search tools: weather  1m 12s · 3.4k tokens` per subagent, built from `subagent_started` / `subagent_ended` and the labelled events; Ctrl-X or Down from an empty editor to focus it, Enter opens that agent's transcript from the per-agent view, `x` stops it via `neo.cancel_agent`, Enter on one that runs as a session of its own switches to that session); a sessions sidebar on the left (every session with its running state, title, last message and changed lines, live from `session.changed`, grouped by repository and worktree: `<repo>/.worktrees/<name>` or a linked worktree found from its `.git` file, groups by latest activity, delegated child sessions indented with `↳`; Ctrl-G to focus it, `[` / `]` jump between groups); a side column with the plan and the files (git status plus what the agent and guests changed, Enter for a diff) when there is something to show.
- **Keyboard only:** prompts and choices are answered with arrows/digits/Enter, `/` opens a command menu (daemon commands are sent as input; /sessions, /help and the like are local), /sessions or Ctrl-G switches sessions, Esc cancels a turn.

## 9. Guest agents

Claude Code, Codex and other ACP harnesses can join for heavy coding.

- Started by the ACP bridge as a sub-session (from the code editor, or by the main agent via a `delegate` tool); output streams onto the same bus.
- Connect to the same MCP gateway and see the same tools.
- **System prompt: add to it, don't replace it.** Their built-in prompts explain their own tools and are what make them good at coding. Append a short block covering the gateway, `asset://` handles and the event tags (`--append-system-prompt` for the Claude Code CLI, the `claude_code` preset + `append` in the Agent SDK, Codex's own custom-instruction config).
- The daemon adds the same connect, disconnect and focus events to their prompts.
- Guests usually reduce tool results to text, so the gateway publishes `tool.result {ref, …}` on the bus and returns text with the ref (`"Cropped → [result:a1f3]"`). Panes match the ref to the stored result and render it with its template; with no match they show the text.

### As built (Phase 6)

- **The bridge** is part of the daemon (`daemon/src/guests/`): a small hand-written ACP client (newline-delimited JSON-RPC over stdio) driving Zed's adapters, `@agentclientprotocol/claude-agent-acp` and `@agentclientprotocol/codex-acp`, started with `bunx` at pinned versions (not repo dependencies). Claude Code's adapter uses the user's own `claude` when it's on PATH. One guest process and ACP session per neo session and guest, kept while the daemon runs; follow-up tasks continue its conversation.
- **Two ways in.** The session agent's `delegate {agent, task, model?, effort?, background?}` core tool (hidden from and refused for subagents) runs the guest inside its own tool call and returns its last message (with `background: true` it returns at once and the reply comes as a message, see **Messages between agents**). Starting one is the user's call: every delegation first asks, once, unless the approvals mode is auto: one `permission_request` from the session's harness titled "Start a subagent?", whose `details` rows (Harness, Model, Effort, Mode when set, Directory; drawn as a `kv` with accent-toned values) say exactly what will run and whose `content` is the task as text (the panes cut it to its first 12 lines, Ctrl-O shows all). The options: start it, allow it for the rest of the session, use another harness instead, or reject. The call's own model and effort win over the user's settings, which fill the rest; "Allow for this session" holds for exactly that harness, directory, model and effort. A `model` or `effort` the harness lists (neo's own list, or an adapter that already runs: nothing is started just to ask) and doesn't know is refused before the question, as the tool result, with the valid names; case, spaces and the display name are forgiven (`deepseek v4 flash` finds `deepseek/deepseek-v4-flash`), the harness gets its own spelling, and `provider/id` is left to neo. The `models {agent?, query?}` core tool (hidden from subagents too) lists what each harness takes by those names, starting an adapter that isn't running yet to ask it, with the user's choice and the harness's default marked; `query` narrows the models to those containing one of its words, and the list stops after 40. The harnesses' own subagents are the user's choice (`/settings subagents`: both, delegate, native). With delegate only they are off, so work is handed on only through `delegate`, which asks first and shows the work: Claude Code gets `disallowedTools: ["Agent", "Task"]` in `session/new`'s `_meta.claudeCode.options`, and Codex `features.multi_agent` and `multi_agent_v2` false in `CODEX_CONFIG` (unless the user's own `CODEX_CONFIG` turns them on). When they are on, they show like delegate's: the daemon advertises the `subagents` client capability, so each adapter reports a subagent as an ACP session of its own (Claude Code in the ACP RFD's form, `subagent_update` on the parent; Codex in AIR's, `subagent_spawned` and `subagent_state_update`), and the daemon logs it as `subagent_started {native: true, name, task}` … `subagent_ended`, with its events labelled `claude/<8 hex>`, unique to it. So the lists show it by its name (Explore), Enter opens its transcript, its questions are its own (`Explore: …`), `x` stops it when the adapter allows that (Claude Code's), and the Agent call in the chat reads `Explore · its description` like a delegation. The user types `/claude task` or `/codex task` in any pane: the guest runs the turn instead of the agent, and a `guest_turn {agent, task, result}` event tells the agent with its next prompt (`<guest agent=… task=… stop=…>reply</guest>`).
- **Same session.** Guest messages, thoughts and tool calls are appended to the session log with `agent: "claude"` (tool call ids prefixed `claude:`). Diffs in guest results announce `file.changed` like the agent's. A delegation brackets its events with `subagent_started {id, agent, task, model?, effort?, cwd?, child?, background?}` and `subagent_ended {id, stopReason, error?}` (not context events). The session's view folds a delegation into **one tool item**: the main harness's own `delegate` call, labelled `neo · first line of the task` (from its `rawInput`, whichever harness made it), running, then done or failed, with the subagent's reply as its result; labelled events between the two markers (messages, thoughts, tool calls, its plan) are in the log but not in that view, and a turn's end closes a delegation whose end was never logged. Questions the subagent asks stay in the session's view. A direct `/claude task` turn (no markers) stays inline, labelled, because it is the user's own conversation with the guest. **A per-agent view** shows one agent's work: `view.subscribe {session, agent}` builds a `SessionView` that keeps only events labelled with that agent (the task as a `user` item labelled `task`, its thoughts, tool calls, plan, permission requests), unprefixed and unfolded; `view.snapshot` / `view.patch` carry `agent`. It is keyed by session and agent in `ViewManager` and the hub, so every client can open any subagent's transcript (the terminal app does, from its agents list); two subagents of the same harness in one session share a view.
- **Appended prompt.** Both adapters accept the same `_meta.jetbrains.air.customInstructions` on `session/new`: appended to Claude Code's preset, Codex's developer instructions. It explains the gateway, the event tags and `asset://` handles.
- **Same context.** Each guest keeps its own "told" state: its prompts start with the `<app …/>` and `<focus>` changes since its last prompt (the full list on the first).
- **Results.** The gateway knows an ACP client by `x-neo-text` (guests and the session's own ACP harness) and gives it text only (Claude Code shows `structuredContent` instead of the text when both are present): connector results get `[result:ref]` plus the file behind each `asset://` handle, and the structured result is stored as a `tool_result {ref, tool, rawOutput}` session event. The daemon's view (not the panes: they get rendered items) matches the ref in a guest's tool call text and draws the result with the app's template. `x-neo-guest` is the other meaning: a subagent, which may not delegate or ask for models but may send_message and wait (guests send both headers, a neo subagent only that one). Adapters differ: Claude Code streams a call's input in later updates (so `tool_call_update` gained `title`, `tool`, `rawInput`), Codex puts MCP results in `rawOutput`.
- **Permissions.** A guest's `session/request_permission` becomes `permission_request {id, agent, title, kind?, content?, details?, options}` (the tool call's kind and its diff or text content, so the card previews what it would change; `details` are labelled facts drawn as a highlighted key–value block above it); panes draw one button per option (action `neo.permit {id, option}`, handled by the daemon); `permission_resolved {id, option?, by?}` answers it, with no option when the turn was cancelled. `permissions: "allow"` (config) picks the allow option without asking, and so does the session's approvals mode (below). **Neo's own MCP tools** (a request titled `mcp__neo__<tool>` or `mcp.neo.<tool>`, or the tool name in Claude Code's `_meta` or Codex's input, for tools_search, tools_call, apps_launch, apps_focus, delegate, models, send_message, wait and ui_library) are allowed without a question in every mode: the gateway gates what needs gating (the delegate gate; connector calls as neo's are), so a delegation from Claude Code or Codex asks once, not twice.
- **The user decides model, effort, mode and resuming.** Nothing is chosen for them: a harness keeps its own defaults (and its own assumptions, like Claude Code's effort for Sonnet) until the user picks. `/model`, `/effort`, `/mode [harness] [value]` (any pane; the daemon answers, not the agent) set it for the session, logged as `setting_changed`; the last choice per harness is the default for new sessions. Without a value they show the harness's own options as a `choice` (buttons, `neo.choose`). The options come from the harness: ACP config options for guests (categories `model`, `thought_level`, `mode`, set with `session/set_config_option`), and the same shape from neo (`configOptions()` / `setConfigOption()` on `AgentSession`: the models of every provider that has a credential, as `provider/id`, the current one first, and the thinking levels; `providersWithCredential` in `neo/src/config.ts` reads the env vars, `NEO_API_KEY` and both auth files without refreshing anything). For `openrouter` the list is OpenRouter's live `GET /models` (`neo/src/openrouter.ts`), not pi-ai's bundled one, which lags: it is fetched in the background when the session starts, kept in memory for an hour, never waited for, and pi-ai's list serves until it arrives or when the fetch fails; pi-ai's entry wins for a model both have, the others are built from a plain openrouter model with the live context length, image input and prices. A neo model value may end in `@<endpoint tag>` (`openrouter/deepseek/x@deepinfra/fp8`; tags have slashes, so the last `@` splits, unless the whole value is a registry model whose id has an `@`). It sets `compat.openRouterRouting = {only: [tag], allow_fallbacks: false}` on a copy of the model, which pi-ai sends as OpenRouter's `provider` field; on any other provider it is refused. After `/model` sets an OpenRouter model without a host (picked or typed), the daemon asks the session's `AgentSession.followUp(category, value)`; neo answers with a `choice` "Host for <model>": "Any (OpenRouter chooses)" (the plain value) and one option per host from `GET /models/<id>/endpoints`, whose value carries the `@tag`. Picking sets the model again, without another follow-up. If the lookup fails or lists nothing, the answer is a `notice` and the step is skipped. OpenRouter knowledge stays in the harness; the daemon only asks and shows. `/resume [guest]` lists the guests' stored conversations for the session's directory (`session/list`, including ones started outside neo) and continues the picked one (`session/resume`); nothing resumes on its own. A client that draws its own picker asks `session.settings {session}` instead of the commands: `session.settings_result` lists every harness the session can switch to (`/harness`'s list) with its `title`, whether it is `current`, and its `options` (category, current value, values). Asking starts each guest's adapter for the session if it isn't running, so a harness that fails or takes over 5 s is listed with `options: []` and an `error`. With `known: true` nothing is started: neo's options and those of an adapter that already runs are listed, the others with `options: []` and no error. A client that only shows the name of the chosen model (`Haiku 5.5` for `haiku`) asks that way. Choosing is still sending the command text as `session.input`.
- **Session titles.** `SessionSummaries` names an untitled session after its first message (`title_source` `first_line`); `Titles` (`titles.ts`) then asks a model for a short one (3–8 words, subject and outcome, as `{"title": "…"}`, three attempts, "New thread" refused) and stores it as `generated`, which `session.changed` carries to the clients. A title without a source was given at `session.create` and is only replaced by `/title`, which writes the title again from the conversation so far; a generated title is written again that way every `REFRESH_EVERY` (10) user messages, which keeps it when it still fits. One call each, outside any session and logged nowhere: `Agent.generateText` for the daemon's own agent (neo: pi-ai's stream, no tools), and for a guest a throwaway adapter process and ACP conversation in an empty temporary directory (`GuestManager.generateText`; a tool asking is turned down). The model is the user's choice, `/title-model [harness] [model|default]`, kept for every session (`setting_changed {harness: "daemon", option: "title_model", value: "claude haiku"}` shows it in the chat); until then `claude` + `haiku` if that guest exists, else the daemon's agent on its own model. `DaemonOptions.titles` / `NEO_TITLES=off` turns the automatic title off.
- **Plans and usage.** Two session events feed status widgets, from the main agent and from guests alike; neither is a context event and neo's restore ignores both. `plan {agent?, entries: [{content, status, priority?}]}` is an agent's whole todo list (ACP's plan update; neo's `todo_write` core tool, emitted when the model makes the call): each one replaces that agent's previous list, and the view draws one checklist item per agent (`plan`, `plan-<agent>`), removed when the list is empty. `usage {agent?, contextUsed?, contextSize?, cost?, inputTokens?, outputTokens?}` carries an agent's latest numbers (consumers keep the newest per agent; the chat doesn't show it). neo sends it after each assistant message: `contextUsed` is that request's input + cache + output tokens, `contextSize` the model's window, and cost and token counts are summed over every message in the log, so they survive restarts. Guests send it for ACP `usage_update` (`used`, `size`, and `cost` only when the currency is USD). A plan update still ends the guest's current message; a usage update does not.
- **Approvals mode.** `/approvals [ask|review|edits|auto]` per session (no value: the next mode, ask → review → edits → auto → ask; shift+tab in the terminal app), `setting_changed {harness: "daemon", option: "approvals"}`. A new session always starts in ask. The change is state for the panes' status line only: the view draws nothing for it. What each mode does:
  - **ask**: file edits are previewed before anything is written (the permission card shows the diff), and bash, starting a guest and guests' requests ask too.
  - **review** ("ask after edit"): each edit is written right away, so editors and the files panel show it at once, and then one `permission_request` for that edit waits for a choice: titled like "Edited src/a.ts — keep it?", the diff as content, options Keep and Revert. Everything that isn't an edit asks as in ask. neo holds the tool call (below): Keep gives the normal result, Revert puts the file back and the tool fails with "The user reverted this edit; the file is back as it was before." (or, when the file changed since, "…but the file had changed since, so it was left as it is."). An ACP harness's edit request is allowed without a question and its tool result can't be held, so the card comes when the completed `tool_call_update` with the diff arrives (`review.ts`; also for edits made without any request); Revert restores the file and appends `edits_reverted`, so the agent hears it with its next prompt. Reverting is the same as below: only when the file still has what the edit wrote, CRLF and BOM kept, a created file deleted. There is no end-of-turn card in this mode: every edit was already reviewed.
  - **edits** ("accept edits"): edits are written right away; afterwards one Keep / Revert card (below) lets the user undo them. Everything that isn't an edit still asks (bash, starting a guest, a guest's non-edit requests). A guest's request counts as an edit when its tool call has kind `edit` or carries a diff.
  - **auto**: nothing asks, except sensitive reads (below).
    Config `permissions: "allow"` for a guest still allows everything in every mode.
- **Reads and sensitive files.** In every mode reading is allowed without a question: neo's `read` tool and an ACP harness's permission requests of kind `read` or `search`. The exception is a **sensitive** file, which asks in every mode, auto included (`permission_request` kind `read`, titled like "Read .env?"; options Allow, Allow for this session, Reject). `sensitive.ts` decides, after resolving the path (relative to the session's directory, `~`, symlinks when the file exists): `.env` and `.env.*` (not `.env.example`, `.sample`, `.template`), `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa`, `id_ed25519`, `id_ecdsa` (not their `.pub`), anything under `~/.ssh/` and `~/.gnupg/`, `~/.aws/credentials`, `~/.netrc`, `~/.pi/agent/auth.json`, `~/.local/share/neo/auth.json`, `~/.config/gh/hosts.yml`, `~/.docker/config.json`, `~/.kube/config`, `*credentials*.json`, `.npmrc`, `.pypirc` and `secrets.*`. A command that mentions a sensitive path (`cat .env`, `source .env`, `grep x ~/.ssh/config`; the command is split into words, `$HOME` and `~` expanded) asks in every mode too, titled "This command touches .env" with the command as content; this holds for neo's bash and for a guest's `execute` request (its `rawInput.command`). A guest's read is judged by the tool call's `locations` and the path fields of its `rawInput`. "Allow for this session" is remembered per file, not per kind, and also covers commands that mention that file. Edits are not asked about by what they touch; the gateway tools are unaffected. A guest configured with `permissions: "allow"` is the user's opt-out and is not asked either.
- **The agent's own actions.** `NewSessionParams.requestPermission(request, signal) → Promise<boolean>` is the harness's hook for asking. neo's `bash`, `read`, `write`, `edit` and `web_fetch` call it before they act (`todo_write`, `skill`, `web_search` and the gateway tools never do); without the hook (tests, other embedders) nothing changes. The hook answers `{allowed, note?, review?, reverted?}`. `write` and `edit` compute the new content first and ask with the diff (`path`, `oldText`, `newText`) before writing; `bash` asks with the command as the title and kind `execute`; `read` asks with kind `read` and the absolute `path` (the daemon allows it unless the file is sensitive); `web_fetch` asks with kind `fetch` and `fetch <url>` as the title, since a URL the model picks can carry data out. `web_search` never asks; how much adult content it may return is `NewSessionParams.safeSearch()`, read at each search: the user's `/safesearch strict|moderate|off`, one value for every session kept in the store's meta table (no session logs it, as it isn't a session's), else config.json's `web.safeSearch` or `NEO_SAFE_SEARCH`. A refusal makes the tool fail with "The user declined this edit." / "The user declined to run this command." / "The user declined to let you read this file.", which the model reads as a tool error. The daemon builds the hook per session (`permissions.ts`) from the mode: auto allows, edits allows edits and asks for the rest, ask asks; review answers an edit's first question with `review: true` (go ahead) and the tool, once the file is written, asks again with `phase: "after"` (same title, the diff): the daemon shows the Keep / Revert card and, on Revert, restores the file itself before answering `{allowed: false, reverted}`, so the tool can tell the model whether the file is back. A cancelled turn leaves a written edit as it is. The question is a `permission_request {agent, title, kind, toolCallId, content, options}` (Allow, Allow for this session, Reject); "for this session" is remembered per session and kind (per file for sensitive ones), in memory. The tool's abort signal drops the question when the turn is cancelled. `permission_request` gained `kind` (the ToolKind asked about) and `content` (diffs or text to preview, shown above the buttons; the answered item collapses to one line).
- **Review after the turn (edits mode).** `review.ts` tracks the edits applied without asking during a turn, from completed tool calls' diff content in the log (path, `oldText`, absent for a created file; `newText`), for every agent including guests and `/claude` direct turns. When `turn_ended` arrives with edits, one `choice` asks, titled like "Edited 3 files: src/a.ts, src/b.ts, notes.md", with Keep, Revert all and (for 2 to 10 files) "Revert <file>". Reverting restores each file to the `oldText` of its first edit in the turn (a created file is deleted), but only when the file still has the turn's last `newText`; otherwise the file is left alone and reported. Line endings and a BOM the file has are kept. A per-file revert asks again about the rest. The result is an `edits_reverted {paths, skipped?}` session event: a context event, so the agent's next prompt starts with `<reverted paths="a.ts, b.ts"/>` (a `skipped` attribute lists the left-alone files; nothing when nothing was restored), and the chat shows a notice ("reverted a.ts"). Open cards don't survive a daemon restart; the edits stay on disk.
- **Notes with an answer.** A button press (`neo.permit`, `neo.choose`) may carry `note`: what the user adds for the agent, e.g. "use tabs instead" (the terminal app: Tab on the focused button). It is logged on `permission_resolved` / `choice_made` and shown on the collapsed answer ("Allow · “use tabs instead”"). How the agent hears it: neo's own tools get it in the tool call's result (allowed: a line "The user added: …" after the result; refused: after the decline message; a Revert in review mode likewise), and so does `delegate` (start or decline). Where the tool result can't be changed (a guest's or an ACP harness's permission request, every `choice` such as the end-of-turn review card), the asker appends a `user_note {note, about}` session event, a context event: the next prompt starts with `<note about="Edited b.ts — keep it? → Revert">…</note>` (a prompt already running isn't interrupted). `/claude` direct turns leave it pending for the session's agent.
- **Show, don't tell.** A toggle of the user's, for every session (`/settings show-dont-tell on|off`, or `/settings` for a card of the toggles with their descriptions; kept in the store's meta table like safe search). Each agent is told when it changed since it was last told, with its next prompt, the way connectors are: `ToldState.changes` adds a `show_dont_tell {on, library?}` context event before the connector events (the session's own agent from the session log, each guest process and neo subagent from its own `ToldState`). On, it carries what `ui_library` returns then, and the prompt starts with `<show-dont-tell on="true">`, a request to answer with neoui blocks where an answer has a shape and keep the prose short, then the library; off, `<show-dont-tell on="false">` and a line to answer in text again. Nothing is shown in the chat; the command's notice says it.
- **The session's harness.** Each session runs on one harness: neo (the default until the user chooses), `claude` (Claude Code) or `codex` (Codex), switched with `/harness [name]` (no name: a `choice` titled "Harness (now: neo)"). Stored as `setting_changed {harness: "daemon", option: "harness", value}`; the chat shows a notice ("harness: claude"), unlike the approvals mode. The last choice is the default for new sessions: a new session whose default isn't the daemon's own agent gets the event at creation, so it keeps that harness whatever is chosen elsewhere later. The valid names are the daemon's agent and the configured guests; an unknown stored one falls back to the agent. `/model`, `/effort` and `/mode` default to this harness. Each turn goes to the session's harness: neo through the in-process agent, an ACP harness through `GuestManager.agent(name)`, an `Agent` like neo whose session is an ACP conversation in "main" mode: its updates carry no `agent` label (messages, tool calls, plan, usage: the status widgets' main-agent slot gets them), tool call and message ids are prefixed `<name>-main` to stay unique next to a `/claude` guest of the same name (a separate process), and its permission requests take the guests' per-mode mapping above, so the edits-mode review card covers its edits too. It reaches the gateway with `x-neo-text` only, so it may delegate. The conversation's id is logged as `harness_message {harness: "claude", message: {acpSession}}`; after a daemon restart the harness is resumed with `session/resume`, or started anew (told the whole conversation, below) when that fails. Connectors and focus are told per harness process, from its own `ToldState`; the session's `guest_turn` and `edits_reverted` events come from the session's pending events. The direct `/claude task` turns still run a labelled guest whatever the session's harness is. Cancel, queueing and idle work as for neo.
- **The session's directory.** `/cd <dir>` moves an idle session (relative to where it is, `~` for home; it must be a directory; refused while a turn runs): the store's `cwd` changes, the agent sessions it loaded close so the next turn loads them, with the history, there (an ACP harness as after a daemon restart: `session/resume` in the new directory, else started anew and told the conversation), the guests it started stop (`GuestManager.stop`), and `session.changed` carries the new `cwd` to the apps. Everything else reads the directory from the store when it needs it (permissions, delegate, plugins). The chat gets a notice ("moved to …") and the agent a `user_note` with its next prompt; no event of its own is logged, so a replay doesn't know the old directory. `/cd` alone says where the session is.
- **Switching harness.** The harness that runs next never saw the turns another ran, so its prompt gets a `<conversation-so-far>` block before the pending events (`handoff.ts`): for each such turn the user's message and the agent's final message (tool calls left out; each text cut at 1500 characters, the whole at about 6000, older turns dropped first). A turn belongs to the harness chosen before its `user_message`; a harness is told the turns after its own last one (neo keeps its history in `harness_message` events, an ACP harness in its own conversation), and all turns when its conversation is new or lost. Turns the user gave a guest directly aren't part of it (the agent gets them as `guest_turn` events).
- **Subagents.** `delegate {agent}` takes `neo`, `claude` or `codex`, also the harness the session runs on (a fresh subagent; a `claude` guest is a separate process from the session's own Claude Code). `model` and `effort` are for that task only: they are set on the subagent's session with `session/set_config_option` by category (neo: `setConfigOption`), put back when it finishes, never logged as the user's settings, and the harness's refusal (an unknown model) is the tool result. **Directories and worktrees.** `delegate` also takes `cwd` (absolute, or relative to the session's directory; must exist) and `worktree` (a name matching `[A-Za-z0-9][A-Za-z0-9._-]*`, not both): the daemon runs `git worktree add .worktrees/<name> -b <name>` in the repository's main checkout (found through `git rev-parse --git-common-dir`, so from inside a worktree too; only when the session's directory is in a repository; `.worktrees/` is added to `.git/info/exclude` unless ignored already; an existing branch of that name is checked out instead) after the user agreed, and the gate's Directory row says `(new worktree, branch fix-login)` or `(worktree)`. A delegation into another directory does not run inside the session: it creates, or reuses (per parent, directory and harness), a **child session** (`SessionInfo.parent`, a `parent` column in the store) whose cwd is that directory, whose harness is the delegated one (`setting_changed harness`, plus the call's model and effort, and the parent's approvals mode), and whose turn the delegate call queues (`app: "delegate"`, the task as the message) and waits for; the tool result is that turn's final reply (with a line naming the directory and session). The parent logs `subagent_started {…, cwd, child}`, so its view still shows one tool item and the agents list can open the child; the child is a normal session in the sidebar (grouped under its worktree, marked as a child). The child's questions (permission requests, choices) are copied into the parent's log while the delegation runs, so they are answered where the user is looking (`Asker.mirror`; answering in either session settles both). A child session cannot delegate. Without `cwd` and `worktree` the subagent stays in the session as above. `neo.cancel_agent {id}` (a `ui.action`) stops one running subagent (in-session: `session/cancel` for a guest, `cancel()` for neo; child: the child's turn is cancelled); the delegate call returns "stopped: cancelled" and the parent's turn goes on.

A neo subagent (`guests/subagent.ts`) is a new session of the neo harness (the daemon's agent when that is neo, else `DaemonOptions.neo`), kept per session like a guest, reaching the gateway with `x-neo-guest` only; its updates are labelled `agent: "neo"` with prefixed ids, and its `harness_message` events are not logged (they aren't neo's history of the session). Its bash, write and edit ask by the approvals mode like neo's own.

- **Messages between agents** (`daemon/src/messages.ts`; issue #479). The session's own agent is `main`; a running subagent is its id (`d_1a2b3c4d`, from `subagent_started`) or, when only one of that harness runs, its harness name. `send_message {to, text}` writes to any of them but oneself; `wait {for?, timeoutSeconds?}` blocks until a message arrives for the caller or one of the named subagents ends (it returns at once when something is waiting, and for `main` when no subagent runs; after `timeoutSeconds`, default 60, max 3600, it says nothing came). Both go through the gateway, which tells the caller apart by its headers: `x-neo-session` and, for a subagent, `x-neo-guest` (the harness name), or, for the subagent that runs as a session of its own in a directory, that session's id. Neither tool asks the user.
  - **Background.** `delegate {background: true}` asks the same "Start a subagent?" question (with a `Runs: in the background` row), then returns at once: `Started claude as d_1a2b3c4d in the background…`. `subagent_started` carries `background: true`. The subagent's final reply is delivered to `main` as a message with `reply="true"`; one the user (or a cancelled turn) stopped reports that too, but without starting a turn. Without `background` the call blocks as before and `main` can't answer anything meanwhile, so `send_message` to `main` from such a subagent is refused ("main is waiting for you to finish…"); peers may still write to it. One subagent per harness and directory runs at a time (a second `delegate` says to message the first), which is how a harness name finds its recipient.
  - **Events.** Every message is an `agent_message {from, to, fromAgent?, toAgent?, text, reply?}` in the session's log (`from`/`to` are `main` or ids; `reply` marks a background subagent's final reply). The chat draws one compact `tool` row per message, `message: claude d_1a2b3c4d → main` (`reply: …` for a final reply) with the text as its expandable body; a message to or from a subagent also shows in its transcript (what was written to it as a `from main` input like the task; what it sent as a row, its final reply not repeated), and as its activity in the agents list (`← main`, `→ main`). The turn an idle agent starts for a message is a `user_message` with `app: "agents"` that the chat does not draw (the row is the message). A background subagent's work stays hidden from the chat while it runs, also past the end of the turn that started it (`turn_ended` closes only the windows of foreground subagents); its list row is marked `[bg]`.
  - **Delivery** is per agent type, and each message is delivered once (it sits in the recipient's in-memory inbox until somebody takes it). neo (the session's agent or a subagent) takes its inbox through `PromptParams.takeMessages`, which the loop polls where it polls its steering queue: at the start of a turn and after every batch of tool calls, also when it would stop; the messages join the conversation as `<message from=… agent=…>…</message>` user messages after the tool results (logged as `harness_message`, so a reload keeps them). An ACP agent (Claude Code, Codex) can't be told anything mid-turn, but every neo tool it calls passes the gateway, which appends the inbox to that tool's result (all ACP callers have `x-neo-text`); a long run of the adapter's own tools therefore delays a message until its next neo tool call or the end of its turn. What is still waiting when a turn ends starts the next one: for the session's agent in the session's drain loop (`takeMessages`, an input with `app: "agents"`), for an in-session subagent in `delegate`'s loop, for a subagent in a directory of its own in that session's drain loop (and `SessionManager.wake` after `idle`). A message for an idle `main` starts a turn at once (`Messenger` → `SessionManager.wake`), unless the last turn was cancelled or failed. `wait` takes the inbox too, so nothing arrives twice.
  - **Limits.** 20 messages per direction between a pair of agents until the user writes (a `user_message` of any app but `agents` and `delegate` starts a new round), 20 unread messages per agent, 20 000 characters per message; a final reply is not counted. The refusal is the tool result and says what to do (stop, finish, ask the user). A recipient that has ended is "claude d_… has ended (finished); it can't read messages" (the last 50 per session are remembered); an unknown one lists who runs.
  - **Cancellation.** The user cancelling the session's turn (`cancel`, also when it is idle) stops its background subagents (`SessionManager.onCancel` → `delegate.stopBackground`); `x` in the agents list (`neo.cancel_agent`) stops one; the daemon stopping stops all and waits a moment for them. A stopped subagent logs `subagent_ended {stopReason: "cancelled"}` and `main` finds `[claude stopped: cancelled]` in its inbox without a turn being started for it. Messages and background subagents don't survive a daemon restart: a `subagent_started` with `background` and no end is closed with an error when the daemon starts.

- **Accounts for guests.** A guest uses its own login (Codex's ChatGPT, Claude Code's) until the user points it at a neo API key (`/login`, §4). They do that with the model: `/model codex openrouter/openai/gpt-5-mini` (`provider/model`; `/model codex` lists the first models of every provider that has a key, other ids can be typed; the same for the session's own harness: `/model openrouter/…` after `/harness codex`). `guests/accounts.ts` turns the pick into a `Route` and `GuestManager` starts the adapter again on it, resuming the same conversation (`session/resume`; a new one when that fails). A plain model afterwards starts it again on its own login. The pick is the session's `model` setting like any other, so it is the default for new sessions; a key that has gone since means the adapter starts on its own login and the log says so. Keys come from the provider's variable, neo's file or pi's, as API keys only: never an OAuth login (a Claude Pro/Max login is for neo only, and Codex keeps its ChatGPT login), never `NEO_API_KEY`. The key goes to the adapter in memory and never into the session log; errors from a start on a key are cut of it.
  - **Codex** (`route: "codex"` in the guest config). Providers that serve the Responses API, the only one Codex still speaks (`wire_api = "chat"` is gone): OpenAI, OpenRouter, Groq, xAI, Fireworks, Vercel AI Gateway. The adapter's own way for this is its `gateway` sign-in: the client says `auth: {_meta: {gateway: true}}` in `initialize`, then `authenticate {methodId: "gateway", _meta: {gateway: {baseUrl, headers: {Authorization}, providerName}}}`, and the adapter makes a `custom-gateway` provider (`wire_api = "responses"`) from it. The model goes in the adapter's `CODEX_CONFIG` (`{"model": id}`, merged into what the user has there), because the adapter only takes the model ids Codex lists through `session/set_config_option`, not a provider's. We did not use `[model_providers.<id>]` with `env_key`: the adapter asks Codex whether it needs a login at every session start, and Codex answers from its global config, so that needs a `CODEX_HOME` of our own with a `config.toml` (verified: an empty one gives "Authentication required"), which loses the user's ChatGPT login and history. The gateway sign-in touches no files and leaves `~/.codex` alone. Providers with only chat completions (DeepSeek, Mistral, Cerebras, Z.ai, Moonshot…) can't be used by Codex: #476.
  - **Claude Code** (`route: "claude"`). The adapter starts with the credential in its environment, which the Agent SDK passes on to Claude Code. An Anthropic API key (`anthropic/…`) is `ANTHROPIC_API_KEY` and `ANTHROPIC_MODEL`. An Anthropic-compatible endpoint is `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN`, with `ANTHROPIC_MODEL` and `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL` all set to the picked model, since the aliases would ask the endpoint for Claude models it doesn't have. Endpoints, from the providers' own Claude Code guides: DeepSeek `https://api.deepseek.com/anthropic`, OpenRouter `https://openrouter.ai/api`, Moonshot `https://api.moonshot.ai/anthropic`, Z.ai `https://api.z.ai/api/anthropic`, MiniMax `https://api.minimax.io/anthropic`. An environment credential outranks Claude Code's own login (cloud provider variables, then `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`, `apiKeyHelper`, `CLAUDE_CODE_OAUTH_TOKEN`, profiles, then `/login`), which is why nothing is set until the user picks a model with a provider, and why a pick clears every one of those variables the daemon itself was started with. A plain model goes back to Claude Code's login. A `sk-ant-oat…` token (a Claude subscription) is refused wherever it is stored, as an API key or in `ANTHROPIC_API_KEY`. A user's `~/.claude/settings.json` `env` block is applied by Claude Code after the process environment and wins over all of this.
- **Config.** `config.json` `guests: {claude: {title, command, permissions}, …}`; env `NEO_GUESTS` (`none` or a list), `NEO_GUEST_PERMISSIONS`.

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
- **Loop state.** Confirm the session log can own the history. The loop is ours now, so change it if not.
- **pi-ai on Bun.** Confirm pi-ai and our loop run cleanly under Bun (provider SDKs, streaming).
- **Tool names (resolved in Phase 1).** Anthropic and OpenAI only accept `[a-zA-Z0-9_-]` in tool names, so the core tools are `tools_search`, `tools_call`, `apps_focus` (and the system prompt says `apps_launch`). Connector tool names like `browser.download` are unaffected: they are arguments to `tools_call`, not tool definitions.
- **`apps_launch`.** The system prompt and the closed-app error tell the model to call `apps_launch`, but it isn't in the fixed tool list. Either add it as a core tool or make it a connector tool reached through `tools_call` (and reword the prompt), before Phase 2.
- **Tool list grows after Phase 1.** Phase 1 ships exactly four tools; Phase 2 adds `tools_search`/`tools_call`/`apps_focus`. Sessions created before that get one prompt-cache miss when the list changes. Harmless during development, but the "never changes" rule only holds from Phase 2 on.
- **What the log stores.** Phase 1 persists every streaming chunk as its own row (so a pane attaching mid-turn replays exactly what others saw), plus a `harness_message` event per complete model message, which is what the harness rebuilds its context from (provider fields like thinking signatures can't be rebuilt from chunks). That roughly doubles the text stored per turn. Coalescing chunks after a turn ends belongs with the compaction question above.
- **Crash mid-turn.** On restart the daemon appends `turn_ended {stopReason: "error"}` to any turn without one, and the harness answers interrupted tool calls with a synthetic error result. A bash command that was running is not re-run, and its side effects are unknown to the model beyond "interrupted".
- **Queued input visibility.** `user_message` is logged when its turn starts, not when it's queued, so other panes don't see a queued input until it runs. A `session.input_queued` event (or queue state in `SessionInfo` updates) may be wanted once several apps share a session.
- **Model credentials.** pi-ai's built-in model list lags new releases (the harness clones a known model of the same provider for unknown ids). Keys come from the provider's env var, `NEO_API_KEY`, then neo's `auth.json` (`NEO_AUTH`, default in the data dir), then pi's `~/.pi/agent/auth.json`. Both files hold pi's format: `api_key` entries and `oauth` logins (ChatGPT, Copilot, Claude Pro/Max). neo refreshes an expired login before the request and writes it back to the file it came from. A refresh rotates the refresh token, so it runs under pi's own lock (`proper-lockfile` on the file) and reads the entry again once it has the lock; neo and a running pi on the same file never refresh at once. A failed refresh ends the turn with "log in again" and leaves the file alone. **Login.** `/login [provider | key [provider]]` (`daemon/src/login.ts`): no argument asks which kind (API key, Claude Pro/Max, ChatGPT, GitHub Copilot, plus any account provider registered with pi-ai); an account provider's id (`anthropic`, `openai-codex`, `github-copilot`) starts its login; `key <provider>` or any other pi-ai provider id asks for that provider's API key. `tinyfish` does the same for TinyFish's key, which is for neo's web search, not a model (`TINYFISH_API_KEY` outranks it). A key or a pasted code is asked with `secret.request` (above), so it is in no card and no log; the login callbacks map onto the session: `onAuth` becomes a `notice` with the address and instructions, `onProgress` a notice, `onSelect` a `choice`, `onPrompt` and `onManualCodeInput` a secret. The command returns once the login has started. Cancelling a prompt, the client disconnecting or ten minutes passing aborts it (an account login with the browser callback has a code prompt open all the time, so a disconnect is seen). Errors are cut of every value the user pasted and every token received before they are shown or logged. The result is a `notice` ("Logged in to deepseek (API key).") and `setAccount` on neo's file. `/logout [provider]` removes the entry from neo's file only; pi's file is pi's own and a variable stays set, and the notice says so. `/accounts` lists each provider with a login, its kind, where it comes from (variable, neo's file, pi's file; the first is used) and when a token expires, never a secret. `notice {text, tone?}` is a session event for lines like these; it is not a context event.
- **Presence carries an MCP endpoint, not tool names (Phase 2).** Apps bring their own MCP server and announce its URL; the gateway lists the tools, caches the schemas (SQLite) so search and "isn't open" errors work while the app is closed, and owns `v`: it bumps it whenever the schemas or the server's `instructions` change. Apps never need to manage a version.
- **When connect/disconnect events are written.** They are computed per session at the start of each turn, as the difference between what is open now and what that session's log last said. Idle sessions get no writes, and an app that closes and reopens between two prompts produces nothing. The consequence: an app that opens mid-turn is only announced on the next prompt.
- **Connector instructions.** An MCP server's `instructions` are shown with that connector's tools in `tools_search` results (the overlay uses this for its ramble skill). They live in the history as tool results, so the cache is untouched, but they are repeated on every search that hits that connector.
- **Approvals are now urgent.** Without a permission flow the overlay confirms `click` and `type_text` with Return in its own UI before running them. Any other app with side effects has to do the same until neo has approvals.
- **Which session do apps follow?** There is no "current session" yet: the overlay follows the most recently active one, and its reset creates a new session that other apps don't know to follow. Phase 3's `agent.focus {session, app}` needs a daemon-owned answer.
- **Missing for the overlay's UI.** Usage/context-size events, daemon settings (model, effort, cwd) and a list of models, and subagent parent ids on tool calls. All were shown by the overlay under the old companion daemon.
- **Daemon restarts are invisible to clients.** Clients resume subscriptions from the last seq they saw, which is correct as long as the log is append-only. Once compaction or log truncation exists, `bus.welcome` should carry a daemon/log instance id so clients can tell they need a full replay.
- **One current session.** Phase 3 answers "concurrent sessions" with one current session that every app follows; other sessions remain reachable from the CLI (`neo attach <id>`), get no `<focus>`, and an `apps_focus` call from one of them makes it current. Several agents active in different apps at once would need per-app sessions.
- **Focus is the agent's location, not the user's.** `<focus>` follows the agent (moved by the user or by itself). The user switching windows without moving the agent isn't reported; if the model should know where the user is looking, apps need to report window focus separately.
- **No GPUI neo app yet.** Grove will be one and host `renderers/gpui`. Phases 3 and 5 used a stand-in code editor (Pad), since removed; the desktop app (`apps/gui`) is GPUI but draws its chat in Svelte.
- **Actions the agent doesn't see.** A button that runs an open app's tool directly shows its result in the chat, but the result is not in the agent's history (only in the session log). If the agent should know, actions need to reach the harness as context (like connector events) at the next prompt.
- **App-side live updates.** The design's `view.patch {ref, patch}` from apps (e.g. an export's progress) isn't built: apps can't address a result they returned, because MCP gives them no id for the call. Options: the gateway passes the tool call id in `_meta`, or apps return a progress token the daemon maps to the item.
- **Pixel snapshots.** Renderer snapshots are text (TUI), HTML (web) and a layout model (GPUI). Screenshot tests need a headless browser and offscreen GPUI rendering; not set up yet.
- **The view has no turn state.** Nothing in a view says whether the last agent message is still streaming, so `neo attach` also follows `session.events` to know when a turn runs and waits 150 ms after the last change before treating an item as settled. Giving agent and thought items `state: running` until `turn_ended` (one more `set` per turn) would make the view self-sufficient.
- **Emphasis without color.** With no color (`NO_COLOR`, pipes) the TUI drops bold, italic and inline code silently, so a highlighted phrase reads like plain text. Textual markers (`*…*`, backticks) in that mode are an option if it matters.
- **Templates can't slice lists (resolved).** `each` takes a `limit`, and `{{index}}` is the 1-based position.
- **The lsp service writes files on `rename`.** Real apps need the approvals story (see above) before the agent can reach the local network or write outside a project.
- **The bash tool's file changes aren't announced.** `file.changed` comes from write/edit diffs (and apps saving); `cp asset://… ~/site/logo.png` through bash changes a file nobody hears about. Watching the session's cwd, or diffing mtimes around a bash call, would close the gap.
- **Guest sessions end with the daemon.** After a restart a delegation starts a fresh guest conversation; the user can `/resume` the old one.
- **Approvals cover the main agent's file edits and bash, not connector calls.** `bash`, `write` and `edit` are gated by the session's approvals mode (§9 As built), as are guests and starting a guest. `tools_call` into an app's tools isn't: apps with side effects still confirm in their own UI. Bash commands that change files aren't part of the edit review (see the bash note above). "Allow for this session" answers live in memory only (a restart asks again).
- **The session's harness (resolved).** The user picks it per session (`/harness`, §9 As built). Left open: an ACP harness resumed after a restart is told the connectors again (its told state is in memory), a switch back to a harness doesn't tell it about `guest_turn`s the other harness had already consumed, and the summary is the final messages only, not what the tools did.
- **Open questions don't survive a restart.** Pending choices and permission requests are answered with "none" when the daemon stops; their buttons then report that they're no longer open.
- **Guests' histories and the main log.** The main agent sees a guest's work only as the `delegate` result or a `guest_turn` summary, not its tool calls; the log has everything. Whether (and how) the main agent should see more is part of the compaction question.
- **Permission noise (resolved for neo's own tools).** Claude Code in its default mode asked before every neo tool call and Codex reviewed them itself; the daemon now answers those requests itself (§9, Permissions), so only the gateway's own gates ask (delegate). The adapters' requests for their own tools (bash, edits) still follow the approvals mode.
- **Pending events across a guest turn.** After a restart, the agent's pending connector events are recomputed from the last `user_message`; a `/claude` turn's message counts, so events from before it that the agent hadn't been told yet are lost (they reappear only if something changes again).
- **Presence is the only way to update context (resolved).** `app.context {app, context}` updates just the context: memory only, no store write, no `connectors.changed`. The SDKs keep it for reconnects (`setContext` / `SetContext` / `set_context`).
