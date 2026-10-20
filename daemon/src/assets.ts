// Asset store: files apps produce (images, downloads, exports), addressed by asset://<id> handles
// that stay valid after the producing app is gone. Content-addressed: the id is a hash prefix, so
// storing the same bytes twice returns the same handle. Files are kept under
// <dataDir>/assets as <id>.<ext>, with <id>.json holding the name, mime and producing app.
// Remote images are downloaded into it on request (fetch), so clients only ever read files.
import { createHash } from "node:crypto";
import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, extname, join } from "node:path";
import type { Asset } from "@suite/protocol/types";
import { BadRequestError } from "./focus.ts";
import { NotFoundError } from "./sessions.ts";

export const ASSET_PREFIX = "asset://";
/** Matches handles inside free text (commands, arguments). */
export const ASSET_HANDLE = /asset:\/\/[a-z]+-[0-9a-f]{12}/g;
const ID = /^[a-z]+-[0-9a-f]{12}$/;

/** A remote image gets this long (less than a client waits for the answer), size and redirects. */
const FETCH_TIMEOUT_MS = 8_000;
const FETCH_MAX_BYTES = 20 * 1024 * 1024;
const FETCH_MAX_REDIRECTS = 5;
/** What a client can draw; anything else is refused before it is stored. */
const REMOTE_IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);
/** Some hosts refuse a request that doesn't say who asks. */
const FETCH_HEADERS = {
  "user-agent": "suite (image download)",
  accept: "image/png,image/jpeg,image/gif,image/webp;q=0.9,*/*;q=0.1",
};

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

function parseRemote(url: string): URL {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new BadRequestError(`not a URL: ${url}`);
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new BadRequestError(`only http and https images are downloaded, not ${url}`);
  }
  return parsed;
}

async function readCapped(res: Response, url: string): Promise<Uint8Array> {
  const length = Number(res.headers.get("content-length") ?? 0);
  if (length > FETCH_MAX_BYTES) throw new BadRequestError(`${url} is larger than 20 MB`);
  const chunks: Uint8Array[] = [];
  let size = 0;
  if (res.body) {
    for await (const chunk of res.body) {
      size += chunk.byteLength;
      if (size > FETCH_MAX_BYTES) throw new BadRequestError(`${url} is larger than 20 MB`);
      chunks.push(chunk);
    }
  }
  return Buffer.concat(chunks);
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export interface PutParams {
  /** Absolute path of a file to copy in. */
  path?: string;
  bytes?: Uint8Array;
  name?: string;
  mime?: string;
  app?: string;
}

export interface StoreOptions {
  /** Lets fetch reach loopback and private addresses; for tests. */
  privateHosts?: boolean;
}

/** An address on this machine or a private network: a URL a model wrote must not reach it. */
export function isPrivateAddress(address: string): boolean {
  const v4 = address.startsWith("::ffff:") ? address.slice(7) : address;
  if (isIP(v4) === 4) {
    const [a = 0, b = 0] = v4.split(".").map(Number);
    return (
      a === 0 ||
      a === 10 ||
      a === 127 ||
      (a === 100 && b >= 64 && b <= 127) ||
      (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168) ||
      a >= 224
    );
  }
  const v6 = address.toLowerCase();
  return (
    v6 === "::" ||
    v6 === "::1" ||
    v6.startsWith("fc") ||
    v6.startsWith("fd") ||
    v6.startsWith("fe8") ||
    v6.startsWith("fe9") ||
    v6.startsWith("fea") ||
    v6.startsWith("feb") ||
    v6.startsWith("ff")
  );
}

export class AssetStore {
  /** Downloaded URLs and their assets; a download in progress is shared. */
  private readonly fetched = new Map<string, Promise<Asset>>();

  constructor(
    private readonly dir: string,
    private readonly options: StoreOptions = {},
  ) {
    mkdirSync(dir, { recursive: true, mode: 0o700 });
  }

  /**
   * Downloads the image at `url` (http or https, a public host) and stores it. The same URL is
   * downloaded once while the daemon runs; a failed download is tried again next time.
   */
  fetch(url: string): Promise<Asset> {
    const known = this.fetched.get(url);
    if (known) return known;
    const download = this.download(url);
    this.fetched.set(url, download);
    download.catch(() => this.fetched.delete(url));
    return download;
  }

  private async download(url: string): Promise<Asset> {
    let target = parseRemote(url);
    const signal = AbortSignal.timeout(FETCH_TIMEOUT_MS);
    for (let hops = 0; ; hops++) {
      await this.checkHost(target);
      let res: Response;
      try {
        res = await fetch(target, { headers: FETCH_HEADERS, redirect: "manual", signal });
      } catch (err) {
        throw new BadRequestError(`can't download ${url}: ${errorMessage(err)}`);
      }
      const location = res.headers.get("location");
      if (res.status >= 300 && res.status < 400 && location !== null) {
        if (hops >= FETCH_MAX_REDIRECTS) throw new BadRequestError(`${url}: too many redirects`);
        target = parseRemote(new URL(location, target).href);
        continue;
      }
      if (!res.ok) throw new BadRequestError(`${url}: HTTP ${res.status}`);
      const bytes = await readCapped(res, url);
      const mime = sniff(bytes);
      if (mime === undefined || !REMOTE_IMAGE_TYPES.has(mime)) {
        throw new BadRequestError(`${url} is not a png, jpeg, gif or webp image`);
      }
      const name = decodeURIComponent(basename(target.pathname)) || target.hostname;
      return this.put({ bytes, name, mime });
    }
  }

  private async checkHost(url: URL): Promise<void> {
    if (this.options.privateHosts) return;
    const host = url.hostname.replace(/^\[|\]$/g, "");
    let addresses = [host];
    if (isIP(host) === 0) {
      try {
        addresses = (await lookup(host, { all: true })).map((a) => a.address);
      } catch (err) {
        throw new BadRequestError(`can't find ${host}: ${errorMessage(err)}`);
      }
    }
    if (addresses.some(isPrivateAddress)) {
      throw new BadRequestError(`${url.hostname} is not a public host`);
    }
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
    // Not read-only on disk: copies made with cp should be ordinary files. The agent's
    // write/edit tools refuse handles instead.
    writeFileSync(path, bytes, { mode: 0o644 });
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
