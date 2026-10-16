// A project's icon for the sidebar: its favicon if the repository has one where web projects keep
// it, else a badge with its initials. Plain functions; the lookup on disk is in `icons.svelte.ts`.

/** Where a favicon is looked for, relative to the repository and to each `apps/*` and `packages/*`. */
export const FAVICON_PLACES = [
  "favicon.svg",
  "favicon.png",
  "public/favicon.svg",
  "public/favicon.png",
  "static/favicon.svg",
  "static/favicon.png",
  "app/icon.svg",
  "app/icon.png",
  "src/app/icon.svg",
  "src/app/icon.png",
  "assets/icon.png",
  "icon.png",
  "favicon.ico",
  "public/favicon.ico",
  "static/favicon.ico",
  "app/favicon.ico",
  "src/app/favicon.ico",
];

/** Folders whose children are projects of their own in a monorepo. */
export const PROJECT_FOLDERS = ["apps", "packages"];

const TYPES: Record<string, string> = {
  svg: "image/svg+xml",
  png: "image/png",
  ico: "image/x-icon",
};

/** The MIME type of an image path by its extension. */
export function imageType(path: string): string {
  const extension = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  return TYPES[extension] ?? "application/octet-stream";
}

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Base64 of some bytes. QuickJS has no `btoa`. */
export function base64(bytes: Uint8Array): string {
  let out = "";
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index] ?? 0;
    const second = bytes[index + 1];
    const third = bytes[index + 2];
    const chunk = (first << 16) | ((second ?? 0) << 8) | (third ?? 0);
    out += ALPHABET[(chunk >> 18) & 63];
    out += ALPHABET[(chunk >> 12) & 63];
    out += second === undefined ? "=" : ALPHABET[(chunk >> 6) & 63];
    out += third === undefined ? "=" : ALPHABET[chunk & 63];
  }
  return out;
}

/** A `data:` URL for an image file's bytes. */
export function dataUrl(path: string, bytes: Uint8Array): string {
  return `data:${imageType(path)};base64,${base64(bytes)}`;
}

/** Up to two initials of a project's name: "svelte-gpui" is SG, "neoworks" is N. */
export function initials(name: string): string {
  const words = name.split(/[-_.\s]+/).filter(Boolean);
  return words
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? "")
    .join("");
}
