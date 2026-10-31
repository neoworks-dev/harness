// Speech to text for the apps (speech.transcribe): the recording goes to a model that hears
// audio, on the user's OpenRouter key, which writes out what was said. Claude Code's own voice
// mode isn't reachable through its ACP adapter and needs a Claude login, so it isn't used.
// NEO_SPEECH_MODEL picks another OpenRouter model that takes audio.
import type { SpeechAudio } from "@neoworks-dev/neo-protocol/types";
import { type AccountFiles, apiKey } from "./guests/accounts.ts";
import { BadRequestError } from "./focus.ts";

const URL = "https://openrouter.ai/api/v1/chat/completions";
/** OpenRouter's alias for Google's newest Flash, which hears audio and stays current. */
const MODEL = "~google/gemini-flash-latest";
const TIMEOUT_MS = 60_000;
const INSTRUCTION =
  "Transcribe this recording word for word, in the language it is spoken in. Reply with the " +
  "transcript only: no quotes, no notes, nothing about the audio. Empty if nothing is said.";

export type FetchFn = (url: string, init: RequestInit) => Promise<Response>;

export interface SpeechOptions {
  accounts: AccountFiles;
  env?: NodeJS.ProcessEnv;
  fetch?: FetchFn;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The text of a chat completion's first choice. */
function replyText(body: unknown): string | undefined {
  if (!isRecord(body) || !Array.isArray(body.choices)) return undefined;
  const choice: unknown = body.choices[0];
  if (!isRecord(choice) || !isRecord(choice.message)) return undefined;
  const content = choice.message.content;
  if (typeof content === "string") return content;
  return undefined;
}

/** OpenRouter's reason for a failed request, or its status. */
function failure(body: unknown, status: number): string {
  if (isRecord(body) && isRecord(body.error) && typeof body.error.message === "string") {
    return body.error.message;
  }
  return `HTTP ${status}`;
}

export function createTranscriber(options: SpeechOptions): (audio: SpeechAudio) => Promise<string> {
  const env = options.env ?? process.env;
  const fetchFn: FetchFn = options.fetch ?? ((url, init) => fetch(url, init));
  return async (audio) => {
    const key = await apiKey(options.accounts, "openrouter");
    if (key === undefined) {
      throw new BadRequestError(
        "Speech to text needs an OpenRouter key: /login openrouter adds one.",
      );
    }
    const model = env.NEO_SPEECH_MODEL || MODEL;
    let response: Response;
    try {
      response = await fetchFn(URL, {
        method: "POST",
        headers: { authorization: `Bearer ${key}`, "content-type": "application/json" },
        body: JSON.stringify({
          model,
          temperature: 0,
          messages: [
            {
              role: "user",
              content: [
                { type: "text", text: INSTRUCTION },
                { type: "input_audio", input_audio: { data: audio.data, format: audio.format } },
              ],
            },
          ],
        }),
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (err) {
      throw new BadRequestError(
        `Transcribing failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
    const body: unknown = await response.json().catch(() => undefined);
    if (!response.ok) {
      throw new BadRequestError(
        `Transcribing with ${model} failed: ${failure(body, response.status)}`,
      );
    }
    const text = replyText(body);
    if (text === undefined) throw new BadRequestError(`${model} sent no transcript`);
    return text.trim();
  };
}
