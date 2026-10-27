# claude-agent-acp patches

Changes to [claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp) that neo wants
upstream. Each is a `git format-patch` against the tag named here; none is applied to the adapter
neo runs (`guests/config.ts` pins it).

- `0001-background-tasks.patch` (v0.91.0): `_session/background_tasks {sessionId, toolCallId?}`
  sends a running tool call's task to the background, as Ctrl+B does in Claude Code, through the
  SDK's `query.backgroundTasks`. Advertised as `_meta.backgroundTasks.supported`. The daemon uses
  it when an adapter advertises it (`GuestManager`, AgentSession.background); term's Ctrl-B offers
  it to neo's commands only until then. Issue #50.

To try one: `git am` it onto the tag in a clone, `npm run build`, and point `guests.claude.command`
in `~/.config/neo/config.json` at `node <clone>/dist/index.js`.
