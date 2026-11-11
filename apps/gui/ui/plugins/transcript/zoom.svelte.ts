// The chat's size (zoom.ts), kept on disk in `<data>/neo-gui/chat.json`, a file of the window's
// and one of the tab's alike (the host's data directory).
import { files } from "@neoworks/svelte-gpui";
import { target } from "../../kernel/target.ts";
import { defaultZoom, parseZoom, stepZoom } from "./zoom.ts";

async function settingsPath(): Promise<string> {
  const { data } = await files.paths();
  await files.mkdir(`${data}/neo-gui`, { recursive: true });
  return `${data}/neo-gui/chat.json`;
}

class ChatZoom {
  value = $state(defaultZoom(target.web));
  /** Set once the user changed it here: the saved value read late doesn't undo that. */
  private changed = false;

  /** Reads the saved size; until then (or without one) the default holds. */
  async load(): Promise<void> {
    try {
      const saved = parseZoom(await files.readText(await settingsPath()));
      if (saved !== undefined && !this.changed) this.value = saved;
    } catch {
      // No file yet.
    }
  }

  step(by: 1 | -1): void {
    this.set(stepZoom(this.value, by));
  }

  reset(): void {
    this.set(defaultZoom(target.web));
  }

  private set(value: number): void {
    this.changed = true;
    this.value = value;
    void settingsPath()
      .then((path) => files.writeText(path, JSON.stringify({ chatZoom: value })))
      .catch(() => {
        // A convenience: the next change tries again.
      });
  }
}

export const chatZoom = new ChatZoom();
