// Fetching for the browser app: http(s) only, redirects followed, bodies capped, time limited.

export const MAX_PAGE_BYTES = 2 * 1024 * 1024;
export const MAX_DOWNLOAD_BYTES = 10 * 1024 * 1024;
const TIMEOUT_MS = 20_000;
const USER_AGENT = "Mozilla/5.0 (compatible; SuiteBrowser/0.1; +https://example.invalid/suite)";

export interface Fetched {
  /** The final URL, after redirects. */
  url: string;
  status: number;
  /** Lower-cased media type without parameters, e.g. text/html. */
  type: string;
  charset?: string;
  bytes: Uint8Array;
  /** The body was longer than the limit and was cut (pages only). */
  truncated: boolean;
}

export interface FetchOptions {
  maxBytes: number;
  /** Fail instead of cutting a body that is too long (downloads). */
  strict?: boolean;
  accept?: string;
}

/** An absolute http(s) URL for `input` (relative to `base` if given); throws a readable error. */
export function parseHttpUrl(input: string, base?: string): URL {
  let url: URL;
  try {
    url = new URL(input.trim(), base);
  } catch {
    if (base === undefined) throw new Error(`not a valid URL: ${input}`);
    throw new Error(`not a valid URL: ${input} (relative to ${base})`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error(`only http(s) URLs are supported, not ${url.protocol}`);
  }
  url.hash = "";
  return url;
}

/** What the address bar was given: a bare host like `example.com` gets `https://`. */
export function addressToUrl(input: string): URL {
  const text = input.trim();
  if (text !== "" && !/^[a-z][a-z0-9+.-]*:/i.test(text)) return parseHttpUrl(`https://${text}`);
  return parseHttpUrl(text);
}

async function readBody(
  response: Response,
  { maxBytes, strict }: FetchOptions,
): Promise<{ bytes: Uint8Array; truncated: boolean }> {
  const declared = Number(response.headers.get("content-length"));
  if (strict === true && declared > maxBytes) {
    throw new Error(`too large: ${declared} bytes (limit ${maxBytes})`);
  }
  const reader = response.body?.getReader();
  if (!reader) return { bytes: new Uint8Array(), truncated: false };
  const chunks: Uint8Array[] = [];
  let total = 0;
  let truncated = false;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > maxBytes) {
      if (strict === true) {
        await reader.cancel();
        throw new Error(`too large: more than ${maxBytes} bytes`);
      }
      chunks.push(value.subarray(0, value.length - (total - maxBytes)));
      total = maxBytes;
      truncated = true;
      await reader.cancel();
      break;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return { bytes, truncated };
}

export async function fetchUrl(url: URL, options: FetchOptions): Promise<Fetched> {
  let response: Response;
  try {
    response = await fetch(url, {
      redirect: "follow",
      signal: AbortSignal.timeout(TIMEOUT_MS),
      headers: { "user-agent": USER_AGENT, accept: options.accept ?? "*/*" },
    });
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(`could not fetch ${url.href}: ${why}`);
  }
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(`${url.href} answered ${response.status} ${response.statusText}`.trim());
  }
  const finalUrl = parseHttpUrl(response.url === "" ? url.href : response.url);
  const header = response.headers.get("content-type") ?? "";
  const [type = "", ...params] = header.split(";");
  const charset = params
    .map((param) => /^\s*charset\s*=\s*"?([^";\s]+)/i.exec(param)?.[1])
    .find((value) => value !== undefined);
  const { bytes, truncated } = await readBody(response, options);
  return {
    url: finalUrl.href,
    status: response.status,
    type: type.trim().toLowerCase(),
    charset,
    bytes,
    truncated,
  };
}

/** The body as text, in the charset the server named if the runtime knows it, else UTF-8. */
export function decodeBody(fetched: Pick<Fetched, "bytes" | "charset">): string {
  try {
    return new TextDecoder(fetched.charset ?? "utf-8").decode(fetched.bytes);
  } catch {
    return new TextDecoder().decode(fetched.bytes);
  }
}

/** A file name for a downloaded URL: the last path segment, else the host. */
export function nameFromUrl(url: string, type: string): string {
  const parsed = new URL(url);
  const last = parsed.pathname
    .split("/")
    .filter((part) => part !== "")
    .at(-1);
  let name = parsed.hostname;
  if (last !== undefined) {
    try {
      name = decodeURIComponent(last);
    } catch {
      name = last;
    }
  }
  name = name.replace(/[\\/\0]/g, "_").slice(0, 100);
  if (/\.[a-z0-9]{1,5}$/i.test(name)) return name;
  const ext = EXTENSIONS[type];
  return ext === undefined ? name : `${name}${ext}`;
}

const EXTENSIONS: Record<string, string> = {
  "image/png": ".png",
  "image/jpeg": ".jpg",
  "image/gif": ".gif",
  "image/webp": ".webp",
  "image/svg+xml": ".svg",
  "application/pdf": ".pdf",
  "text/html": ".html",
  "text/plain": ".txt",
  "application/json": ".json",
};
