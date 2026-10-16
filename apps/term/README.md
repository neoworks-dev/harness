# apps/term

`suite-term`, the suite's full-screen terminal app: the shared conversation in a pane that scrolls on its own, an input editor, status lines under the prompt, and the plan / changed-files / usage panels. `suite` with no arguments starts the daemon if needed and opens it in the current directory.

It is only a client. The conversation lives in the daemon, so closing the app never stops work, and the overlay, Pad, the browser, the photo editor and other terminals show the same session. It registers as the app `terminal` (a second one as `terminal-2`, …; kind app, not home, context `{cwd}`), and the agent moves into it when you start it or type in it.

```sh
make build                      # bin/suite-term (next to bin/suite)
suite                           # opens it (SUITE_TERM overrides which binary)
suite-term --new                # a new session in this directory
suite-term --session 3fa9       # a session by id or unique prefix
```

Without flags it shows the agent's current session if that works in this directory, else the most recently active session here, else creates one. The daemon not running prints `daemon not running: run suite (or suite up)` and exits 1 before the screen is touched. Alternate screen, raw mode and bracketed paste are restored on exit and on panic. There is no mouse capture, so tmux (or the terminal) selects text as usual.

## Layout

```
 ... chat (scrolls inside the pane; follows the tail)             │ plan
 ...                                                              │ files
 ─────────────────────────────────────────────────────────────    │
 > message the agent · / for commands        (grows to 8 lines)
 ~/site · main ↑1 · pi · model sonnet · in photo-editor
 ctx 35% · $0.18 · ◐ working 12s · 2 queued · disconnected — reconnecting
 ⏵⏵ auto (shift+tab to cycle)                       press ctrl-c again to quit
```

- Line 1: the directory, git branch, the settings chosen for each harness (`/model`, `/effort`, `/mode`, only what is set) and, when the agent is in another app, `in <app>`.
- Line 2: the usage line, then the turn state (a spinner and the time while a turn runs, queued inputs) or the lost connection in the danger color.
- Line 3: the approvals mode (see below), and short messages (errors from the daemon, "copied").
- The side column (plan above files) shows only when there is something in it and the terminal is wide enough. A diff opened from the files panel replaces the chat.
- Items are drawn by `renderers/ratatui` (cached per item, width and options), with a blank line between them. `NO_COLOR` turns colors off.

## Keys

| Key                                          | What                                                                                     |
| -------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Enter                                        | send (trimmed, non-empty)                                                                |
| Alt-Enter, Ctrl-J                            | new line (pasting several lines keeps them in the editor)                                |
| Up / Down                                    | move between lines; on the first / last line through this session's input history        |
| Ctrl-A / Ctrl-E, Alt-B / Alt-F, Left / Right | line start / end, words, characters                                                      |
| Ctrl-W, Ctrl-U, Ctrl-K, Alt-D                | delete word back, to line start, to line end, word forward                               |
| PgUp / PgDn                                  | scroll the chat a page                                                                   |
| Home / End                                   | top / bottom of the chat when the editor is empty (Ctrl-Home / Ctrl-End always)          |
| Ctrl-U / Ctrl-D                              | half page up / down when the editor is empty (Ctrl-D quits when the chat is at the tail) |
| Ctrl-O                                       | expand / collapse tool results (`/expand`)                                               |
| Ctrl-T                                       | show / hide thoughts (`/thoughts`)                                                       |
| Esc                                          | close the menu; else dismiss the prompt highlight; else cancel the running turn          |
| Ctrl-C                                       | clear the editor; on an empty editor press twice within 1.5 s to quit                    |
| Shift-Tab                                    | approvals: ask / auto                                                                    |
| Tab                                          | complete the slash command; next button of a prompt; else editor → Plan → Files → editor |
| Ctrl-S                                       | session switcher (`/sessions`)                                                           |
| 1-9, arrows, Enter                           | answer the active prompt (while the editor is empty)                                     |
| Ctrl-P / Ctrl-N                              | pick an older prompt / back (else history)                                               |
| ? (empty editor), `/help`                    | key list                                                                                 |
| Ctrl-L                                       | repaint                                                                                  |

While scrolled up, new output does not move the view; a dim `↓ new output (End)` shows. Sending or a new prompt jumps back to the tail.

**Prompts.** Permission requests and choices (`/model`, `/resume`, "start a guest?") are items with buttons. The newest one that is still pending is active: drawn with a bar in the margin and its first button reversed. Arrows or Tab move between buttons, Enter or the digit presses one (`ui.action`), Esc dismisses the highlight. `copy:` buttons copy through OSC 52 (tmux needs `set -g set-clipboard on`).

**Commands.** `/` at the start of the input opens a menu (Up/Down, Tab completes, Enter runs). Sent to the daemon like any input: `/clear`, `/model`, `/effort`, `/mode`, `/resume`, `/claude <task>`, `/codex <task>`, `/approvals [ask|auto]`. Local: `/sessions`, `/expand`, `/thoughts`, `/help`, `/quit`.

**`/clear`** makes the daemon switch to a fresh session in the same directory; the app follows it (the next `agent.focus` names it) and starts over with an empty chat and panels.

**Approvals.** `ask` (default) shows guest permission requests and "start a guest?" as buttons; `auto` lets the daemon answer them (allow once). Shift-Tab sends `/approvals auto|ask`; the mode comes back as a `setting_changed {harness: "suite", option: "approvals"}` event, so every pane agrees.

**Sessions.** Ctrl-S lists sessions (title, directory, age, state); type to filter, Enter switches (and moves the agent to the new one), the first row starts a new session here. Input history is kept per session in memory and in `<data dir>/term-history/<session>.jsonl`.

## How it is built

- `app.rs`: `App` is the whole state; `handle_key`, `handle_event` (the SDK's events), `handle_msg` and `tick` return `Effect`s (send input, press a button, cancel, switch session, copy, quit) and `draw` paints a frame. No terminal or network in it.
- `driver.rs`: picks the session at start, runs effects on a worker thread (SDK requests block up to 10 s) and pumps events into the app.
- `runtime.rs`: terminal setup and restore, the loop (poll keys 50 ms, drain events, tick, redraw only when something changed).
- `chat.rs` (view items, cached lines, scrolling), `editor.rs`, `history.rs`, `menu.rs` (commands, switcher), `status.rs`, `widgets/` (side panels).

## Tests

`cargo test -p suite-term`: unit tests for editing, history, the slash menu, scrolling and follow, prompt buttons, Esc / Ctrl-C, status lines, session choice, and screen snapshots at 100x30, 60x20 and others (`tests/snapshots/`, rewritten with `UPDATE_SNAPSHOTS=1`), all on ratatui's `TestBackend`. `tests/daemon.rs` starts the real daemon (echo agent, temp socket and data dir; skipped without bun), connects the app, chats, toggles approvals, `/clear`s, and runs two terminals side by side. Nothing opens a terminal.
