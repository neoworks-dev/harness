// Example bus frames. Typed against the generated TS types (so tsc checks them), written to
// protocol/examples/*.json by `make codegen`, and decoded/re-encoded by the Go round-trip test.
// Add one whenever a schema gains a type or field.
import type { Envelope } from "../gen/types.ts";

const session = "s_7k2m9q4x1c";

export const examples: Record<string, Envelope> = {
  "bus-hello": { topic: "bus.hello", payload: { protocol: 1, client: "cli" } },
  "bus-welcome": {
    topic: "bus.welcome",
    payload: { protocol: 1, daemonVersion: "0.1.0", pid: 4242, startedAt: 1760565600000 },
  },
  "bus-ack": { topic: "bus.ack", id: "3" },
  "bus-error": {
    topic: "bus.error",
    id: "4",
    payload: { code: "not_found", message: "no session s_nope" },
  },
  "bus-error-no-id": {
    topic: "bus.error",
    payload: { code: "bad_request", message: "frame is not valid JSON" },
  },
  "session-create": {
    topic: "session.create",
    id: "1",
    payload: { cwd: "/home/user/site", title: "Logo work" },
  },
  "session-create-defaults": { topic: "session.create", id: "1", payload: {} },
  "session-info": {
    topic: "session.info",
    id: "1",
    payload: {
      id: session,
      title: "Logo work",
      cwd: "/home/user/site",
      createdAt: 1760565600000,
      lastActivity: 1760565600000,
      lastSeq: 0,
      state: "idle",
      queued: 0,
    },
  },
  "session-list": { topic: "session.list", id: "2" },
  "session-list-result": {
    topic: "session.list_result",
    id: "2",
    payload: {
      sessions: [
        {
          id: session,
          cwd: "/home/user/site",
          createdAt: 1760565600000,
          lastActivity: 1760565712345,
          lastSeq: 17,
          state: "running",
          queued: 1,
        },
      ],
    },
  },
  "session-subscribe": {
    topic: "session.subscribe",
    id: "5",
    payload: { session, fromSeq: 1 },
  },
  "session-subscribed": {
    topic: "session.subscribed",
    id: "5",
    payload: {
      session: {
        id: session,
        cwd: "/home/user/site",
        createdAt: 1760565600000,
        lastActivity: 1760565712345,
        lastSeq: 17,
        state: "idle",
        queued: 0,
      },
      replayedThrough: 17,
    },
  },
  "session-unsubscribe": { topic: "session.unsubscribe", id: "6", payload: { session } },
  "session-cancel": { topic: "session.cancel", id: "7", payload: { session } },
  "session-input": {
    topic: "session.input",
    id: "8",
    payload: { session, app: "cli", text: "Create hello.txt" },
  },
  "app-presence": {
    topic: "app.presence",
    id: "9",
    payload: {
      app: "browser",
      kind: "app",
      title: "Browser",
      open: true,
      context: { url: "https://example.com" },
      mcp: { url: "http://127.0.0.1:7841/mcp", headers: { authorization: "Bearer t0k3n" } },
      launch: { command: ["browser", "--new-window"], cwd: "/home/user" },
    },
  },
  "app-presence-closing": {
    topic: "app.presence",
    payload: { app: "lsp", kind: "service", open: false },
  },
  "connectors-list": { topic: "connectors.list", id: "10" },
  "connectors-list-result": {
    topic: "connectors.list_result",
    id: "10",
    payload: {
      connectors: [
        {
          app: "browser",
          kind: "app",
          title: "Browser",
          connected: true,
          v: 2,
          tools: [
            { name: "browser.download", description: "Download a URL into the asset store." },
          ],
        },
        {
          app: "photo-editor",
          kind: "app",
          connected: false,
          v: 3,
          tools: [],
          error: "connection refused",
        },
      ],
    },
  },
  "session-input-images": {
    topic: "session.input",
    id: "11",
    payload: { session, app: "overlay", text: "What is this?", images: ["/tmp/shot-1.png"] },
  },
  "agent-focus": {
    topic: "agent.focus",
    payload: { session, app: "pad", from: "overlay", reason: "user" },
  },
  "agent-focus-nowhere": { topic: "agent.focus", payload: { session, reason: "initial" } },
  "agent-move": { topic: "agent.move", id: "12", payload: { app: "pad" } },
  "agent-move-home-new-session": { topic: "agent.move", id: "13", payload: { session } },
  "connectors-changed": {
    topic: "connectors.changed",
    payload: {
      connectors: [
        {
          app: "overlay",
          kind: "app",
          title: "Desktop overlay",
          connected: true,
          home: true,
          pid: 4711,
          context: { monitor: "DP-1" },
          v: 4,
          tools: [{ name: "overlay.point", description: "Point at something on screen." }],
        },
      ],
    },
  },
  "app-presence-home": {
    topic: "app.presence",
    id: "14",
    payload: { app: "overlay", kind: "app", open: true, home: true, pid: 4711 },
  },
  "event-user-message": {
    topic: "session.events",
    payload: {
      session,
      seq: 1,
      ts: 1760565600100,
      event: { type: "user_message", app: "cli", text: "Create hello.txt" },
    },
  },
  "event-user-message-images": {
    topic: "session.events",
    payload: {
      session,
      seq: 12,
      ts: 1760565601100,
      event: { type: "user_message", app: "overlay", text: "This?", images: ["/tmp/shot-1.png"] },
    },
  },
  "event-agent-thought-chunk": {
    topic: "session.events",
    payload: {
      session,
      seq: 2,
      ts: 1760565600200,
      event: { type: "agent_thought_chunk", messageId: "m1", text: "The user wants a file." },
    },
  },
  "event-agent-message-chunk": {
    topic: "session.events",
    payload: {
      session,
      seq: 3,
      ts: 1760565600300,
      event: { type: "agent_message_chunk", messageId: "m1", text: "Creating it now." },
    },
  },
  "event-tool-call": {
    topic: "session.events",
    payload: {
      session,
      seq: 4,
      ts: 1760565600400,
      event: {
        type: "tool_call",
        toolCallId: "call_1",
        title: "write hello.txt",
        kind: "edit",
        status: "pending",
        rawInput: { path: "hello.txt", content: "hi\n" },
      },
    },
  },
  "event-tool-call-update": {
    topic: "session.events",
    payload: {
      session,
      seq: 5,
      ts: 1760565600500,
      event: {
        type: "tool_call_update",
        toolCallId: "call_1",
        status: "completed",
        content: [
          { type: "text", text: "Wrote 3 bytes to hello.txt" },
          { type: "diff", path: "/home/user/site/hello.txt", newText: "hi\n" },
        ],
        rawOutput: { bytes: 3 },
      },
    },
  },
  "event-tool-call-update-minimal": {
    topic: "session.events",
    payload: {
      session,
      seq: 6,
      ts: 1760565600550,
      event: { type: "tool_call_update", toolCallId: "call_1", status: "in_progress" },
    },
  },
  "event-turn-ended": {
    topic: "session.events",
    payload: {
      session,
      seq: 7,
      ts: 1760565600600,
      event: { type: "turn_ended", stopReason: "error", error: "rate limited" },
    },
  },
  "event-app-connected": {
    topic: "session.events",
    payload: {
      session,
      seq: 8,
      ts: 1760565600700,
      event: {
        type: "app_connected",
        app: "lsp",
        kind: "service",
        v: 1,
        tools: ["diagnostics", "definition", "references", "rename"],
      },
    },
  },
  "event-app-disconnected": {
    topic: "session.events",
    payload: {
      session,
      seq: 9,
      ts: 1760565600800,
      event: { type: "app_disconnected", app: "photo-editor" },
    },
  },
  "event-focus-changed": {
    topic: "session.events",
    payload: {
      session,
      seq: 10,
      ts: 1760565600900,
      event: { type: "focus_changed", app: "code-editor", context: { project: "~/site" } },
    },
  },
  "event-harness-message": {
    topic: "session.events",
    payload: {
      session,
      seq: 11,
      ts: 1760565601000,
      event: {
        type: "harness_message",
        harness: "pi",
        message: { role: "user", content: "Create hello.txt", timestamp: 1760565600100 },
      },
    },
  },
};
