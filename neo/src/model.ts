import { getModels, type Api, type KnownProvider, type Model } from "@earendil-works/pi-ai/compat";
import type { Logger } from "./logger.ts";

/**
 * Looks the model up in pi-ai's registry. The registry lags behind new releases, so an unknown
 * id on a known provider reuses that provider's first model with the id swapped, taking text only.
 */
export function resolveModel(
  provider: string,
  modelId: string,
  logger?: Logger,
  imagesSetting?: boolean,
  /** Models to look in before the registry (openrouter's live list). */
  live?: readonly Model<Api>[],
): Model<Api> {
  const model =
    live?.find((candidate) => candidate.id === modelId) ?? lookup(provider, modelId, logger);
  if (imagesSetting === undefined) return model;
  const input: Model<Api>["input"] = imagesSetting ? ["text", "image"] : ["text"];
  return { ...model, input };
}

function lookup(provider: string, modelId: string, logger?: Logger): Model<Api> {
  const models: Model<Api>[] = getModels(provider as KnownProvider);
  const known = models.find((model) => model.id === modelId);
  if (known) return known;
  const template = models[0];
  if (!template) throw new Error(`Unknown model provider "${provider}"`);
  logger?.warn(
    `Model "${modelId}" is not in pi-ai's registry for "${provider}"; using ${template.id} settings (set NEO_IMAGES=1 if it takes images).`,
  );
  return { ...template, id: modelId, name: modelId, input: ["text"] };
}
