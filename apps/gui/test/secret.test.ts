import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { SecretRequest } from "@suite/protocol/types";
import { incoming, stale, submitted, type Ask } from "../ui/plugins/secret/ask.ts";

function request(id: string, session = "s1", placeholder?: string): SecretRequest {
  const made: SecretRequest = { id, session, title: "DeepSeek API key" };
  if (placeholder !== undefined) made.placeholder = placeholder;
  return made;
}

const open: Ask = { id: "a", session: "s1", title: "OpenAI API key" };

test("a request for the shown session opens a question", () => {
  expect(incoming(undefined, request("a", "s1", "sk-"), "s1")).toEqual({
    show: { id: "a", session: "s1", title: "DeepSeek API key", placeholder: "sk-" },
    cancel: [],
  });
  expect(incoming(undefined, request("a"), "s1").show).toEqual({
    id: "a",
    session: "s1",
    title: "DeepSeek API key",
  });
});

test("a second request replaces the open one, which is answered as cancelled", () => {
  const next = incoming(open, request("b"), "s1");
  expect(next.show?.id).toBe("b");
  expect(next.cancel).toEqual(["a"]);
});

test("a request for another session is cancelled and leaves the open question alone", () => {
  expect(incoming(open, request("b", "other"), "s1")).toEqual({ show: open, cancel: ["b"] });
  expect(incoming(undefined, request("b"), undefined)).toEqual({ show: undefined, cancel: ["b"] });
});

test("only a typed value is sent, without surrounding blanks", () => {
  expect(submitted("")).toBeUndefined();
  expect(submitted("  \n ")).toBeUndefined();
  expect(submitted("  sk-123 \n")).toBe("sk-123");
});

test("a question goes when the connection or the session goes", () => {
  expect(stale(open, "s1", "open")).toBeUndefined();
  expect(stale(open, "s1", "reconnecting")).toBe("connection");
  expect(stale(open, "s2", "open")).toBe("session");
  expect(stale(open, "s2", "closed")).toBe("connection");
});

test("the secret pane keeps the text out of every store and log", () => {
  const dir = join(import.meta.dir, "../ui/plugins/secret");
  for (const file of readdirSync(dir)) {
    const source = readFileSync(join(dir, file), "utf8");
    for (const leak of [
      /console\./,
      /localStorage/,
      /history-store|appendHistory/,
      /invoke\(/,
      /\.error\s*=/,
    ]) {
      expect(source, `${file} ${leak}`).not.toMatch(leak);
    }
  }
});
