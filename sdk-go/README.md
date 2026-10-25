# sdk-go

Go client for neo bus: connect, register as a connector (presence), list connectors, create, follow and drive sessions, and follow the resolved view of a session (items, theme, button actions). It has the same shape as `sdk-ts`. The only dependency besides the generated protocol types is `github.com/coder/websocket`.

```go
import (
    protocol "neoworks.dev/neo/protocol/go"
    sdk "neoworks.dev/neo/sdk-go"
)
```

Consumers outside this workspace need `replace neoworks.dev/neo/protocol/go => ../protocol/go` (and the same for `sdk-go`) in their `go.mod`, like `cli/go.mod`.

## Connect

```go
c, err := sdk.Dial(ctx, sdk.Options{Client: "my-app", Reconnect: true})
if errors.Is(err, sdk.ErrDaemonDown) { /* nothing listening on the socket */ }
defer c.Close()
```

The socket is `Options.Socket`, else `$NEO_SOCKET`, else `$XDG_RUNTIME_DIR/neo/bus.sock`, else `/tmp/neo-<uid>/bus.sock` (`sdk.SocketPath`). `Dial` does the WebSocket upgrade at `/bus` and the hello/welcome exchange; `c.Welcome()` has the daemon info.

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

### Tools: `sdk-go/tools`

`tools.Start` hosts the app's tools as an MCP server, the counterpart of sdk-ts's `createToolServer`. It is a small hand-written Streamable HTTP server on `net/http` (no dependency): stateless, plain `application/json` responses, bound to 127.0.0.1 on a free port, with a random bearer token that the presence carries in `Mcp.Headers` (the gateway sends them with every request). It answers `initialize` (echoing a protocol version it knows), `ping`, `notifications/*` (202), `tools/list` and `tools/call`; protocol errors are JSON-RPC errors, a failing handler is an `isError` result.

```go
srv, err := tools.Start(tools.Options{
    Name:         "photo-editor",
    Instructions: "Edits one image at a time…", // shown by tools_search next to the tools
    Tools: []tools.Tool{{
        Name:        "crop",
        Description: "Cut out a rectangle.",
        InputSchema: map[string]any{ // "type": "object" is filled in
            "properties": map[string]any{"width": map[string]any{"type": "integer"}},
            "required":   []string{"width"},
        },
        OutputSchema: map[string]any{"properties": map[string]any{"handle": map[string]any{"type": "string"}}},
        Handler: func(ctx context.Context, args map[string]any) (tools.Result, error) {
            // args is the arguments object (numbers are float64). A returned error becomes an isError result.
            return tools.Result{Text: "Cropped", Structured: map[string]any{"handle": "asset://img-…"}}, nil
        },
    }},
})
defer srv.Close()
err = c.SetPresence(ctx, protocol.AppPresence{App: "photo-editor", Kind: protocol.ConnectorKindApp, Open: true, Mcp: srv.Endpoint()})
```

`Result.Structured` becomes `structuredContent` (the data view templates get); without `Text` its JSON is the text content. The tool list is fixed once the server started (there is no server-to-client stream, so no `list_changed`); a changed list needs a new server and a new `SetPresence`. Tests can use `sdktest.StartReal(t)`, which runs the real TypeScript daemon (`NEO_AGENT=echo`, temp socket and data dir; skips without `bun`) and calls tools through its `/mcp` with `CallTool("app.tool", args)`.

## Assets and file changes

```go
a, err := c.PutAsset(ctx, protocol.AssetPut{Data: &base64Bytes, Name: &name}) // or Path: absolute path to copy in
a.Handle // asset://img-0123456789ab: content-addressed, valid after this app disconnects
a.Path   // the stored file: read-only, same machine; copy it before editing
a, err = c.GetAsset(ctx, "asset://img-0123456789ab") // *sdk.Error with code not_found if unknown

err = c.FileChanged(ctx, "/abs/path/logo.png")        // after saving a file: no reply, not echoed back
c, _ = sdk.Dial(ctx, sdk.Options{OnFileChanged: func(f protocol.FileChanged) { /* f.Path, f.App ("agent" or an app), f.Session */ }})
```

The daemon records `Options.Client` as the asset's app and as the source of a `file.changed`, so give the client the app's name.

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

`sdk-go/sdktest` is an in-process fake daemon (sessions, replay/live subscriptions, a scripted agent, a small view resolver with `view.subscribe` / `view.patch` / `ui.action` / `theme` (`PatchView` adds items of your own), presence and connector listing, `agent.focus` / `agent.move` / `connectors.changed` with a home app, request recording, `asset.put` / `asset.get` with files in a temp dir, `file.changed` with `FileChanges` / `SendFileChanged`) that the SDK's and the CLI's tests share:

```go
d := sdktest.New(t)
d.AddSession("s1", "title", time.Now())
c, _ := sdk.Dial(ctx, sdk.Options{Socket: d.Socket})
```

`c.SetContext(ctx, map[string]string{...})` updates only the presence context (what the app shows now, seen by the agent in `<focus>`), without re-sending the presence: cheap enough for every cursor move, and kept for reconnects.
