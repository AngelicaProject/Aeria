import type { AiHeaderConfig, AiModelConfig, AiModelSelection, AiProviderDto, AiProviderInput, AiProviderPresetDto, ReasoningEffort } from "./types";

/** Effort values in the order the settings UI offers them. */
export const reasoningEfforts: readonly ReasoningEffort[] = ["minimal", "low", "medium", "high"];

/** Builds the input for creating a provider from a preset. Models come from the provider later. */
export function providerFromPreset(preset: AiProviderPresetDto, baseUrl = preset.baseUrl ?? ""): AiProviderInput {
  return { id: null, kind: preset.kind, name: preset.name, baseUrl, models: [], sessionHeader: preset.sessionHeader, headers: [] };
}

/** Builds the input for replacing a stored provider. */
export function providerInput(provider: AiProviderDto, changes: Partial<Omit<AiProviderInput, "id" | "kind">> = {}): AiProviderInput {
  return {
    id: provider.id,
    kind: provider.kind,
    name: provider.name,
    baseUrl: provider.baseUrl,
    models: provider.models,
    sessionHeader: provider.sessionHeader,
    headers: provider.headers,
    ...changes,
  };
}

/**
 * Replaces the model list with the provider's own listing. Models the
 * provider still reports keep their configured efforts and context window.
 */
export function syncModels(models: readonly AiModelConfig[], remoteIds: readonly string[]): { models: AiModelConfig[]; added: number; removed: number } {
  const configured = new Map(models.map((model) => [model.id, model]));
  const next = addModels([], remoteIds).map((model) => configured.get(model.id) ?? model);
  const remote = new Set(next.map((model) => model.id));
  return {
    models: next,
    added: next.filter((model) => !configured.has(model.id)).length,
    removed: models.filter((model) => !remote.has(model.id)).length,
  };
}

/** Normalizes header rows for saving; rows with an empty name are dropped. */
export function normalizeHeaders(headers: readonly AiHeaderConfig[]): AiHeaderConfig[] {
  return headers
    .map((header) => ({ name: header.name.trim().toLowerCase(), value: header.value.trim() }))
    .filter((header) => header.name.length > 0);
}

/** Adds model IDs that are not configured yet, keeping the existing order. */
export function addModels(models: readonly AiModelConfig[], ids: readonly string[]): AiModelConfig[] {
  const next = [...models];
  const known = new Set(models.map((model) => model.id));
  for (const raw of ids) {
    const id = raw.trim();
    if (!id || known.has(id)) continue;
    known.add(id);
    next.push({ id, contextWindow: null, reasoningEfforts: [] });
  }
  return next;
}

export function removeModel(models: readonly AiModelConfig[], id: string): AiModelConfig[] {
  return models.filter((model) => model.id !== id);
}

/** Enables or disables one effort, keeping efforts in canonical order. */
export function toggleEffort(models: readonly AiModelConfig[], id: string, effort: ReasoningEffort): AiModelConfig[] {
  return models.map((model) => {
    if (model.id !== id) return model;
    const enabled = new Set(model.reasoningEfforts);
    if (enabled.has(effort)) enabled.delete(effort);
    else enabled.add(effort);
    return { ...model, reasoningEfforts: reasoningEfforts.filter((value) => enabled.has(value)) };
  });
}

/** Parses a context window field; blank means unknown. Returns `undefined` for invalid input. */
export function parseContextWindow(value: string): number | null | undefined {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!/^\d+$/.test(trimmed)) return undefined;
  const parsed = Number(trimmed);
  return parsed > 0 && parsed <= 0xffffffff ? parsed : undefined;
}

/** Returns the efforts the selected model accepts, or none when it is not configured. */
export function selectableEfforts(providers: readonly AiProviderDto[], selection: Pick<AiModelSelection, "providerId" | "modelId"> | null): ReasoningEffort[] {
  if (!selection) return [];
  const model = providers.find((provider) => provider.id === selection.providerId)?.models.find((candidate) => candidate.id === selection.modelId);
  return model ? [...model.reasoningEfforts] : [];
}

/** Encodes a provider/model pair as one select value. */
export function selectionKey(selection: Pick<AiModelSelection, "providerId" | "modelId">): string {
  return JSON.stringify([selection.providerId, selection.modelId]);
}

export function parseSelectionKey(value: string): Pick<AiModelSelection, "providerId" | "modelId"> | null {
  try {
    const parsed: unknown = JSON.parse(value);
    if (Array.isArray(parsed) && parsed.length === 2 && typeof parsed[0] === "string" && typeof parsed[1] === "string") {
      return { providerId: parsed[0], modelId: parsed[1] };
    }
  } catch {
    // An unparsable value means no selection.
  }
  return null;
}
