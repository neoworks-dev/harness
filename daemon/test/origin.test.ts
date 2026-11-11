import { afterEach, expect, test } from "bun:test";
import { isLoopbackOrigin } from "../src/server.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function daemon(): Promise<TestDaemon> {
  const d = await startTestDaemon();
  cleanups.push(() => d.stop());
  return d;
}

/** Bun's WebSocket takes headers, which the DOM's typing of it does not know. */
const HeaderSocket = WebSocket as unknown as new (
  url: string,
  options?: { headers: Record<string, string> },
) => WebSocket;

/** Opens /bus the way a browser page does, with the Origin it came from (none for a script). */
function dial(d: TestDaemon, origin: string | undefined): Promise<"open" | "refused"> {
  const url = `${d.info.url.replace("http", "ws")}/bus?token=${d.info.token}`;
  const ws = new HeaderSocket(url, origin === undefined ? undefined : { headers: { origin } });
  return new Promise((resolve) => {
    ws.onopen = () => {
      ws.close();
      resolve("open");
    };
    ws.onerror = () => resolve("refused");
  });
}

test("loopback pages are allowed in, whatever their port", () => {
  expect(isLoopbackOrigin("http://127.0.0.1:41234")).toBe(true);
  expect(isLoopbackOrigin("http://localhost:3000")).toBe(true);
  expect(isLoopbackOrigin("http://localhost")).toBe(true);
});

test("pages elsewhere are not, nor look-alikes", () => {
  expect(isLoopbackOrigin("https://evil.example")).toBe(false);
  expect(isLoopbackOrigin("http://127.0.0.1.evil.example")).toBe(false);
  expect(isLoopbackOrigin("http://localhost.evil.example:80")).toBe(false);
  expect(isLoopbackOrigin("https://127.0.0.1:8443")).toBe(false);
  expect(isLoopbackOrigin("null")).toBe(false);
  expect(isLoopbackOrigin("")).toBe(false);
});

test("/bus takes a client without an Origin, as before", async () => {
  expect(await dial(await daemon(), undefined)).toBe("open");
});

test("/bus takes a page on this machine", async () => {
  const d = await daemon();
  expect(await dial(d, "http://127.0.0.1:50321")).toBe("open");
  expect(await dial(d, "http://localhost:50321")).toBe("open");
});

test("/bus refuses a page from another site, even with the token", async () => {
  const d = await daemon();
  expect(await dial(d, "https://evil.example")).toBe("refused");
  expect(await dial(d, "null")).toBe("refused");
  const plain = await fetch(`${d.info.url}/bus?token=${d.info.token}`, {
    headers: { origin: "https://evil.example" },
  });
  expect(plain.status).toBe(403);
});

test("a page on this machine may upload to /assets across ports; another site may not", async () => {
  const d = await daemon();
  const page = "http://127.0.0.1:50321";
  const asked = await fetch(`${d.info.url}/assets?name=a.txt`, {
    method: "OPTIONS",
    headers: { origin: page, "access-control-request-method": "POST" },
  });
  expect(asked.status).toBe(204);
  expect(asked.headers.get("access-control-allow-origin")).toBe(page);
  expect(asked.headers.get("access-control-allow-headers")).toContain("authorization");

  const stored = await fetch(`${d.info.url}/assets?name=a.txt&mime=text/plain`, {
    method: "POST",
    body: "hello",
    headers: { origin: page, authorization: `Bearer ${d.info.token}` },
  });
  expect(stored.status).toBe(200);
  expect(stored.headers.get("access-control-allow-origin")).toBe(page);
  const asset = (await stored.json()) as { path: string };
  expect(await Bun.file(asset.path).text()).toBe("hello");

  const elsewhere = await fetch(`${d.info.url}/assets?name=a.txt`, {
    method: "POST",
    body: "hello",
    headers: { origin: "https://evil.example", authorization: `Bearer ${d.info.token}` },
  });
  expect(elsewhere.status).toBe(403);
  expect(elsewhere.headers.get("access-control-allow-origin")).toBeNull();
});

test("a request with no Origin gets no CORS headers", async () => {
  const d = await daemon();
  const res = await fetch(`${d.info.url}/assets?name=a.txt`, {
    method: "POST",
    body: "hi",
    headers: { authorization: `Bearer ${d.info.token}` },
  });
  expect(res.status).toBe(200);
  expect(res.headers.get("access-control-allow-origin")).toBeNull();
});
