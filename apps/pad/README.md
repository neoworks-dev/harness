# apps/pad

Pad (binary `suite-pad`) is a rough GPUI **code editor stand-in** until Grove exists: files with line numbers on the left, the agent's chat pane (`renderers/gpui`) on the right. It is the app **`code-editor`** on the bus (title "Code editor (Pad)"), so `bin/suite move code-editor` brings the agent's pane here, and the agent can `apps_focus` it and call its tools.

```sh
bun run daemon                                  # terminal 1
cargo run -p suite-pad -- ~/site/src/Header.tsx # terminal 2 (path[:line] arguments are optional)
bin/suite move code-editor                      # the chat pane opens on the conversation
```

Needs the suite daemon (it exits otherwise). The lsp service (started by the daemon) gives it diagnostics; without it the problems panel says so.

## Keys

| Key                                                                                      | Does                                                                                                                                                   |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `ctrl-o`                                                                                 | open a file: a path prompt, prefilled with the current file's folder (`path` or `path:line`, `~` and relative paths work; enter opens, escape cancels) |
| `ctrl-s`                                                                                 | save, tell the other apps (`file.changed`), check diagnostics again                                                                                    |
| `ctrl-r`                                                                                 | discard edits and reload from disk                                                                                                                     |
| `ctrl-w`                                                                                 | close the file (twice to discard unsaved edits)                                                                                                        |
| `ctrl-pagedown` / `ctrl-pageup`                                                          | next / previous file; or click a tab                                                                                                                   |
| `ctrl-1` / `ctrl-2` / `ctrl-3`                                                           | focus the editor / the chat pane / the chat input (`F6` cycles; 2 and 3 need the agent to be here)                                                     |
| arrows, home (first non-blank, then column 0), end, page up/down, `ctrl-home`/`ctrl-end` | move; with `shift` they select; the mouse clicks and drags                                                                                             |
| `ctrl-a`, `ctrl-c`, `ctrl-x`, `ctrl-v`                                                   | select all, copy, cut, paste                                                                                                                           |
| `enter` (keeps the indent), `tab` (indent), `backspace`, `delete`                        | edit                                                                                                                                                   |
| `ctrl-z`, `ctrl-shift-z` / `ctrl-y`                                                      | undo, redo                                                                                                                                             |
| in the chat input                                                                        | `enter` sends, `escape` cancels the running turn, `up` moves into the chat's buttons                                                                   |

Rough edges: no syntax highlighting, no horizontal scroll (long lines are cut off), no word movement, no search, one window, UTF-8 text files up to 4 MB.

## What it does

- **Editing**: tabs, line numbers, cursor and selection, undo; a file that is already open and unmodified is **reloaded when it changes on disk** (the daemon sends `file.changed` when the agent writes or edits a file, other apps send it after saving). With unsaved edits it is not touched; the tab gets a `!` and the status line says "modified, changed on disk" (`ctrl-r` takes the disk version, `ctrl-s` overwrites it).
- **Diagnostics**: after opening, saving or reloading, Pad asks the lsp service through the daemon's MCP gateway (`POST ${url}/mcp`, core tool `tools_call {name: "lsp.diagnostics"}`, with the URL and token from `daemon.json` beside the bus socket) on a background thread. Problems are underlined, the line number gets a red/amber dot, and the list at the bottom jumps to a line when clicked. Edits turn the list "stale" until the next save.
- **Presence**: app `code-editor`, `launch: {command: [<this executable>]}` (so `apps_launch` can start it), `mcp` = its tool server, `templates` for the tool results, and `context: {file, line}` of the current file, updated as the cursor moves (the agent sees it in `<focus>` while it is here).
- **Chat images**: `asset://` images in the transcript are resolved with `client.get_asset` (see `renderers/gpui`).

## Tools (`suite_sdk::tools::ToolServer`, on loopback)

| Tool        | Args                       | Result (`structuredContent`) → template                                                                                                                                                                        |
| ----------- | -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `open`      | `path` (absolute), `line?` | `{path, lines, language, line}` → file chip + Line / Lines / Language                                                                                                                                          |
| `reveal`    | `path`, `line`, `column?`  | opens, scrolls to the middle, highlights the line until the cursor moves; `{path, line, column, text, lines, language}` → file chip (`path:line`) + the line as code                                           |
| `selection` | none                       | `{open, path, language, line, column, lineText, text, startLine, startColumn, endLine, endColumn, dirty}`; `text` is the selected text → file chip (`path:start-end`) + `code` with the selected text + Cursor |
| `files`     | none                       | `{count, files: [{path, current, dirty, changedOnDisk, state, lines, language}]}` → table                                                                                                                      |

Tool calls arrive on the tool server's threads and are forwarded to the window through a channel (`tools::Request`), answered from the GPUI side with a 10 s timeout. A bad path or a file that isn't UTF-8 text is a tool error.

## Code

`buffer.rs` (text, cursor, selection, undo), `editor.rs` (open files, save/reload/conflict, diagnostics, marks), `gateway.rs` (gateway call + parsing), `tools.rs` (tools, templates, `apply` on the model), `pad.rs` (window state, keys, bus events, tool requests, IME), `view.rs` (drawing), `field.rs` (the single-line inputs), `focus.rs` (follows `agent.focus`). None of the first four touches GPUI.

## Tests

`cargo test -p suite-pad`: unit tests of the model, tools, templates' data and the gateway client (against a fake HTTP server), and headless GPUI tests (`TestAppContext`, no window) against the real daemon (`bun run daemon/src/main.ts`, echo agent, `SUITE_SERVICES=none`; skipped if bun is missing): the pane following the agent, the tools called through the gateway (with a stand-in `lsp` connector answering diagnostics), `file.changed` both ways, keys, focus switching, and `asset://` images. The window itself (real fonts, pixels, IME, the mouse) is not covered by any test.
