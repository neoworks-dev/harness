// A session's input history on disk, under the app's data directory. The walking itself is in
// lib/history.ts; this part touches files, so a window is needed to run it.
import { files } from "@neoworks/svelte-gpui";
import { History, historyFile, historyLine, parseHistory } from "./history.ts";

let directory: Promise<string> | undefined;

/** `<data>/suite/gui-history`, made on first use. */
function historyDirectory(): Promise<string> {
  directory ??= files.paths().then(async ({ data }) => {
    const path = `${data}/suite/gui-history`;
    await files.mkdir(path, { recursive: true });
    return path;
  });
  return directory;
}

/** The saved history of `session`; empty when there is none or it cannot be read. */
export async function loadHistory(session: string): Promise<History> {
  try {
    const file = historyFile(await historyDirectory(), session);
    if (file === undefined) return new History();
    return new History(parseHistory(await files.readText(file)));
  } catch {
    return new History();
  }
}

/** Appends one sent input; a failure is ignored (history is a convenience). */
export async function appendHistory(session: string, text: string): Promise<void> {
  try {
    const file = historyFile(await historyDirectory(), session);
    if (file !== undefined) await files.appendText(file, historyLine(text));
  } catch {
    // Not worth interrupting the user for.
  }
}
