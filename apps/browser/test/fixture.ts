// A little web server to browse in tests: an HTML page with images, a PNG, redirects, an
// oversized file and a few odd responses.
import type { Server } from "bun";

/** A real 1x1 PNG. */
export const PNG = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  ),
  (c) => c.charCodeAt(0),
);

export interface Fixture {
  url: string;
  server: Server<undefined>;
  hits: string[];
  stop(): Promise<void>;
}

const PAGE = (origin: string): string => `<!doctype html>
<html>
<head>
<title>Fixture &amp; Co</title>
<meta property="og:image" content="/og.png">
<link rel="icon" href="/favicon.png">
<style>body { color: red }</style>
<script>var secret = "<b>not text</b>";</script>
</head>
<body>
<h1>Hello page</h1>
<p>First paragraph with <b>bold</b> and a <a href="/next">link</a>.</p>
<p>Second&nbsp;paragraph &lt;escaped&gt;.</p>
<img src="/logo.png" alt="The logo">
<img src="rel/pic.png" srcset="/small.png 480w, /large.png 1600w" alt="Responsive">
<img src="/logo.png" alt="Duplicate">
<img src="/pixel.gif" width="1" height="1" alt="tracker">
<img src="data:image/png;base64,AAAA" alt="inline">
<a href="mailto:me@example.com">mail</a>
<a href="${origin}/other">other</a>
<noscript>enable javascript</noscript>
</body>
</html>`;

export function startFixture(): Fixture {
  const hits: string[] = [];
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      hits.push(url.pathname);
      const origin = url.origin;
      switch (url.pathname) {
        case "/":
          return new Response(PAGE(origin), { headers: { "content-type": "text/html" } });
        case "/redirect":
          return Response.redirect(`${origin}/`, 302);
        case "/logo.png":
        case "/og.png":
        case "/large.png":
          return new Response(PNG, { headers: { "content-type": "image/png" } });
        case "/download":
          // Mislabelled: the asset store should sniff the PNG.
          return new Response(PNG, { headers: { "content-type": "application/octet-stream" } });
        case "/notes.txt":
          return new Response("plain words\n", { headers: { "content-type": "text/plain" } });
        case "/big.bin":
          return new Response(new Uint8Array(11 * 1024 * 1024));
        case "/bigpage":
          return new Response(`<p>${"x".repeat(3 * 1024 * 1024)}</p>`, {
            headers: { "content-type": "text/html" },
          });
        case "/archive.zip":
          return new Response(new Uint8Array([0x50, 0x4b, 3, 4]), {
            headers: { "content-type": "application/zip" },
          });
        default:
          return new Response("nothing here", { status: 404 });
      }
    },
  });
  return {
    url: `http://127.0.0.1:${server.port}`,
    server,
    hits,
    stop: () => server.stop(true),
  };
}

/** The message `promise` rejected with (fails the test's expectations if it did not reject). */
export async function failure(promise: Promise<unknown>): Promise<string> {
  try {
    await promise;
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
  return "(did not reject)";
}
