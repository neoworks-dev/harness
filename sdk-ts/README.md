# @neoworks-dev/neo-sdk

Connect an app to the neo harness. With it an app can:

- offer tools to the agent, and tell the agent what it is showing;
- create sessions, send them input, and follow what the agent does;
- draw the chat the way the daemon resolved it, from a browser or a terminal;
- follow where the agent is, which sessions exist, and which apps are connected;
- survive a daemon restart without losing its place.

It runs in Node, Bun and browsers.

## Install and import

```sh
npm install @neoworks-dev/neo-sdk
```

```ts
import { connect, createToolServer, ViewState } from "@neoworks-dev/neo-sdk";
```

Everything comes from the root. In Node and Bun, `connect` finds the daemon on this machine by itself, and `createToolServer` hosts your tools. A browser gets the same package without those two (the `browser` export condition), so it doesn't load Node modules or the MCP SDK.

In this repo, depend on it with `"@neoworks-dev/neo-sdk": "workspace:*"`. To publish: `bun run tools/npm-package.ts sdk-ts`, then `npm publish sdk-ts/dist --access public`.

## An app that offers tools to the agent

```ts
import { connect, createToolServer } from "@neoworks-dev/neo-sdk";

const notes: string[] = [];

const tools = await createToolServer({
  name: "notes",
  instructions: "Notes are plain text; list before you append.",
  tools: [
    {
      name: "append",
      description: "Append a line to the notes",
      inputSchema: { type: "object", properties: { line: { type: "string" } }, required: ["line"] },
      handler: async ({ line }) => {
        notes.push(String(line));
        return `added: ${String(line)}`;
      },
    },
  ],
});

const client = await connect({
  client: "notes",
  waitForDaemon: true, // keep trying if the daemon is not up yet
  presence: { app: "notes", kind: "app", title: "Notes", open: true, mcp: tools.endpoint },
});

// Tell the agent what the user is looking at; cheap enough for every cursor move.
await client.setContext({ file: "/home/me/todo.txt", line: "12" });

// On the way out: marks the app closed, then disconnects.
await client.close();
await tools.close();
```

The daemon lists the tools next to the app's name (`notes.append`) and forwards the agent's calls to your handler. A handler returns a string, or `{ content, structuredContent, isError }`; if it throws, the agent sees the message as an error result. A complete app with result templates and buttons is in [`examples/notes-app.ts`](examples/notes-app.ts).

## Following and driving a session

```ts
import { connect } from "@neoworks-dev/neo-sdk";

const client = await connect({ client: "my-script" });

const session = await client.createSession({ title: "Demo" });
const stopFollowing = client.subscribe(session.id, {
  onEvent: (event) => {
    if (event.type === "agent_message_chunk") process.stdout.write(event.text);
    if (event.type === "turn_ended") console.log(`\n[${event.stopReason}]`);
  },
});

await client.input(session.id, "What is in my notes?");
// ... later
stopFollowing();
await client.close();
```

`subscribe` replays the session from the start (pass `fromSeq` to skip history) and then streams live. Where the agent is and what exists is always at hand:

```ts
client.focus.on((focus) => console.log("the agent is in", focus?.app ?? "no app"));
client.sessions.on((sessions) => console.log(sessions.map((s) => `${s.title}: ${s.state}`)));
const focus = await client.focus.waitFor(); // the current one, waiting for the first if needed
await client.moveAgent({ app: "editor" }); // or { session } to switch the current session
```

## Rendering the chat view

The daemon resolves a session into view items (messages, tool calls, diffs, buttons). `ViewState` keeps the latest snapshot with patches applied and tells you what changed, so a renderer can update its output instead of redrawing it.

```ts
import { ViewState } from "@neoworks-dev/neo-sdk";

const view = new ViewState();
const stop = client.subscribeView(session.id, {
  onSnapshot: (snapshot) => {
    view.applySnapshot(snapshot); // replaces everything, also after a reconnect
    drawAll(view.items);
  },
  onPatch: (patch) => {
    for (const change of view.applyPatch(patch)) drawChange(change); // added, replaced, removed, blocks
  },
});

// A button in the view was pressed:
await client.action(session.id, item.ref, button.action);
```

To follow what one agent did inside a session (a subagent, or a guest the user addressed), pass `agent: "claude"` to `subscribeView` and keep a separate `ViewState` for it.

## Connecting from a browser, or another runtime

On the daemon's machine `connect` reads the address from `daemon.json` (`daemonJsonAddress`). A browser can't read that file, so there, give `connect` the address yourself: the daemon's loopback URL and its token, for example served to the page by a small local server that reads `daemon.json`.

```ts
import { assetUrl, connect, fixedAddress } from "@neoworks-dev/neo-sdk";

const address = { url: "http://127.0.0.1:41234", token };
const client = await connect({ client: "web", address: fixedAddress(address) });

img.src = assetUrl(address, "asset://img-0123456789ab") ?? ""; // images the agent read
```

A restarted daemon has a new port and token, so the client asks for the address again on every reconnect. If your page can fetch a fresh one, pass your own function:

```ts
const client = await connect({
  client: "web",
  address: async () => (await fetch("/daemon")).json(),
});
```

The address carries the daemon token: keep it inside your own page or process.

## Reference

### `connect(options): Promise<Client>`

