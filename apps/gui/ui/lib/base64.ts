// Base64, which QuickJS has no `btoa` for: image bytes and SVG markup into `data:` URLs.

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

/** A `data:` URL for SVG markup (ASCII, as icon sets ship it). */
export function svgDataUrl(markup: string): string {
  const bytes = new Uint8Array(markup.length);
  for (let index = 0; index < markup.length; index++)
    bytes[index] = markup.charCodeAt(index) & 0xff;
  return `data:image/svg+xml;base64,${base64(bytes)}`;
}
