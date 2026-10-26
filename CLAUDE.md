## Git

Commit the worktree first if it's dirty, then write your changes. Short, to-the-point commit title; a body explaining the change when the title doesn't carry it; ask if you're unsure what to write. On a feature branch, only commit what that branch is for.

No trailers, ever: no `Co-Authored-By` on a commit, no "Generated with Claude Code" on a PR.

## Issues

Issues on `neoworks-dev/harness`, this repo's remote, carry goals across sessions. A session ends and its context goes with it; an issue is the only thing that carries a goal to the next one. So:

- Work that finishes in this session, with me here, needs no issue.
- Work that won't finish in one session gets one, however loosely defined it still is.
- Something concrete found along the way, that isn't what we're doing now, gets one instead of being done.

Write them as soon as the list exists, not once the work starts.

Write to GitHub as the bot: issues, comments and labels go through `gh bot` (`gh bot issue comment 12 --body …`), so they show as `neoworks-bot[bot]`, not as me. Plain `gh` is for reading only. The bot as author already says a model wrote it, so no "written by Claude" line in the text. If `gh bot` fails, say so rather than falling back to plain `gh`. It lives in `~/Documents/neoworks/gh-bot`, and `bun run qa` uses it on its own.

Every issue gets a type, a priority, an effort and an area. Type, priority and effort are GitHub's own issue type and the organization's issue fields, never labels.

- **Type** — `Bug` something is broken, `Feature` something new, `Task` everything else.
- **Priority** — `Urgent` drop everything, `High` next up, `Medium` normal, `Low` nice to have.
- **Effort** — `Low` an hour or two, `Medium` about a day, `High` several days or needs splitting.

`gh bot` sets all three at once, by name: `gh bot issue create --type Bug --priority High --effort Low --label area:neo …`, and `gh bot issue edit 12 --priority Urgent` to change them later.

Area is a label, exactly one of:

- `area:daemon` — sessions, views, permissions, commands, the gateway
- `area:neo` — the neo agent: loop, tools, skills, hooks, models, accounts
- `area:guests` — Claude Code, Codex and other harnesses over ACP, delegate
- `area:ui` — neoui, the view protocol, the renderers
- `area:term` — the terminal app (`apps/term`)
- `area:gui` — the desktop app (`apps/gui`)
- `area:plugins` — plugins, marketplaces, MCP servers, language servers
- `area:apps` — the SDKs and the stand-in apps

Two areas is fine when an issue genuinely spans them; three means split it.

`ai-found` is not a third axis — the QA harness adds it to everything it files, so a model's findings can be told apart from a person's. Nothing else uses it.

Use Edit, Read and Write instead of sed/awk/python.