| Option              | Meaning                                                                                                                    |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `client`            | Your client's name.                                                                                                        |
| `address?`          | `() => Promise<DaemonAddress>`, called on every (re)connect. Default (Node, Bun): the local daemon. Required in a browser. |
| `presence?`         | The app's registration (`AppPresence`), sent on connect and after every reconnect.                                         |
| `reconnect?`        | Reconnect after a drop, with backoff (default `true`).                                                                     |
| `waitForDaemon?`    | Keep trying while the daemon is not running, instead of rejecting (default `false`).                                       |
| `requestTimeoutMs?` | How long to wait for an answer before rejecting with `TimeoutError` (default 10000).                                       |
| `onError?`          | Failures nobody is waiting for: a refused subscription, an error thrown by a callback.                                     |

Rejects with `DisconnectedError` if the daemon cannot be reached, `NeoError` if it refuses the client or the presence.

### `Client`

Requests resolve when the daemon has acted.

| Member                                       | Does                                                                                       |
| -------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `createSession(params?)`                     | Starts a session (`cwd?`, `title?`).                                                       |
| `listSessions()`                             | The daemon's sessions.                                                                     |
| `input(session, text, { app?, images? })`    | Sends user input; resolves once queued.                                                    |
| `cancel(session)`                            | Cancels the running turn.                                                                  |
| `subscribe(session, options)`                | Events: replay from `fromSeq?`, then live. See below.                                      |
| `subscribeView(session, options)`            | The resolved view: snapshot, then patches. See below.                                      |
| `action(session, ref, action, args?)`        | A button was pressed in view item `ref`.                                                   |
| `moveAgent({ app?, session? })`              | Moves the agent to an app (home without `app`) and/or makes a session current.             |
| `setPresence(presence)`                      | Replaces the registration, now and on reconnects.                                          |
| `setContext(context)`                        | Updates what the app shows, without re-sending the presence.                               |
| `listConnectors()`                           | The connected apps and their tools.                                                        |
| `putAsset({ path? \| data?, name?, mime? })` | Stores a file or base64 bytes; resolves with an `Asset` whose `handle` others can use.     |
| `getAsset(handle)`                           | Looks an asset up (`not_found` if unknown).                                                |
| `fetchAsset(url)`                            | Has the daemon download a public web image into the store (`bad_request` if it isn't one). |
| `fileChanged(path)`                          | Tells other clients you changed a file. Throws `DisconnectedError` if offline.             |
| `onFileChanged(callback)`                    | Another client or the agent changed a file (never your own).                               |
| `onSecretRequest(callback)`                  | The daemon asks this client alone for a secret (an API key): show a masked field.          |
| `answerSecret(id, value?)`                   | Answers it; no value means cancelled. Never store or log the value.                        |
| `close()`                                    | Marks the app closed, disconnects for good.                                                |

Everything that listens returns a function that stops it.

`subscribe` options: `onEvent(event, record)`, `onLive?(session)` (the replay is over; fires again after a reconnect), `onError?`, `fromSeq?`. After a reconnect it continues from the last event delivered.

`subscribeView` options: `onSnapshot(snapshot)` (replace your state; fires again after a reconnect), `onPatch(patch)` (never before a snapshot), `onError?`, `agent?`.

Subscribing while disconnected is fine: it starts when the connection opens.

### Watched values

These properties are `Watched<T>`: `.value` is the latest, `.on(callback)` returns an unsubscribe function, `.waitFor(timeoutMs?, accept?)` resolves with the first value (that `accept` approves, if given) and rejects with `TimeoutError` if none arrives.

| Property     | Value                                                                         |
| ------------ | ----------------------------------------------------------------------------- |
| `state`      | `"connecting" \| "open" \| "reconnecting" \| "closed"`                        |
| `focus`      | `AgentFocus \| undefined`: the current session and the app showing the agent. |
| `theme`      | `Theme \| undefined`                                                          |
| `connectors` | `ConnectorInfo[]`, kept current.                                              |
| `sessions`   | `SessionInfo[]`, kept current as sessions change.                             |

### `ViewState`

`applySnapshot(snapshot)`, `applyPatch(patch)` (returns `ViewChange[]`), `items`, `get(ref)`, `reset(items?)`, `session`, `agent`, `seq`. Items are never edited, only replaced, so a renderer can compare them by identity. Also `componentAtPath(component, path)` and `hasChildren(component)`.

### Addresses

`DaemonAddress { url, token }`, `fixedAddress(address)`, `busUrl(address)`, `assetUrl(address, handle)`; in Node and Bun also `daemonJsonAddress(env?)`, `daemonJsonPath(env?)`.

### Errors

- `NeoError`: the daemon said no; `code` is `bad_request`, `not_found`, `unsupported_protocol` or `internal`.
- `DisconnectedError`: there is no connection. Requests waiting when it drops reject with it too.
- `TimeoutError`: no answer in time, or a `waitFor` ran out of time.

### `await createToolServer(options)` (Node and Bun)

`{ name, version?, instructions?, tools, skills? }` where a tool is `{ name, description, inputSchema, outputSchema?, handler(args) }`. Returns `{ url, headers, endpoint, setTools(tools), close() }`: pass `endpoint` as `presence.mcp`; `setTools` swaps the list and tells the daemon.

What agents need to know to work in the app goes in skills, not in the tools' descriptions or the instructions: `skills: [{ name: "layers", description: "Working with layers", text }]`. They are served as MCP prompts without arguments; agents are told each one's name and description when the app connects and load the text (`skills_load`) when the work calls for it.
