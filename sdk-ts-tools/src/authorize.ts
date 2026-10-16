import { timingSafeEqual } from "node:crypto";

/** A random bearer token: 32 bytes, hex. */
export function newToken(): string {
  return Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString("hex");
}

/** Whether the request carries `Authorization: Bearer <token>` with the right token. */
export function hasToken(req: Request, token: string): boolean {
  const header = req.headers.get("authorization");
  if (header === null || !header.startsWith("Bearer ")) return false;
  const given = header.slice("Bearer ".length);
  if (given.length !== token.length) return false;
  return timingSafeEqual(Buffer.from(given), Buffer.from(token));
}
