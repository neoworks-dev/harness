// Old/new text → numbered diff lines with hunk headers, so renderers never diff.
import { structuredPatch } from "diff";
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";

const CONTEXT = 3;

export function diffLines(oldText: string | undefined, newText: string): DiffLine[] {
  const patch = structuredPatch("a", "b", oldText ?? "", newText, "", "", { context: CONTEXT });
  const lines: DiffLine[] = [];
  for (const hunk of patch.hunks) {
    lines.push({
      kind: "hunk",
      text: `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@`,
    });
    let oldLine = hunk.oldStart;
    let newLine = hunk.newStart;
    for (const raw of hunk.lines) {
      const text = raw.slice(1);
      if (raw.startsWith("+")) lines.push({ kind: "add", text, new: newLine++ });
      else if (raw.startsWith("-")) lines.push({ kind: "remove", text, old: oldLine++ });
      else if (raw.startsWith(" "))
        lines.push({ kind: "context", text, old: oldLine++, new: newLine++ });
      // "\ No newline at end of file" carries no content.
    }
  }
  return lines;
}
