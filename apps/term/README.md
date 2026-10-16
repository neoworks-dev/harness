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
 Sessions · 3               │ ... chat (scrolls inside the pane;     │ plan
▎  Sticky header            │     follows the tail)                  │ files
▎  Done. The header now…    │                                        │
 ◐ Fix the login  +12 −3    │ ─────────────────────────────────────  │
   Looking at the valid…    │ > message the agent · / for commands   (grows to 8 lines)
                            │ ~/site · main ↑1 · pi · model sonnet · in photo-editor
                            │ ctx 35% · $0.18 · ◐ working 12s · 2 queued · disconnected — reconnecting
                            │ ⏵⏵ accept edits on (shift+tab to cycle)       press ctrl-c again to quit
```

- Line 1: the directory, git branch, the session's harness (`pi`, `claude`, `codex`; `pi` until a `/harness` setting says otherwise) with the settings chosen for it (`/model`, `/effort`, `/mode`, only what is set) and, when the agent is in another app, `in <app>`.
- Line 2: the usage line (context and cost of the session's own agent; guests' usage is left out), then the turn state (a spinner and the time while a turn runs, queued inputs) or the lost connection in the danger color.
- Line 3: the approvals mode (see below), and short messages (errors from the daemon, "copied").
- The side column (plan above files) shows only when there is something in it and the terminal is wide enough. A diff opened from the files panel replaces the chat.
- The sessions sidebar (left, 31 columns) lists every session, newest activity first: a spinner while it runs, the title (bold accent for the one you are in, with a bar in the margin), `+added −removed` lines of its edits in green and red, and under the title a muted line with the last message (`2 queued ·` in front when inputs wait). It is fed by `list_sessions` at start and `session.changed` events, so other terminals' and apps' sessions move and update live. It needs 52 columns for the chat beside it (and yields to the plan / files column when both do not fit), else it is hidden; `/sidebar` switches it off and on.
- Items are drawn by `renderers/ratatui` (cached per item, width and options), with a blank line between them. `NO_COLOR` turns colors off.

## Keys

| Key                                          | What                                                                                                |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| Enter                                        | send (trimmed, non-empty)                                                                           |
| Alt-Enter, Ctrl-J                            | new line (pasting several lines keeps them in the editor)                                           |
| Up / Down                                    | move between lines; on the first / last line through this session's input history                   |
| Ctrl-A / Ctrl-E, Alt-B / Alt-F, Left / Right | line start / end, words, characters                                                                 |
| Ctrl-W, Ctrl-U, Ctrl-K, Alt-D                | delete word back, to line start, to line end, word forward                                          |
| PgUp / PgDn                                  | scroll the chat a page                                                                              |
| Mouse wheel, Shift-Up / Shift-Down           | scroll the chat 3 lines / 1 line (the mouse is captured: select text with Shift-drag)               |
| Home / End                                   | top / bottom of the chat when the editor is empty (Ctrl-Home / Ctrl-End always)                     |
| Ctrl-U / Ctrl-D                              | half page up / down when the editor is empty (Ctrl-D quits when the chat is at the tail)            |
| Ctrl-O                                       | expand / collapse tool results (`/expand`)                                                          |
| Ctrl-T                                       | show / hide thoughts (`/thoughts`)                                                                  |
| Esc                                          | close the menu; else dismiss the prompt (the editor comes back); else cancel the running turn       |
| Ctrl-C                                       | clear the editor; on an empty editor press twice within 1.5 s to quit                               |
| Shift-Tab                                    | approvals: ask → review → accept edits → auto → ask (`/approvals <next>`)                           |
| Tab                                          | complete the slash command; a note for the agent on a prompt; else editor → Sessions → Plan → Files |
| Ctrl-G                                       | focus the sessions sidebar (shows it if switched off); again to leave                               |
| Ctrl-S                                       | session switcher (`/sessions`)                                                                      |
| Ctrl-R                                       | GitHub issues and pull requests (`/github`)                                                         |
| 1-9, arrows, Enter                           | answer the active prompt (it has taken the editor's place)                                          |
| Ctrl-P / Ctrl-N                              | pick an older prompt / back (else history)                                                          |
| ? (empty editor), `/help`                    | key list                                                                                            |
| Ctrl-L                                       | repaint                                                                                             |

While scrolled up, new output does not move the view; a dim `↓ new output (End)` shows. Sending or a new prompt jumps back to the tail.

**Prompts.** Permission requests and choices (`/model`, `/resume`, "start a guest?") are items with buttons. The oldest one still pending is active and **takes the editor's place**: the question with a compact preview (a diff is cut to 10 lines; the chat keeps the whole item), the buttons with the first one reversed, and a line of keys (`enter allow once · ←/→ choose · tab add a note · esc dismiss`), with `(1 of 3)` when several wait (they come one at a time). While it is there nothing reaches the editor, so Enter can only answer the prompt: arrows move over the buttons, Enter or the digit presses one (`ui.action`), Esc dismisses the prompt (it stays pending in the chat; Ctrl-P brings it back). The draft in the editor is untouched and returns when the prompt is gone. If you typed within the last 800 ms when a prompt arrives, the editor stays and a line above it says `permission needed — waiting until you stop typing`; the prompt takes over after a pause. `copy:` buttons copy through OSC 52 (tmux needs `set -g set-clipboard on`).

**Notes.** Tab on a permission request or choice opens a one-line input under the buttons (`note › `): type what the agent should hear ("use tabs instead"), Enter answers with the focused button plus the note (`suite.permit` / `suite.choose` with `note` in the args), Esc goes back to the buttons. The answered card shows `Allow · “use tabs instead”`.

**Sessions sidebar.** With focus (Ctrl-G or Tab; the selected row is reversed): Up/Down or j/k move, PgUp/PgDn, g/G jump to the ends, Enter shows that session (the same switch as the Ctrl-S picker: the agent moves here too), Esc, q, h or Left give the keys back to the editor.

**Commands.** `/` at the start of the input opens a menu (Up/Down, Tab completes, Enter runs). Sent to the daemon like any input: `/clear`, `/model`, `/effort`, `/mode`, `/resume`, `/claude <task>`, `/codex <task>`, `/harness [pi|claude|codex]` (switch this session's agent), `/approvals [ask|review|edits|auto]`. Local: `/sessions`, `/sidebar`, `/expand`, `/thoughts`, `/github`, `/help`, `/quit`.

**`/clear`** makes the daemon switch to a fresh session in the same directory; the app follows it (the next `agent.focus` names it) and starts over with an empty chat and panels.

**Approvals.** `ask` (default) asks before file edits, commands, starting a guest and guests' requests; `review` writes each edit right away and then asks Keep / Revert for that edit (a card with its diff; the agent is told when you revert); `edits` writes the agent's edits right away and asks for the rest, and after the turn a card offers Keep / Revert all / Revert <file>; `auto` asks for nothing. Reads are allowed in every mode, except sensitive files (`.env`, keys, credentials) and commands that mention them, which ask in every mode. Shift-Tab sends `/approvals review|edits|auto|ask` (the next mode); the mode comes back as a `setting_changed {harness: "suite", option: "approvals"}` event, so every pane agrees, and the status line names it (`? ask before acting`, `⏵ review each edit`, `⏵⏵ accept edits on`, `⏵⏵ auto mode on`). Changing it is not shown in the chat.

A permission request that would change a file (or, in review mode, did) shows the diff above its buttons (additions green, removals red). Up to 20 lines are shown, then `… N more lines`; Ctrl-O shows the whole diff. The buttons stay at the end of the item, where the view jumps to when the request arrives.

**Diff view.** Enter on a file in the files panel opens its `git diff HEAD` over the chat, side by side (old left, new right) from 90 columns and unified below that. It takes every key while open: Up/Down or j/k, PgUp/PgDn, g/G, Ctrl-U/Ctrl-D scroll; `n` / `p` jump to the next / previous hunk; Left/Right or `h` / `l` scroll sideways 8 cells and `0` goes back to the left edge (long lines are cut, the marked change stays in place); `s` switches split and unified; `f` shows the whole file as context and back (it no longer pages); Esc or `q` close it.

**GitHub.** Ctrl-R or `/github` opens a full-screen view over the chat for the repository `gh` finds in the session's directory (it needs the [GitHub CLI](https://cli.github.com), logged in with `gh auth login`; a missing `gh`, a directory that is not a GitHub repository and a missing login are said so on the screen). It only reads: every command is checked against a read-only allowlist (listing and viewing issues and pull requests, a pull request's diff, one `gh api repos/…` GET) before it runs, so the app cannot write to GitHub. Everything loads on background threads with a `loading…` line; lists and details are cached until the session's directory changes (`r` reloads).

- _List_: tabs Issues / Pull requests (Tab or 1 / 2), each row `#number`, state (open green, closed red, merged accent, draft muted), title, labels, author, comment count and the time since the last update (`3h`, `2d`, `3w`). Up/Down or j/k, PgUp/PgDn, g/G and the mouse wheel move; Enter opens; `/` types a search (GitHub's search syntax, e.g. `label:bug author:alice`; Enter applies, Esc cancels); `o` switches open / all states; `r` reloads; Esc or `q` close. Columns drop from the right as the terminal narrows (labels, author, comments).
- _Detail_: title, state, author, labels, the description (markdown read in the terminal: headings, lists, quotes, code, bold / italic / inline code, links; HTML comments from templates are dropped), the comments in order, and for a pull request also the branches and size, the review decision, reviews, comments on the diff, the checks (failing and pending ones named) and the changed files. Scrolls like the diff view (keys and wheel). `d` shows a pull request's diff in the diff view above (Esc comes back). Esc goes back to the list, `q` closes.
- _To the model_: `a` (Enter in a detail) puts a `<github-issue repo="o/r" number="12" url="…" state="open">…</github-issue>` block (`<github-pr …>` for a pull request) into the editor after what is already typed and closes the view, so you can add instructions; `s` sends it at once as a user message after a line like `Here is issue #12 from GitHub.`. The block holds the title, facts, description and conversation (comments, reviews, comments on the diff), cut to about 12000 characters: the description and each comment are shortened, the oldest comments dropped first with a note. From the list both keys fetch the detail first if it is not cached (the footer says `fetching #12…`).

