// Which session the window opens and the name it registers under. Plain functions, so they are
// tested without a window (apps/gui/test).
import type { AgentFocus, ConnectorInfo, SessionInfo } from "@neoworks-dev/neo-protocol/types";

export interface StartupArgs {
  /** An exact session id or a unique prefix. */
  session?: string;
  /** Start a new session in `cwd`. */
  new?: boolean;
}

export type SessionChoice = { existing: string } | { new: true } | { error: string };

function sameDir(a: string, b: string): boolean {
  return a.replace(/\/+$/, "") === b.replace(/\/+$/, "");
}

/**
 * `session` first (an exact id or a unique prefix), then `new`, else the agent's current session
 * when it works in `cwd`, else the most recently active session in `cwd`, else a new one.
 */
export function chooseSession(
  sessions: readonly SessionInfo[],
  focus: AgentFocus | undefined,
  cwd: string,
  args: StartupArgs,
): SessionChoice {
  const wanted = args.session;
  if (wanted !== undefined) {
    if (sessions.some((s) => s.id === wanted)) return { existing: wanted };
    const hits = sessions.filter((s) => s.id.startsWith(wanted));
    const [only] = hits;
    if (hits.length === 1 && only) return { existing: only.id };
    if (hits.length === 0) return { error: `no session "${wanted}"` };
    return { error: `"${wanted}" matches ${hits.length} sessions: be more specific` };
  }
  if (args.new === true) return { new: true };
  const current = sessions.find((s) => s.id === focus?.session);
  if (current && sameDir(current.cwd, cwd)) return { existing: current.id };
  let latest: SessionInfo | undefined;
  for (const session of sessions) {
    if (!sameDir(session.cwd, cwd)) continue;
    if (!latest || session.lastActivity > latest.lastActivity) latest = session;
  }
  if (latest) return { existing: latest.id };
  return { new: true };
}

/**
 * `desktop`, or `desktop-2`… when another window is already connected under that name
 * (registering a taken name would replace the other one).
 */
export function pickAppId(connectors: readonly ConnectorInfo[]): string {
  const taken = (name: string): boolean => connectors.some((c) => c.app === name && c.connected);
  if (!taken("desktop")) return "desktop";
  let n = 2;
  while (taken(`desktop-${n}`)) n++;
  return `desktop-${n}`;
}
