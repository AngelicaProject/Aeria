import type { StringState, TranslationOverlayDto } from "./types";

/** The state of a string from its entry: untranslated (`null`), translated, or fuzzy. */
export function stringState(translation: TranslationOverlayDto | null | undefined): StringState | null {
  if (!translation) return null;
  if (translation.fuzzy) return "fuzzy";
  return translation.targetMacro ? "translated" : null;
}
