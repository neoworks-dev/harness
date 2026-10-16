import type { Palette } from "../../lib/palette.ts";
import type { CheckState, State } from "./github.ts";

/** Open is green, closed red, merged the accent, a draft muted. */
export function stateColor(state: State, p: Palette): string {
  switch (state) {
    case "open":
      return p.success;
    case "closed":
      return p.danger;
    case "merged":
      return p.accent;
    case "draft":
      return p.muted;
  }
}

export function checkColor(state: CheckState, p: Palette): string {
  switch (state) {
    case "pass":
      return p.success;
    case "fail":
      return p.danger;
    case "pending":
      return p.accent;
    case "skipped":
      return p.muted;
  }
}

export function checkMark(state: CheckState): string {
  switch (state) {
    case "pass":
      return "✓";
    case "fail":
      return "✗";
    case "pending":
      return "…";
    case "skipped":
      return "–";
  }
}
