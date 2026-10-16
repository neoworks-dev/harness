import { readFile } from "node:fs/promises";
import { extname } from "node:path";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import type { ImageContent } from "@mariozechner/pi-ai";
import { Type } from "typebox";
import type { RequestPermission } from "../agent.ts";
import { type AssetResolver, resolvePath } from "./paths.ts";
import { askToProceed, withNote } from "./permission.ts";
import { MAX_BYTES, MAX_LINES, splitLines, truncateHead } from "./truncate.ts";

const IMAGE_TYPES: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
};

/** Reads attached images for a prompt. Unknown types and unreadable files are skipped. */
export async function loadImages(paths: string[]): Promise<ImageContent[]> {
  const images: ImageContent[] = [];
  for (const path of paths) {
    const mimeType = IMAGE_TYPES[extname(path).toLowerCase()];
    if (!mimeType) continue;
    const data = await readFile(path).catch(() => undefined);
    if (data) images.push({ type: "image", data: data.toString("base64"), mimeType });
  }
  return images;
}

const parameters = Type.Object({
  path: Type.String({
    description: "File to read: absolute, relative to the working directory, or starting with ~.",
  }),
  offset: Type.Optional(
    Type.Number({ description: "Line to start reading from (1-based). Default: 1." }),
  ),
  limit: Type.Optional(Type.Number({ description: "Maximum number of lines to read." })),
});

export interface ReadDetails {
  path: string;
  /** Set when the result was cut short. */
  truncated?: boolean;
}

export function createReadTool(
  cwd: string,
  assets?: AssetResolver,
  ask?: RequestPermission,
): AgentTool<typeof parameters, ReadDetails> {
  return {
    name: "read",
    label: "read",
    description:
      "Read a file. Text files are returned as plain lines (no line numbers); png, jpeg, gif " +
      "and webp images are returned as images. Output is cut to 2000 lines or 50KB, whichever " +
      "comes first, and says how to continue with offset. Other binary files can't be read.",
    parameters,
    async execute(toolCallId, { path, offset, limit }, signal) {
      const absolute = resolvePath(cwd, path, assets);
      // The daemon lets ordinary reads through; sensitive files (.env, keys) ask first.
      const { note: added } = await askToProceed(
        ask,
        { title: `read ${path}`, kind: "read", toolCallId, path: absolute },
        "The user declined to let you read this file.",
        signal,
      );
      const buffer = await readFile(absolute);

      const mimeType = IMAGE_TYPES[extname(absolute).toLowerCase()];
      if (mimeType) {
        return {
          content: [
            {
              type: "text",
              text: withNote(`Read image ${path} (${mimeType}, ${buffer.length} bytes).`, added),
            },
            { type: "image", data: buffer.toString("base64"), mimeType },
          ],
          details: { path: absolute },
        };
      }
      if (buffer.subarray(0, 8000).includes(0)) {
        throw new Error(`${path} is a binary file and can't be read as text.`);
      }

      const text = buffer
        .toString("utf8")
        .replace(/^\uFEFF/, "")
        .replaceAll("\r\n", "\n");
      const lines = splitLines(text);
      const start = Math.max(1, Math.floor(offset ?? 1));
      if (start > lines.length && !(start === 1 && lines.length <= 1)) {
        throw new Error(`offset ${start} is beyond the end of ${path} (${lines.length} lines).`);
      }
      const end = limit === undefined ? lines.length : Math.min(lines.length, start - 1 + limit);
      const slice = truncateHead(lines.slice(start - 1, end).join("\n"));

      if (slice.shownLines === 0 && slice.totalLines > 0) {
        const note = `line ${start} is longer than ${MAX_BYTES} bytes`;
        throw new Error(`${path}: ${note}. Use bash (e.g. sed -n '${start}p' | cut -c1-5000).`);
      }
      const lastShown = start - 1 + slice.shownLines;
      let result = slice.text;
      if (lastShown < lines.length) {
        const cutBy = slice.truncated ? `${MAX_LINES} lines / ${MAX_BYTES} bytes` : "limit";
        const note = `Showing lines ${start}-${lastShown} of ${lines.length} (${cutBy}).`;
        result += `\n\n[${note} Use offset=${lastShown + 1} to continue.]`;
      }
      return {
        content: [{ type: "text", text: withNote(result, added) }],
        details: { path: absolute, truncated: lastShown < lines.length },
      };
    },
  };
}
