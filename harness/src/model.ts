import { getModels, type Api, type KnownProvider, type Model } from "@mariozechner/pi-ai";
import type { Logger } from "./logger.ts";

/**
 * Looks the model up in pi-ai's registry. The registry lags behind new releases, so an unknown
 * id on a known provider reuses that provider's first model with the id swapped.
 */
export function resolveModel(provider: string, modelId: string, logger?: Logger): Model<Api> {
  const models: Model<Api>[] = getModels(provider as KnownProvider);
  const known = models.find((model) => model.id === modelId);
  if (known) return known;
  const template = models[0];
  if (!template) throw new Error(`Unknown model provider "${provider}"`);
  logger?.warn(
    `Model "${modelId}" is not in pi-ai's registry for "${provider}"; using ${template.id} settings.`,
  );
  return { ...template, id: modelId, name: modelId };
}
