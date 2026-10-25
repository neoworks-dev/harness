import { getModels, type Api, type KnownProvider, type Model } from "@mariozechner/pi-ai";
import type { Logger } from "./logger.ts";

/**
 * Looks the model up in pi-ai's registry. The registry lags behind new releases, so an unknown
 * id on a known provider reuses that provider's first model with the id swapped.
 */
/** Models pi's registry doesn't know yet (or lists as text-only) that do take images. */
const VISION = new Set(["deepseek/deepseek-flash"]);

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
  const images = imagesSetting ?? (VISION.has(`${provider}/${modelId}`) ? true : undefined);
  if (images === undefined) return model;
  const input: Model<Api>["input"] = images ? ["text", "image"] : ["text"];
  return { ...model, input };
}

function lookup(provider: string, modelId: string, logger?: Logger): Model<Api> {
  const models: Model<Api>[] = getModels(provider as KnownProvider);
  const known = models.find((model) => model.id === modelId);
  if (known) return known;
  const template = models[0];
  if (!template) throw new Error(`Unknown model provider "${provider}"`);
  if (VISION.has(`${provider}/${modelId}`)) return { ...template, id: modelId, name: modelId };
  logger?.warn(
    `Model "${modelId}" is not in pi-ai's registry for "${provider}"; using ${template.id} settings (set SUITE_IMAGES=1 if it takes images).`,
  );
  return { ...template, id: modelId, name: modelId };
}
