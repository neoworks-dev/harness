// Where the composer's attachments come from. Natively a file dialog gives paths and nothing else
// arrives on its own. A browser tab has only File objects, so main.web.ts swaps in a source that
// uploads them to the daemon first (web/attachments.ts) and also hears drops and pasted images.
import { files, invoke } from "@neoworks/svelte-gpui";

export interface AttachmentSource {
  /** Asks the user for files. Resolves with absolute paths on the daemon's machine, null when cancelled. */
  pick(): Promise<string[] | null>;
  /** Ctrl-V: the clipboard's image as a path, or null when it holds none. */
  pasteImage(): Promise<string | null>;
  /**
   * Hears files that arrive without being asked for, as the same kind of paths; `onError` hears
   * one that could not be taken. Returns a function that stops listening.
   */
  onFiles(listener: (paths: string[]) => void, onError: (error: Error) => void): () => void;
}

const native: AttachmentSource = {
  pick: () => files.pickFiles({ multiple: true, prompt: "Attach files" }),
  pasteImage: () => invoke("clipboard_image", null).catch(() => null),
  onFiles: () => () => {},
};

let source = native;

/** Replaces the native source. Called once, by an entry, before the window starts. */
export function useAttachments(next: AttachmentSource): void {
  source = next;
}

export const pickAttachments: AttachmentSource["pick"] = () => source.pick();

export const pasteAttachment: AttachmentSource["pasteImage"] = () => source.pasteImage();

export const onAttachments: AttachmentSource["onFiles"] = (listener, onError) =>
  source.onFiles(listener, onError);
