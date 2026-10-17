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

- **Sessions** (left, Ctrl-S shows or hides it). Every session, grouped by project and, under it, by worktree (`<repo>/.worktrees/<name>` or any linked worktree). The newest activity is first. Each shows its title, the lines its edits changed (`+12 −3`), the last thing said and how long ago. The session you are in has a bar in the margin; one that finished while you looked elsewhere has a bold title until you open it, and the header counts them (`3 done`). A session that another one delegated to sits under its parent with `↳`. Press a row to open it.
  - **Search** (Ctrl-Shift-F) replaces the list with the sessions whose title, last message or project holds every word typed, titles first; Up and Down walk the results, Enter opens one, Esc clears. It searches what the daemon lists, not whole conversations.
  - The header's buttons **filter by project** (a popover of the projects; the choice is kept while the window is open), **add a project** (the system's folder picker, then a new session in the folder) and start a **new session** in the shown session's project (Shift asks which project).
  - **Pin** a session to keep it at the top, or **settle** it to fold it away under "Settled (n)" at the bottom until it does something again. Both are in the row's right-press menu, with the folder and id to copy and a project filter; hovering a row shows a Settle button where its age is. Pins and settled sessions are kept in `suite-gui/sessions.json` in the data directory.
  - Where a row shows its age it shows what the session is doing instead: a blue spinner and `Working 12s` while it runs (counted from when the window first saw it run), a green `Done` once it finished while you looked elsewhere.
- **The chat** (middle), the input under it, and the status line at the bottom. A turn running shows its work as it comes (commands, reads, edits and the rest as rows of one group, "Working for 12s" under them); a finished turn folds it under "Worked for 3m 43s", with the files it changed on a card under the agent's last reply. Press a row to see what it did.
- **Attachments.** The paperclip picks files, and Ctrl-V pastes an image from the clipboard (it needs `wl-paste` or `xclip`). They wait over the input and go with the next message; an image shows as a small picture with a button in its corner to take it back, other files as chips with their names.
- **Agents** (under the input). While a subagent or guest runs, and for a few seconds after, one row each: its harness and model, what it is doing, how long it has run and its tokens. Press a row (or Enter on it) to read that agent's own chat, `stop` (or `x`) to end it. An agent working in a worktree of its own opens that session instead.
- **Plan and files** (right, only when there is something to show and the window is wide enough). The plan is the agent's todo list, and each guest's. Files are what git says changed in the session's directory plus what the agent touched, tagged by who (`(agent)`, `(claude)`); press one to see its diff in the viewer.
- **Workspace** (Ctrl-G shows or hides it), a card floating at the conversation's top right: this machine and the session's folder (copy the path, show it in the file manager), **Open in** the editor last used (the others found on `PATH`: zed, code, cursor, windsurf, idea, subl), the branch (switch to a local one; it refuses while the tree has changes), **Commit & push** (Enter in the message field commits everything, and pushes if the branch has an upstream; or leave the commit to the agent) and **Changes** with the lines added and removed, which opens the viewer.
- **The viewer** (beside the chat, Ctrl-D or the diff button at the window's right edge; it takes the side panel's place while open). The **Diff** tab lists every file git sees changed (**Uncommitted**) or the files the agent edited in the latest turn (**Latest turn**), each under a header that stays at the top as you scroll: the path (press it to open the file in a tab of its own), what it changed and a button to copy the path. Unmodified lines are folded away, press the fold to show them. The toolbar switches between unified and split lines, wraps long lines, folds every file and reloads; the panel's left edge drags to resize it, and its button makes it take the conversation's place (the input stays under it). A diff the agent shows in the chat, and a pull request's diff from the GitHub view, open here too.
- **Comments on code.** Press a line number in the diff or in a file (shift-press another to stretch the selection) and write what you think of those lines. The comment waits as a chip over the input, with the lines marked in the viewer, and goes with your next message as a block: the file, the lines, the quoted code and your words. A message of comments alone is fine.
- **GitHub** (Ctrl-R), in place of the chat. The issues and pull requests of the project's repository, as `gh` finds it. Open one to read its description, comments, reviews and checks; **Send to the model** puts it into the conversation. It only reads: the app never writes to GitHub. It needs the [GitHub CLI](https://cli.github.com) installed and logged in (`gh auth login`).

## Keys

| Key          | What                                                            |
| ------------ | --------------------------------------------------------------- |
| Ctrl-S       | show or hide the sessions                                       |
| F6           | the sessions get the keyboard (again to leave)                  |
| Ctrl-N       | a new session                                                   |
| Ctrl-Shift-F | search the sessions                                             |
| Ctrl-X       | the agents list gets the keyboard (again, or Esc, to leave)     |
| Ctrl-R       | the GitHub view (again to close)                                |
| Ctrl-D       | the viewer: the diff and files beside the chat (again to close) |
| Ctrl-G       | show or hide the workspace card                                 |
| Ctrl-O       | tool results in full                                            |
| Ctrl-T       | the agent's thoughts                                            |
| Shift-Tab    | approvals: ask, review, edits, auto                             |
| Esc          | closes what is open; else cancels the running turn              |

In the sessions: Up/Down (or j/k) move, `[` and `]` jump between projects and worktrees, Home/End go to the ends, Enter opens, Esc leaves.

In the agents list: Up/Down select, Enter opens that agent's chat (Esc or Enter comes back), `x` stops it.

In the GitHub view: Tab (or 1 and 2) switches between issues and pull requests, Up/Down move, Enter opens, `/` searches (GitHub's search syntax, such as `label:bug author:alice`), `o` switches between open and all, `r` reloads, `s` sends the item to the model, `d` shows a pull request's diff, Esc goes back and then closes.
