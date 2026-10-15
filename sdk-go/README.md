# sdk-go

Go client for the suite bus: connect, register as a connector (presence), list connectors, create, follow and drive sessions, and follow the resolved view of a session (items, theme, button actions). It has the same shape as `sdk-ts`. The only dependency besides the generated protocol types is `github.com/coder/websocket`.

```go
import (
    protocol "neoworks.dev/suite/protocol/go"
    sdk "neoworks.dev/suite/sdk-go"
)
```

Consumers outside this workspace need `replace neoworks.dev/suite/protocol/go => ../protocol/go` (and the same for `sdk-go`) in their `go.mod`, like `cli/go.mod`.

## Connect

```go
c, err := sdk.Dial(ctx, sdk.Options{Client: "my-app", Reconnect: true})
if errors.Is(err, sdk.ErrDaemonDown) { /* nothing listening on the socket */ }
defer c.Close()
```

The socket is `Options.Socket`, else `$SUITE_SOCKET`, else `$XDG_RUNTIME_DIR/suite/bus.sock`, else `/tmp/suite-<uid>/bus.sock` (`sdk.SocketPath`). `Dial` does the WebSocket upgrade at `/bus` and the hello/welcome exchange; `c.Welcome()` has the daemon info.

All methods take a `context.Context`, are safe to call concurrently, and may overlap with event delivery: replies are matched to requests by id. A reply of `bus.error` becomes an `*sdk.Error` (`errors.As`; it carries the protocol error code).

## Presence

```go
err := c.SetPresence(ctx, protocol.AppPresence{
    App: "photo", Kind: protocol.ConnectorKindApp, Title: &title, Open: true,
    Context: map[string]string{"file": "logo.png"},
    Mcp:     &protocol.McpEndpoint{URL: "http://127.0.0.1:7391/mcp"},
    Launch:  &protocol.LaunchSpec{Command: []string{"photo-editor"}},
})
```

The app stays registered while the connection lives. Call `SetPresence` again to update the context; `Open: false` or `c.Close()` unregisters. Tools are not sent over the bus: the daemon's gateway lists them from the MCP endpoint.

`c.ListConnectors(ctx)` returns what the daemon knows (app, kind, connected, `v`, tools, error).

**Out of scope:** an MCP server helper for registering tools. A Go MCP server needs a large dependency; run any Streamable HTTP MCP server and point `Mcp.URL` at it.

## Where the agent is

The daemon owns the current session and tells every client where the agent lives (`agent.focus`: session, hosting app, reason) right after the handshake and on every change.

```go
c, _ := sdk.Dial(ctx, sdk.Options{
    Client:       "my-app",
    OnFocus:      func(f protocol.AgentFocus) { /* every agent.focus, in order */ },
    OnConnectors: func(l []protocol.ConnectorInfo) { /* full list, on every change */ },
})
focus, err := c.WaitFocus(ctx) // the first agent.focus; c.Focus() is the latest, if any yet
err = c.MoveAgent(ctx, protocol.AgentMove{App: &app})       // to an open app
err = c.MoveAgent(ctx, protocol.AgentMove{Session: &id})    // no App: home; Session: make it current
list := c.Connectors()  // fetched after each (re)connect, then kept current (home, pid, context)
```

Callbacks run one at a time on their own goroutine, so they may call the client; a slow one delays later notifications, not the connection. `MoveAgent` returns after the resulting `agent.focus` was broadcast. After a reconnect the daemon sends the focus again (`Reason: initial`).

## Sessions

```go
info, err := c.CreateSession(ctx, protocol.SessionCreateParams{Cwd: &dir})
sessions, err := c.ListSessions(ctx)

sub, err := c.Subscribe(ctx, info.ID, 1) // from seq 1; use lastSeen+1 to resume
for _, rec := range sub.Replay {         // history, already received
    render(rec)
}
// sub.Info is the session as of the subscribe (state, lastSeq).
for rec := range sub.Events() {          // live, in order, no gaps or duplicates
    render(rec)
}
err = sub.Err() // why Events closed

err = c.Input(ctx, protocol.SessionInput{Session: info.ID, App: "my-app", Text: "hi", Images: []string{"/abs/a.png"}})
err = c.Cancel(ctx, info.ID)
```

`Subscribe` returns once the daemon has sent the whole replay, so the replay/live boundary is the split between `sub.Replay` and `sub.Events()`. Live events are queued without limit, so a slow consumer never blocks replies to its own requests. One subscription per session per client.

## The view

The daemon resolves a session's events into view items (`protocol/schemas/view.schema.json`) that renderers draw. `SubscribeView` returns the snapshot and then a stream of changes:

```go
view, err := c.SubscribeView(ctx, id)  // view.Snapshot: the items as of now
state := view.State()                  // a sdk.ViewState holding the snapshot
for u := range view.Updates() {        // patches, in order
    if err := u.ApplyTo(state); err != nil { /* drifted: resubscribe */ }
    draw(state.Items(), state.Seq)
}
err = view.Err() // why Updates closed
view.Close()     // view.unsubscribe
```

`ViewState` is the piece renderers share: `Reset(snapshot)` and `Apply(patch)` implement the ops (`set` replaces an item or appends a new one, `blocks` replaces `blocks[from:]` of the text component at `path`, `remove` drops an item). Applying never mutates components handed out earlier, so `Items()` results stay valid. A patch that does not fit (unknown item, bad path) is an error and leaves `Seq` alone.

Each `ViewUpdate` is either a `Patch` or a `Snapshot`: with `Options.Reconnect` the subscription asks for a fresh snapshot after a reconnect (a view has no seq to resume from) and delivers it as the first update, which `ApplyTo` handles with `Reset`. One view subscription per session per client, independent of the event `Subscribe`.

```go
theme, err := c.WaitTheme(ctx)  // sent after the handshake; c.Theme() is the latest
c, _ = sdk.Dial(ctx, sdk.Options{OnTheme: func(t protocol.Theme) { /* every theme, in order */ }})

err = c.Action(ctx, id, item.Ref, "photo-editor.open_layer", args) // ui.action for a button
```

`Action` returns once the daemon accepted the request. `copy:` actions are the client's own and are refused. `SubscribeLive` is `Subscribe` without the replay (`Replay` is empty, `Info` says where the log stands), for clients that only watch what happens next.

## Reconnect

Off by default (the CLI wants a lost connection to be an error). With `Options.Reconnect`, the client redials with backoff after the connection drops and then:

- repeats `bus.hello`,
- re-sends the last `SetPresence` (if still open),
- resubscribes every subscription from its last seen seq + 1, dropping anything already delivered, so `Events()` continues without gaps or duplicates. If the daemon refuses (the session is gone), the subscription ends with that error,
- resubscribes every view, which delivers a fresh snapshot as an update (see above).

Requests in flight when the connection drops fail with `ErrConnectionLost` and are not retried (the daemon may have processed them). Requests made while disconnected wait for the new connection until their context ends. `c.Done()` closes only when the client is closed; without `Reconnect` it also closes when the connection is lost (`c.Err()` says which). `Dial` itself never retries.

## Testing

`sdk-go/sdktest` is an in-process fake daemon (sessions, replay/live subscriptions, a scripted agent, a small view resolver with `view.subscribe` / `view.patch` / `ui.action` / `theme` (`PatchView` adds items of your own), presence and connector listing, `agent.focus` / `agent.move` / `connectors.changed` with a home app, request recording) that the SDK's and the CLI's tests share:

```go
d := sdktest.New(t)
d.AddSession("s1", "title", time.Now())
c, _ := sdk.Dial(ctx, sdk.Options{Socket: d.Socket})
```