**Sessions.** Ctrl-S lists sessions (title, directory, age, state); type to filter, Enter switches (and moves the agent to the new one), the first row starts a new session here. Input history is kept per session in memory and in `<data dir>/term-history/<session>.jsonl`.

## How it is built

- `app.rs`: `App` is the whole state; `handle_key`, `handle_event` (the SDK's events), `handle_msg` and `tick` return `Effect`s (send input, press a button, cancel, switch session, copy, quit) and `draw` paints a frame. No terminal or network in it.
- `driver.rs`: picks the session at start, runs effects on a worker thread (SDK requests block up to 10 s) and pumps events into the app.
- `runtime.rs`: terminal setup and restore, the loop (poll keys 50 ms, drain events, tick, redraw only when something changed).
- `chat.rs` (view items, cached lines, scrolling), `editor.rs`, `history.rs`, `menu.rs` (commands, switcher), `status.rs`, `sessions.rs` (the sessions sidebar), `widgets/` (side panels), `github/` (the GitHub view: `gh.rs` the read-only runner behind a trait, `model.rs` the JSON, `md.rs` / `doc.rs` / `view.rs` drawing, `format.rs` the block for the model).

## Tests

`cargo test -p suite-term`: unit tests for editing, history, the slash menu, scrolling and follow, prompt buttons, Esc / Ctrl-C, status lines, session choice, the GitHub view (recorded `gh` JSON through a fake runner, no network), and screen snapshots at 100x30, 60x20 and others (`tests/snapshots/`, rewritten with `UPDATE_SNAPSHOTS=1`), all on ratatui's `TestBackend`. `tests/daemon.rs` starts the real daemon (echo agent, temp socket and data dir; skipped without bun), connects the app, chats, toggles approvals, `/clear`s, and runs two terminals side by side. Nothing opens a terminal.
