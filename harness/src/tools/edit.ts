import { readFile, writeFile } from "node:fs/promises";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { Type } from "typebox";
import { type AssetResolver, resolvePath } from "./paths.ts";
import type { FileChangeDetails } from "./write.ts";

const parameters = Type.Object({
  path: Type.String({
    description: "File to edit: absolute, relative to the working directory, or starting with ~.",
  }),
  edits: Type.Array(
    Type.Object({
      oldText: Type.String({
        description:
          "Exact text to replace, including whitespace. Must occur exactly once in the original " +
          "file; include enough surrounding lines to make it unique.",
      }),
      newText: Type.String({ description: "Replacement text." }),
    }),
    { description: "Replacements. All are matched against the original file, not one by one." },
  ),
});

const BOM = "\uFEFF";

interface Match {
  index: number;
  end: number;
  newText: string;
}

function countOccurrences(text: string, needle: string): number {
  let count = 0;
  for (let at = text.indexOf(needle); at !== -1; at = text.indexOf(needle, at + 1)) count++;
  return count;
}

export function createEditTool(
  cwd: string,
  assets?: AssetResolver,
): AgentTool<typeof parameters, FileChangeDetails> {
  return {
    name: "edit",
    label: "edit",
    description:
      "Edit a file with exact text replacements. Every oldText must match exactly one place in " +
      "the original file and the replaced regions must not overlap; all edits are applied " +
      "together. Prefer one call with several edits over several calls. Line endings (CRLF) " +
      "and a UTF-8 BOM are preserved.",
    parameters,
    async execute(_toolCallId, { path, edits }) {
      if (edits.length === 0) throw new Error("edits is empty.");
      const raw = await readFile(resolvePath(cwd, path, assets), "utf8");
      const hasBom = raw.startsWith(BOM);
      const crlf = raw.includes("\r\n");
      const original = raw.slice(hasBom ? 1 : 0).replaceAll("\r\n", "\n");

      const matches: Match[] = [];
      edits.forEach((edit, i) => {
        const label = `edits[${i}]`;
        const oldText = edit.oldText.replaceAll("\r\n", "\n");
        if (oldText === "") throw new Error(`${label}: oldText is empty.`);
        const count = countOccurrences(original, oldText);
        if (count === 0) throw new Error(`${label}: oldText was not found in ${path}.`);
        if (count > 1) {
          throw new Error(
            `${label}: oldText matches ${count} places in ${path}; add surrounding context so it matches once.`,
          );
        }
        const index = original.indexOf(oldText);
        const newText = edit.newText.replaceAll("\r\n", "\n");
        matches.push({ index, end: index + oldText.length, newText });
      });

      const ordered = matches
        .map((match, i) => ({ ...match, i }))
        .sort((a, b) => a.index - b.index);
      for (let k = 1; k < ordered.length; k++) {
        const prev = ordered[k - 1];
        const next = ordered[k];
        if (prev && next && next.index < prev.end) {
          throw new Error(
            `edits[${prev.i}] and edits[${next.i}] overlap; merge them into one edit.`,
          );
        }
      }

      let updated = original;
      for (const match of ordered.reverse()) {
        updated = updated.slice(0, match.index) + match.newText + updated.slice(match.end);
      }
      if (updated === original) throw new Error(`The edits make no change to ${path}.`);

      const eol = crlf ? updated.replaceAll("\n", "\r\n") : updated;
      const absolute = resolvePath(cwd, path, assets);
      await writeFile(absolute, hasBom ? BOM + eol : eol);
      return {
        content: [{ type: "text", text: `Applied ${edits.length} edit(s) to ${path}.` }],
        details: { path: absolute, oldText: original, newText: updated },
      };
    },
  };
}
