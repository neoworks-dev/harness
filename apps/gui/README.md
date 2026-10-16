# apps/gui

`suite-gui`, the suite's desktop app: the shared conversation in a native window, with the sessions of every project at its left, the agents working for you under the input, and the plan and changed files at its right. It is the terminal app ([apps/term](../term/README.md)) with a mouse, a real window and pictures.

It is only a client. The conversation lives in the daemon, so closing the window never stops work, and the terminal app, Pad, the browser and other windows show the same session.

## Running it

```sh
make build                  # bin/suite and bin/suite-gui (builds the window's UI too)
suite gui                   # the agent's session if it works in this directory, else the latest one here, else a new one
suite gui --session 3fa9    # a session by id or unique prefix
suite gui --new             # a new session in this directory
```

`suite gui` starts the daemon if it is not running, opens the window and returns: the terminal is free to use. The window's own output goes to `gui.log` in the data directory. `SUITE_GUI` names another `suite-gui` binary.

## What you see

- **Sessions** (left, Ctrl-S shows or hides it). Every session, grouped by project and, under it, by worktree (`<repo>/.worktrees/<name>` or any linked worktree). The newest activity is first. Each shows its title, the lines its edits changed (`+12 −3`), the last thing said and how long ago. The session you are in has a bar in the margin; one that is working has a spinner; one that finished while you looked elsewhere has a green dot and a bold title until you open it, and the header counts them (`3 done`). A session that another one delegated to sits under its parent with `↳`. Press a row to open it, `+ New` to start a session in this directory.
- **The chat** (middle), the input under it, and the status line at the bottom.
- **Agents** (under the input). While a subagent or guest runs, and for a few seconds after, one row each: its harness and model, what it is doing, how long it has run and its tokens. Press a row (or Enter on it) to read that agent's own chat, `stop` (or `x`) to end it. An agent working in a worktree of its own opens that session instead.
- **Plan and files** (right, only when there is something to show and the window is wide enough). The plan is the agent's todo list, and each guest's. Files are what git says changed in the session's directory plus what the agent touched, tagged by who (`(agent)`, `(claude)`); press one to see its diff.
- **GitHub** (Ctrl-R), in place of the chat. The issues and pull requests of the project's repository, as `gh` finds it. Open one to read its description, comments, reviews and checks; **Send to the model** puts it into the conversation. It only reads: the app never writes to GitHub. It needs the [GitHub CLI](https://cli.github.com) installed and logged in (`gh auth login`).

## Keys

| Key       | What                                                        |
| --------- | ----------------------------------------------------------- |
| Ctrl-S    | show or hide the sessions                                   |
| F6        | the sessions get the keyboard (again to leave)              |
| Ctrl-N    | a new session                                               |
| Ctrl-X    | the agents list gets the keyboard (again, or Esc, to leave) |
| Ctrl-R    | the GitHub view (again to close)                            |
| Ctrl-O    | tool results in full                                        |
| Ctrl-T    | the agent's thoughts                                        |
| Shift-Tab | approvals: ask, review, edits, auto                         |
| Esc       | closes what is open; else cancels the running turn          |

In the sessions: Up/Down (or j/k) move, `[` and `]` jump between projects and worktrees, Home/End go to the ends, Enter opens, Esc leaves.

In the agents list: Up/Down select, Enter opens that agent's chat (Esc or Enter comes back), `x` stops it.

In the GitHub view: Tab (or 1 and 2) switches between issues and pull requests, Up/Down move, Enter opens, `/` searches (GitHub's search syntax, such as `label:bug author:alice`), `o` switches between open and all, `r` reloads, `s` sends the item to the model, `d` shows a pull request's diff, Esc goes back and then closes.
