// The icon of each kind of work: phosphor's counterparts of the icons t3code draws.
import brain from "@phosphor-icons/core/regular/brain.svg";
import checkCircle from "@phosphor-icons/core/regular/check-circle.svg";
import chat from "@phosphor-icons/core/regular/chat-circle.svg";
import eye from "@phosphor-icons/core/regular/eye.svg";
import globe from "@phosphor-icons/core/regular/globe.svg";
import search from "@phosphor-icons/core/regular/magnifying-glass.svg";
import pencil from "@phosphor-icons/core/regular/note-pencil.svg";
import robot from "@phosphor-icons/core/regular/robot.svg";
import terminal from "@phosphor-icons/core/regular/terminal.svg";
import wrench from "@phosphor-icons/core/regular/wrench.svg";
import type { WorkKind } from "./work.ts";

const ICONS: Record<WorkKind, string> = {
  command: terminal,
  read: eye,
  edit: pencil,
  search,
  web: globe,
  plan: checkCircle,
  agent: robot,
  tool: wrench,
  thought: brain,
  question: chat,
};

/** SVG markup of the icon for a kind of work. */
export function iconOf(kind: WorkKind): string {
  return ICONS[kind];
}
