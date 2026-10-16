// Asset store: files apps produce (images, downloads, exports), addressed by asset://<id> handles
// that stay valid after the producing app is gone. Content-addressed: the id is a hash prefix, so
// storing the same bytes twice returns the same handle. Files are kept read-only under
// <dataDir>/assets as <id>.<ext>, with <id>.json holding the name, mime and producing app.
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, extname, join } from "node:path";
import type { Asset } from "@suite/protocol/types";
import { BadRequestError } from "./focus.ts";
import { NotFoundError } from "./sessions.ts";

export const ASSET_PREFIX = "asset://";
/** Matches handles inside free text (commands, arguments). */
export const ASSET_HANDLE = /asset:\/\/[a-z]+-[0-9a-f]{12}/g;
const ID = /^[a-z]+-[0-9a-f]{12}$/;

const MIME_BY_EXT: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".svg": "image/svg+xml",
  ".bmp": "image/bmp",
  ".ico": "image/x-icon",
  ".pdf": "application/pdf",
  ".json": "application/json",
  ".txt": "text/plain",
  ".md": "text/markdown",
  ".html": "text/html",
  ".css": "text/css",
  ".js": "text/javascript",
  ".zip": "application/zip",
};

const EXT_BY_MIME: Record<string, string> = {
  "image/png": ".png",
  "image/jpeg": ".jpg",
  "image/gif": ".gif",
  "image/webp": ".webp",
  "image/svg+xml": ".svg",
  "image/bmp": ".bmp",
  "application/pdf": ".pdf",
};

function sniff(bytes: Uint8Array): string | undefined {
  const starts = (...sig: number[]): boolean => sig.every((b, i) => bytes[i] === b);
  if (starts(0x89, 0x50, 0x4e, 0x47)) return "image/png";
  if (starts(0xff, 0xd8, 0xff)) return "image/jpeg";
  if (starts(0x47, 0x49, 0x46, 0x38)) return "image/gif";
  if (starts(0x52, 0x49, 0x46, 0x46) && bytes[8] === 0x57 && bytes[9] === 0x45) return "image/webp";
  if (starts(0x25, 0x50, 0x44, 0x46)) return "application/pdf";
  return undefined;
}

function guessMime(bytes: Uint8Array, name: string | undefined): string {
  const sniffed = sniff(bytes);
  if (sniffed !== undefined) return sniffed;
  const byExt = name === undefined ? undefined : MIME_BY_EXT[extname(name).toLowerCase()];
  if (byExt !== undefined) return byExt;
  return "application/octet-stream";
}

interface Meta {
  name: string;
  mime: string;
  app?: string;
  createdAt: number;
  file: string;
}

function parseMeta(text: string): Meta | undefined {
  const value: unknown = JSON.parse(text);
  if (typeof value !== "object" || value === null) return undefined;
  const { name, mime, app, createdAt, file } = value as Record<string, unknown>;
  if (typeof name !== "string" || typeof mime !== "string" || typeof file !== "string") {
    return undefined;
  }
  if (typeof createdAt !== "number") return undefined;
  const meta: Meta = { name, mime, createdAt, file };
  if (typeof app === "string") meta.app = app;
  return meta;
}

export interface PutParams {
  /** Absolute path of a file to copy in. */
  path?: string;
  bytes?: Uint8Array;
  name?: string;
  mime?: string;
  app?: string;
}

export class AssetStore {
  constructor(private readonly dir: string) {
    mkdirSync(dir, { recursive: true, mode: 0o700 });
  }

  put(params: PutParams): Asset {
    let bytes = params.bytes;
    if (params.path !== undefined) {
      if (bytes !== undefined) throw new BadRequestError("asset.put takes path or data, not both");
      if (!existsSync(params.path)) throw new BadRequestError(`no file ${params.path}`);
      bytes = readFileSync(params.path);
    }
    if (bytes === undefined) throw new BadRequestError("asset.put needs path or data");
    const name = params.name ?? (params.path === undefined ? undefined : basename(params.path));
    const mime = params.mime ?? guessMime(bytes, name);
    const hash = createHash("sha256").update(bytes).digest("hex").slice(0, 12);
    const id = `${mime.startsWith("image/") ? "img" : "file"}-${hash}`;
    const existing = this.get(id);
    if (existing) return existing;

    const ext = EXT_BY_MIME[mime] ?? extname(name ?? "").toLowerCase();
    const file = `${id}${ext}`;
    const meta: Meta = { name: name ?? file, mime, createdAt: Date.now(), file };
    if (params.app !== undefined) meta.app = params.app;
    const path = join(this.dir, file);
    writeFileSync(path, bytes, { mode: 0o400 });
    chmodSync(path, 0o400);
    writeFileSync(join(this.dir, `${id}.json`), JSON.stringify(meta), { mode: 0o600 });
    return this.info(id, meta, bytes.byteLength);
  }

  /** Looks up a handle (asset://id) or a bare id. */
  get(handle: string): Asset | undefined {
    const id = handle.startsWith(ASSET_PREFIX) ? handle.slice(ASSET_PREFIX.length) : handle;
    if (!ID.test(id)) return undefined;
    const metaPath = join(this.dir, `${id}.json`);
    if (!existsSync(metaPath)) return undefined;
    const meta = parseMeta(readFileSync(metaPath, "utf8"));
    if (!meta) return undefined;
    const path = join(this.dir, meta.file);
    if (!existsSync(path)) return undefined;
    return this.info(id, meta, Bun.file(path).size);
  }

  require(handle: string): Asset {
    const asset = this.get(handle);
    if (!asset) throw new NotFoundError(`no asset ${handle}`);
    return asset;
  }

  /** The stored file's path, for the agent's tools. */
  resolve(handle: string): string | undefined {
    return this.get(handle)?.path;
  }

  private info(id: string, meta: Meta, size: number): Asset {
    const asset: Asset = {
      handle: `${ASSET_PREFIX}${id}`,
      path: join(this.dir, meta.file),
      name: meta.name,
      mime: meta.mime,
      size,
      createdAt: meta.createdAt,
    };
    if (meta.app !== undefined) asset.app = meta.app;
    return asset;
  }
}
