import { languagePreferences, type LanguagePreference } from "../i18n/translate.ts";

/**
 * Per-machine renderer preferences. They are presentation conveniences stored
 * in local storage and never enter project data.
 */
export type ListDensity = "compact" | "comfortable";
/** What a source or target pane shows: the macro text, or the string as the game shows it. */
/** A pane shows the string as text with tag chips, or as its macro code. */
export type PaneMode = "text" | "code";
/** The tab the translation editor's side pane shows. */
export type SidePaneTab = "note" | "languages" | "history";
export const sidePaneTabs = ["note", "languages", "history"] as const;
/** How quest and cutscene sheets are shown: as the strings list, or as a scene. */
export type DialogueView = "strings" | "scene";

export type Preferences = {
  language: LanguagePreference;
  interfaceZoom: number;
  editorFontSize: number;
  highlightMacros: boolean;
  showControlCharacters: boolean;
  sourcePaneMode: PaneMode;
  targetPaneMode: PaneMode;
  sidePaneTab: SidePaneTab;
  listDensity: ListDensity;
  dialogueView: DialogueView;
  focusTargetOnNext: boolean;
  /** The model machine translation uses; empty until one is chosen. */
  translationModel: string;
  /** Its reasoning effort; empty for the model's default. */
  translationEffort: string;
};

export const interfaceZoomOptions = [0.9, 1, 1.1, 1.25] as const;
export const editorFontSizes = [12, 13, 14, 15, 16, 18, 20] as const;

export const defaultPreferences: Preferences = {
  language: "system",
  interfaceZoom: 1,
  editorFontSize: 14,
  highlightMacros: true,
  showControlCharacters: true,
  sourcePaneMode: "text",
  targetPaneMode: "text",
  sidePaneTab: "note",
  listDensity: "comfortable",
  dialogueView: "scene",
  focusTargetOnNext: true,
  translationModel: "",
  translationEffort: "",
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
    language: pick(stored.language, languagePreferences, defaultPreferences.language),
    interfaceZoom: pick(stored.interfaceZoom, interfaceZoomOptions, defaultPreferences.interfaceZoom),
    editorFontSize: pick(stored.editorFontSize, editorFontSizes, defaultPreferences.editorFontSize),
    highlightMacros: boolean("highlightMacros"),
    showControlCharacters: boolean("showControlCharacters"),
    sourcePaneMode: pick(stored.sourcePaneMode, ["text", "code"] as const, defaultPreferences.sourcePaneMode),
    targetPaneMode: pick(stored.targetPaneMode, ["text", "code"] as const, defaultPreferences.targetPaneMode),
    sidePaneTab: pick(stored.sidePaneTab, sidePaneTabs, defaultPreferences.sidePaneTab),
    listDensity: pick(stored.listDensity, ["compact", "comfortable"] as const, defaultPreferences.listDensity),
    dialogueView: pick(stored.dialogueView, ["strings", "scene"] as const, defaultPreferences.dialogueView),
    focusTargetOnNext: boolean("focusTargetOnNext"),
    translationModel: typeof stored.translationModel === "string" ? stored.translationModel : defaultPreferences.translationModel,
    translationEffort: typeof stored.translationEffort === "string" ? stored.translationEffort : defaultPreferences.translationEffort,
  };
}
