# sdk-go

Go client for the suite bus: connect, register as a connector (presence), list connectors, and create, follow and drive sessions. It has the same shape as `sdk-ts`. The only dependency besides the generated protocol types is `github.com/coder/websocket`.

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

## Reconnect

Off by default (the CLI wants a lost connection to be an error). With `Options.Reconnect`, the client redials with backoff after the connection drops and then:

- repeats `bus.hello`,
- re-sends the last `SetPresence` (if still open),
- resubscribes every subscription from its last seen seq + 1, dropping anything already delivered, so `Events()` continues without gaps or duplicates. If the daemon refuses (the session is gone), the subscription ends with that error.

Requests in flight when the connection drops fail with `ErrConnectionLost` and are not retried (the daemon may have processed them). Requests made while disconnected wait for the new connection until their context ends. `c.Done()` closes only when the client is closed; without `Reconnect` it also closes when the connection is lost (`c.Err()` says which). `Dial` itself never retries.

## Testing

`sdk-go/sdktest` is an in-process fake daemon (sessions, replay/live subscriptions, a scripted agent, presence and connector listing, request recording) that the SDK's and the CLI's tests share:

```go
d := sdktest.New(t)
d.AddSession("s1", "title", time.Now())
c, _ := sdk.Dial(ctx, sdk.Options{Socket: d.Socket})
```
