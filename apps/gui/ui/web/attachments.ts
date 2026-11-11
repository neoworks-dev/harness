/// <reference lib="dom" />
// The composer's attachments in a browser tab. The tab has File objects,
// the daemon wants paths on its own disk: each file is uploaded to the daemon first (lib/upload.ts)
// and the path of the stored copy is attached. Covers the file picker, files dropped on the page
// and an image pasted into it. Folders cannot be read by a browser: a dropped one is skipped.
// Importing this module installs it.
import { invoke } from "@neoworks/svelte-gpui";
import { useAttachments, type AttachmentSource } from "../kernel/attachments.ts";
import { uploadAll, type Send } from "../lib/upload.ts";

const send: Send = (url, init) => fetch(url, init as RequestInit);

async function uploadFiles(files: readonly File[]): Promise<string[]> {
  return uploadAll(files, await invoke("daemon_address", null), send);
}

/** Files the user chooses in the browser's own dialog; null when cancelled. */
function choose(): Promise<File[] | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.addEventListener("change", () => resolve(Array.from(input.files ?? [])));
    input.addEventListener("cancel", () => resolve(null));
    input.click();
  });
}

/** The files of a drop, without the folders. */
function droppedFiles(transfer: DataTransfer): File[] {
  const items = Array.from(transfer.items).filter((item) => item.kind === "file");
  return items
    .filter((item) => item.webkitGetAsEntry()?.isDirectory !== true)
    .flatMap((item) => item.getAsFile() ?? []);
}

export const web: AttachmentSource = {
  async pick() {
    const picked = await choose();
    return picked === null ? null : uploadFiles(picked);
  },

  // The page's paste event brings the image (see onFiles); there is no clipboard to read here.
  pasteImage: () => Promise.resolve(null),

  onFiles(listener, onError) {
    const take = (files: File[]): void => {
      if (files.length > 0) void uploadFiles(files).then(listener, onError);
    };
    const hasFiles = (transfer: DataTransfer | null): transfer is DataTransfer =>
      transfer !== null && Array.from(transfer.types).includes("Files");
    const over = (event: DragEvent): void => {
      if (hasFiles(event.dataTransfer)) event.preventDefault();
    };
    const drop = (event: DragEvent): void => {
      if (!hasFiles(event.dataTransfer)) return;
      event.preventDefault();
      take(droppedFiles(event.dataTransfer));
    };
    const paste = (event: ClipboardEvent): void => {
      const images = Array.from(event.clipboardData?.files ?? []).filter((file) =>
        file.type.startsWith("image/"),
      );
      if (images.length === 0) return; // text pastes into the input as usual
      event.preventDefault();
      take(images);
    };
    document.addEventListener("dragover", over);
    document.addEventListener("drop", drop);
    document.addEventListener("paste", paste, true);
    return () => {
      document.removeEventListener("dragover", over);
      document.removeEventListener("drop", drop);
      document.removeEventListener("paste", paste, true);
    };
  },
};

useAttachments(web);
