import type { PermissionAsk, RequestPermission } from "../agent.ts";

/**
 * Asks before a tool acts, when the embedder gave a hook. A refusal (or a cancelled turn while
 * waiting) throws: pi turns a thrown error into the tool's error result, which the model reads.
 */
export async function askToProceed(
  ask: RequestPermission | undefined,
  request: PermissionAsk,
  declined: string,
  signal?: AbortSignal,
): Promise<void> {
  if (!ask) return;
  const allowed = await ask(request, signal);
  if (signal?.aborted) throw new Error("Cancelled before the user answered.");
  if (!allowed) throw new Error(declined);
}
