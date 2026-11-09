// neo's agents on the neoshell desktop. This process is the widget's data source: it follows the
// daemon's session list and serves it to the shell over D-Bus (dbus.ts), and opens a session in
// neo-term when the widget asks. D-Bus starts it when the shell first needs it (the .service file
// `task shell-widget:install` writes), and it waits for the daemon if that isn't up yet.
import { connect } from "@neoworks-dev/neo-sdk";
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { exportWidgetSource } from "./dbus.ts";
import { openInTerm } from "./open.ts";
import { summarize } from "./summary.ts";

export const DBUS_NAME = "dev.neoworks.Neo.Agents";
// The day's totals roll over at midnight even when no session changes.
const REFRESH_MS = 60_000;

let sessions: SessionInfo[] = [];
let connected = false;

const source = await exportWidgetSource(DBUS_NAME, {
  open: (args) => open(args),
});

publish();

const neo = await connect({ client: "shell-widget", waitForDaemon: true });
neo.state.on((state) => {
  connected = state === "open";
  publish();
});
neo.sessions.on((list) => {
  sessions = list;
  publish();
});
connected = true;
sessions = await neo.listSessions();
publish();
setInterval(publish, REFRESH_MS);

function publish(): void {
  const summary = summarize(sessions, new Date(), connected);
  source.publish("agents", summary.agents);
  source.publish("usage", summary.usage);
}

function open(args: unknown): { ok: boolean } {
  const id =
    typeof args === "object" && args !== null ? (args as { session?: unknown }).session : undefined;
  const session = sessions.find((known) => known.id === id);
  if (session === undefined) return { ok: false };
  openInTerm(session.id, session.cwd);
  return { ok: true };
}
