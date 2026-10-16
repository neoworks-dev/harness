# sdk-rs

Minimal Rust client for the bus (crate `suite-sdk`), for GPUI apps. One background thread owns a blocking WebSocket on the daemon's Unix socket; there is no async runtime, so it fits GPUI's own executor.

- **Requests are blocking** (10 s timeout): call them from a background task. `list_sessions`, `create_session`, `subscribe`, `unsubscribe`, `input`, `cancel`, `move_agent`, `set_presence`, `list_connectors`, `subscribe_view`, `unsubscribe_view`, `subscribe_agent_view`, `unsubscribe_agent_view` (the view of one agent's work in a session: its snapshot and patches have `agent` set, the session's own none), `action`, `put_asset`, `get_asset`; `file_changed` sends without waiting.
- **Pushes arrive on `events()`**, an `async_channel::Receiver<Event>` a task can `.recv().await`: `Focus` (`agent.focus`), `Connectors` (`connectors.changed`), `Session` (replayed and live session events), `Subscribed` (replay finished), `Connection(bool)`, `Theme`, `ViewSnapshot`/`ViewPatch` (see `ViewState`) and `FileChanged`.
- **`reconnect: true`** redials with backoff, repeats hello and the current presence, and resubscribes from the last seq seen + 1, so a session's events arrive in order with no gaps or repeats. Requests in flight when the connection drops fail with `Error::Disconnected` and are not retried (the daemon may have run them). The daemon re-sends `agent.focus` after each hello.
- Events queue without bound until read, so either read them or don't call `events()`.

```rust
use suite_sdk::{Client, Event, Options};

let mut options = Options::new("my-app");
options.reconnect = true;
let client = Client::connect(options)?; // Error::DaemonDown if nothing listens

let events = client.events();
let session = client.list_sessions()?.remove(0);
client.subscribe(&session.id, 1)?; // replay from seq 1, then live
client.input(&session.id, "my-app", "hello", &[])?;

// In a GPUI task: loop { match events.recv().await { ... } }
while let Ok(event) = events.recv_blocking() {
    match event {
        Event::Session(record) => println!("{} {:?}", record.seq, record.event),
        Event::Focus(focus) if focus.app.as_deref() == Some("my-app") => { /* open the pane */ }
        _ => {}
    }
}
```

Types come from `suite_protocol` (re-exported as `suite_sdk::protocol`).

## Tools

`suite_sdk::tools::ToolServer` is a small MCP server (Streamable HTTP, JSON responses, bearer token) on 127.0.0.1, so an app can bring its own tools without a large MCP crate. Handlers run on the server's threads; send to your UI thread through a channel when a tool needs app state.

```rust
use suite_sdk::tools::{Tool, ToolResult, ToolServer};

let open = Tool::new("open", "Open a file", json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    |args| ToolResult::structured("opened", json!({"path": args["path"]})));
let server = ToolServer::start("code-editor", None, vec![open])?;
presence.mcp = Some(server.endpoint()); // keep `server` alive as long as the app runs
```

## Tests

`cargo test -p suite-sdk`: `tests/fake_daemon.rs` runs against a scripted daemon (framing, routing, reconnect); `tests/daemon.rs` starts the real daemon (`bun run daemon/src/main.ts`, echo agent) and is skipped with a message if `bun` is missing.

`client.set_context(app, context)` updates only the presence context (what the app shows now, seen by the agent in `<focus>`), without re-sending the presence: cheap enough for every cursor move, and kept for reconnects.
