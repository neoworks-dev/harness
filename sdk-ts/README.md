# sdk-ts

TS client for the suite bus (`@suite/sdk`): connection, presence, tool hosting, session attach. Bun only.

Bun's WebSocket can't dial Unix sockets, so the client uses the daemon's loopback listener. It finds the URL and token in `daemon.json` next to the socket (`SUITE_SOCKET`, else `$XDG_RUNTIME_DIR/suite/bus.sock`, else `/tmp/suite-<uid>/bus.sock`) and re-reads it on every reconnect, since both change on each daemon start.

```ts
import { SuiteClient } from "@suite/sdk";
import { createToolServer } from "@suite/sdk/tools";

const tools = createToolServer({
  name: "notes",
  instructions: "Notes are plain text; list before you append.",
  tools: [
    {
      name: "append",
      description: "Append a line to the notes",
      inputSchema: { type: "object", properties: { line: { type: "string" } }, required: ["line"] },
      handler: async ({ line }) => `added: ${String(line)}`,
    },
  ],
});

const suite = await SuiteClient.connect({
  client: "notes",
  waitForDaemon: true, // keep retrying if the daemon isn't up yet
  presence: { app: "notes", kind: "app", open: true, mcp: tools.endpoint },
});

const [session = await suite.createSession()] = await suite.listSessions();
suite.subscribe(session.id, {
  fromSeq: session.lastSeq + 1, // skip history; omit to replay it
  onEvent: (event) => {
    if (event.type === "agent_message_chunk") process.stdout.write(event.text);
  },
});
await suite.input(session.id, "What is in my notes?");

await suite.close(); // sends presence open:false first
await tools.close();
```

## `SuiteClient`

- `SuiteClient.connect({ client, presence?, reconnect = true, waitForDaemon = false, env?, onError? })`. Rejects if the daemon isn't running unless `waitForDaemon`. `reconnect` governs what happens after a connection drops.
- Reconnects with backoff (250 ms, doubling, cap 5 s), re-sends hello and the current presence, and resumes each subscription from the last seq it delivered, so subscribers see no gaps or duplicates. Requests in flight when the connection drops reject with `DisconnectedError`; requests while down reject too.
- `onConnectionChange(cb)` is called with `true` once a (re)connect is fully set up and `false` when it drops. `isConnected` is the current state.
- `setPresence(presence)` is sent now and on every reconnect.
- `listSessions()`, `createSession({ cwd?, title? })`, `input(session, text, { app?, images? })`, `cancel(session)`, `listConnectors()`.
- `subscribe(session, { fromSeq?, onEvent, onLive?, onError? })` returns `{ close() }`. `onLive` fires when the replay is over (again after each reconnect).
- `close()` unregisters the presence, then closes for good.
- Daemon-side failures reject with `SuiteError` (`code` from the protocol); an unavailable connection with `DisconnectedError`.

## `createToolServer` (`@suite/sdk/tools`)

An MCP Streamable HTTP server on `127.0.0.1` (random port) behind a random bearer token. Returns `{ url, headers, endpoint, setTools(tools), close() }`; pass `endpoint` as `presence.mcp`. A tool is `{ name, description, inputSchema, outputSchema?, handler(args) }`; the handler returns a string or `{ content?, structuredContent?, isError? }`, and a thrown error becomes an `isError` text result. `setTools` swaps the list and sends `tools/list_changed`.
