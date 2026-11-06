// Appended to the own system prompt of Claude Code and Codex (never replacing it: it explains
// their own tools and is what makes them good at coding). neo's core prompt, the same every agent
// gets, the role, and where neo's tools are for them.
import { CORE_PROMPT, GUEST_ROLE, MAIN_ROLE } from "@neoworks-dev/neo/system-prompt";

/** neo's tools reach a guest as an MCP server's, and their handles name the files behind them. */
const THROUGH_MCP = `neo's tools named above come from its "neo" MCP server. A result with an asset:// handle
names the file behind it, so your own tools can read or copy it.`;

/** For a guest: another agent or the user handed it a task. */
export const GUEST_PROMPT = `${CORE_PROMPT}\n\n${GUEST_ROLE}\n\n${THROUGH_MCP}`;

/** For the agent the user runs the session on. */
export const MAIN_PROMPT = `${CORE_PROMPT}\n\n${MAIN_ROLE}\n\n${THROUGH_MCP}`;
