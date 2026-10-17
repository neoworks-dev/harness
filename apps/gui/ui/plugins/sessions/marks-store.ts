// The sidebar's marks on disk: `<data>/suite-gui/sessions.json`. The marks themselves are in
// marks.ts; this part touches files, so a window is needed to run it.
import { files } from "@neoworks/svelte-gpui";
import { NO_MARKS, parseMarks, serializeMarks, type Marks } from "./marks.ts";

let directory: Promise<string> | undefined;

/** `<data>/suite-gui`, made on first use. */
function marksDirectory(): Promise<string> {
  directory ??= files.paths().then(async ({ data }) => {
    const path = `${data}/suite-gui`;
    await files.mkdir(path, { recursive: true });
    return path;
  });
  return directory;
}

/** The saved marks; none when there is no file or it cannot be read. */
export async function loadMarks(): Promise<Marks> {
  try {
    return parseMarks(await files.readText(`${await marksDirectory()}/sessions.json`));
  } catch {
    return NO_MARKS;
  }
}

/** Writes the marks; a failure is ignored (they are a convenience, and the next change tries again). */
export async function saveMarks(marks: Marks): Promise<void> {
  try {
    await files.writeText(`${await marksDirectory()}/sessions.json`, serializeMarks(marks));
  } catch {
    // Not worth interrupting the user for.
  }
}
