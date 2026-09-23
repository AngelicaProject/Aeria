/**
 * Per-machine renderer preferences. They are presentation conveniences stored
 * in local storage and never enter project data.
 */
export type ListDensity = "compact" | "comfortable";

export type Preferences = {
  interfaceZoom: number;
  editorFontSize: number;
  highlightMacros: boolean;
  showControlCharacters: boolean;
  listDensity: ListDensity;
  focusTargetOnNext: boolean;
};

export const interfaceZoomOptions = [0.9, 1, 1.1, 1.25] as const;
export const editorFontSizes = [12, 13, 14, 15, 16, 18, 20] as const;

export const defaultPreferences: Preferences = {
  interfaceZoom: 1,
  editorFontSize: 14,
  highlightMacros: true,
  showControlCharacters: true,
  listDensity: "comfortable",
  focusTargetOnNext: true,
};

function pick<T>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.includes(value as T) ? value as T : fallback;
}

/** Reads stored preferences, replacing anything unknown with its default. */
export function parsePreferences(raw: string | null): Preferences {
  let stored: Record<string, unknown> = {};
  try {
    const parsed = JSON.parse(raw ?? "null") as unknown;
    if (parsed && typeof parsed === "object") stored = parsed as Record<string, unknown>;
  } catch {
    return defaultPreferences;
  }
  const boolean = (key: keyof Preferences) => typeof stored[key] === "boolean" ? stored[key] as boolean : defaultPreferences[key] as boolean;
  return {
    interfaceZoom: pick(stored.interfaceZoom, interfaceZoomOptions, defaultPreferences.interfaceZoom),
    editorFontSize: pick(stored.editorFontSize, editorFontSizes, defaultPreferences.editorFontSize),
    highlightMacros: boolean("highlightMacros"),
    showControlCharacters: boolean("showControlCharacters"),
    listDensity: pick(stored.listDensity, ["compact", "comfortable"] as const, defaultPreferences.listDensity),
    focusTargetOnNext: boolean("focusTargetOnNext"),
  };
}
