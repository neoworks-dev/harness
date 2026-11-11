import { expect, test } from "bun:test";
import { upload, uploadAll, uploadedHandle, type Send, type Upload } from "../ui/lib/upload.ts";

interface Call {
  url: string;
  headers: Record<string, string>;
  body: unknown;
}

function daemonStoring(calls: Call[], status = 200): Send {
  return (url, init) => {
    calls.push({ url, headers: init.headers, body: init.body });
    const name = new URL(url).searchParams.get("name");
    return Promise.resolve({
      ok: status === 200,
      status,
      json: () =>
        Promise.resolve({ handle: "asset://img-0123456789ab", path: `/data/assets/${name}` }),
    });
  };
}

/** A file as a page has it, holding `text`. */
function file(name: string, type: string, text = "x"): Upload {
  return { name, type, arrayBuffer: () => Promise.resolve(new TextEncoder().encode(text).buffer) };
}

const address = { url: "http://127.0.0.1:41234/", token: "tok" };

test("an uploaded file goes as its bytes and comes back as its path on the daemon's disk", async () => {
  const calls: Call[] = [];
  const path = await upload(file("shot one.png", "image/png", "png!"), address, daemonStoring(calls));
  expect(path).toBe("/data/assets/shot one.png");
  const [call] = calls;
  const url = new URL(call?.url ?? "");
  expect(url.origin + url.pathname).toBe("http://127.0.0.1:41234/assets");
  expect(url.searchParams.get("name")).toBe("shot one.png");
  expect(url.searchParams.get("mime")).toBe("image/png");
  expect(call?.headers).toEqual({ authorization: "Bearer tok" });
  // Bytes, not a Blob: the page's fetch (svelte-gpui's) takes no Blob.
  expect(call?.body).toBeInstanceOf(Uint8Array);
  expect(new TextDecoder().decode(call?.body as Uint8Array)).toBe("png!");
});

test("the handle an upload gave back is kept for the path, for a preview the page can load", async () => {
  const path = await upload(file("shot.png", "image/png"), address, daemonStoring([]));
  expect(uploadedHandle(path)).toBe("asset://img-0123456789ab");
  expect(uploadedHandle("/somewhere/else.png")).toBeUndefined();
});

test("a file of unknown type is uploaded without a mime", async () => {
  const calls: Call[] = [];
  await upload(file("notes", ""), address, daemonStoring(calls));
  expect(new URL(calls[0]?.url ?? "").searchParams.has("mime")).toBe(false);
});

test("several files give their paths in the order they were given", async () => {
  const paths = await uploadAll(
    [file("a.png", "image/png"), file("b.txt", "text/plain")],
    address,
    daemonStoring([]),
  );
  expect(paths).toEqual(["/data/assets/a.png", "/data/assets/b.txt"]);
});

test("a refused upload names the file and what the daemon said", async () => {
  const refused = await upload(file("a.png", "image/png"), address, daemonStoring([], 401)).then(
    () => "",
    (error: Error) => error.message,
  );
  expect(refused).toBe("Could not upload a.png: the daemon said 401");
});
