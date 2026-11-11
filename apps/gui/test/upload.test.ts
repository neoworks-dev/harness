import { expect, test } from "bun:test";
import { upload, uploadAll, type Send } from "../ui/lib/upload.ts";

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

const address = { url: "http://127.0.0.1:41234/", token: "tok" };

test("an uploaded file comes back as its path on the daemon's disk", async () => {
  const calls: Call[] = [];
  const file = { name: "shot one.png", type: "image/png" };
  const path = await upload(file, address, daemonStoring(calls));
  expect(path).toBe("/data/assets/shot one.png");
  const [call] = calls;
  const url = new URL(call?.url ?? "");
  expect(url.origin + url.pathname).toBe("http://127.0.0.1:41234/assets");
  expect(url.searchParams.get("name")).toBe("shot one.png");
  expect(url.searchParams.get("mime")).toBe("image/png");
  expect(call?.headers).toEqual({ authorization: "Bearer tok" });
  expect(call?.body).toBe(file);
});

test("a file of unknown type is uploaded without a mime", async () => {
  const calls: Call[] = [];
  await upload({ name: "notes", type: "" }, address, daemonStoring(calls));
  expect(new URL(calls[0]?.url ?? "").searchParams.has("mime")).toBe(false);
});

test("several files give their paths in the order they were given", async () => {
  const paths = await uploadAll(
    [
      { name: "a.png", type: "image/png" },
      { name: "b.txt", type: "text/plain" },
    ],
    address,
    daemonStoring([]),
  );
  expect(paths).toEqual(["/data/assets/a.png", "/data/assets/b.txt"]);
});

test("a refused upload names the file and what the daemon said", async () => {
  const refused = await upload(
    { name: "a.png", type: "image/png" },
    address,
    daemonStoring([], 401),
  ).then(
    () => "",
    (error: Error) => error.message,
  );
  expect(refused).toBe("Could not upload a.png: the daemon said 401");
});
