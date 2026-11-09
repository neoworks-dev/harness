# neo on the neoshell desktop

An Agents widget for [neoshell](../../../neoshell): what your sessions are doing, which need you,
and what today cost. A row opens its session in `neo-term`.

It is an extension an installed program ships (neoshell's `sdk/README.md`): the widget runs
sandboxed in the shell, and this app is its data source. It follows the daemon's session list and
serves it on the session bus as `dev.neoworks.Neo.Agents`.

```sh
task shell-widget:install   # build, then install for this user; restart neoshell once
```

That writes:

- `~/.local/share/neoshell/extensions/dev.neoworks.agents/` holding `manifest.json` and
  `client.js`;
- `~/.local/share/dbus-1/services/dev.neoworks.Neo.Agents.service`, so D-Bus starts this app the
  first time the shell asks for its data. It then waits for the daemon if that isn't up.

The widget shows, for each session (subagents fold into their parent):

- a dot for its status: needs you, working, done and not yet seen, or idle;
- its title and cost;
- in the large size, its last message and how full its context is.

Today's cost counts the sessions active since local midnight, each with everything it has spent.

A row opens `neo-term --session <id>` in `$TERMINAL`, or else in the first terminal found on
`PATH`. The terminal starts with the environment D-Bus started this app with. If nothing opens,
run `dbus-update-activation-environment --systemd PATH WAYLAND_DISPLAY` once in your session
startup.

`bun test apps/shell-widget` covers the summary, the widget's wording, the terminal command line,
and a round trip over the real session bus. That last test is skipped where there is no session
bus.
