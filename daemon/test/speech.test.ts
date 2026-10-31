import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createTranscriber, type FetchFn } from "../src/speech.ts";

function accounts(withKey: boolean): { authPath: string; piAuthPath: string } {
  const dir = mkdtempSync(join(tmpdir(), "neo-speech-"));
  const authPath = join(dir, "auth.json");
  const file: Record<string, unknown> = {};
  if (withKey) file.openrouter = { type: "api_key", key: "sk-or-test" };
  writeFileSync(authPath, JSON.stringify(file));
  return { authPath, piAuthPath: join(dir, "pi.json") };
}

test("the recording goes to OpenRouter as audio, the reply comes back as the text", async () => {
  let sent: { url: string; init: RequestInit } | undefined;
  const fetch: FetchFn = async (url, init) => {
    sent = { url, init };
    return Response.json({ choices: [{ message: { content: " Run the tests.\n" } }] });
  };
  const transcribe = createTranscriber({ accounts: accounts(true), env: {}, fetch });
  expect(await transcribe({ data: "UklGRg==", format: "wav" })).toBe("Run the tests.");
  expect(sent?.url).toBe("https://openrouter.ai/api/v1/chat/completions");
  expect(new Headers(sent?.init.headers).get("authorization")).toBe("Bearer sk-or-test");
  const body = JSON.parse(String(sent?.init.body));
  expect(body.model).toBe("~google/gemini-flash-latest");
  expect(body.messages[0].content[1]).toEqual({
    type: "input_audio",
    input_audio: { data: "UklGRg==", format: "wav" },
  });
});

test("no key, or a failed request, is an error the user can read", async () => {
  const fetch: FetchFn = async () =>
    Response.json({ error: { message: "model can't hear" } }, { status: 400 });
  const env = { NEO_SPEECH_MODEL: "x/deaf" };
  const failing = createTranscriber({ accounts: accounts(true), env, fetch });
  expect(failing({ data: "", format: "wav" })).rejects.toThrow(
    "Transcribing with x/deaf failed: model can't hear",
  );
  const keyless = createTranscriber({ accounts: accounts(false), env: {}, fetch });
  expect(keyless({ data: "", format: "wav" })).rejects.toThrow("/login openrouter");
});
