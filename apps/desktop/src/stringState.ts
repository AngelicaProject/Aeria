import type { StringState, TranslationOverlayDto } from "./types";

/**
 * The state of a string from its entry: untranslated (`null`), translated,
 * reviewed by a person, or fuzzy. A changed source asks for attention first,
 * even of a reviewed string.
 */
export function stringState(translation: TranslationOverlayDto | null | undefined): StringState | null {
  if (!translation) return null;
  if (translation.fuzzy) return "fuzzy";
  if (!translation.targetMacro) return null;
  return translation.reviewed ? "reviewed" : "translated";
}
