/**
 * Target languages of a project: the language it translates into, as a
 * BCP 47 tag such as `ru` or `pt-BR`. Rust validates the same rule
 * (`aeria_core::is_target_language`); this module offers common choices
 * and names them.
 */

/** Languages offered first; any other tag can be typed. */
export const commonTargetLanguages: readonly string[] = [
  "ru", "be", "kk", "pl", "cs", "tr", "es", "es-419", "pt-BR", "it", "nl", "sv",
  "ko", "zh-Hans", "zh-Hant", "th", "vi", "id", "en", "ja", "de", "fr",
];

/** The tag of a project created before its target language was chosen. */
export const UNDETERMINED_LANGUAGE = "und";

/** Whether `tag` is a BCP 47 tag a project can translate into: not `und`. */
export function isTargetLanguage(tag: string): boolean {
  return /^[A-Za-z]{2,3}(-[A-Za-z0-9]{1,8})*$/.test(tag) && tag.toLowerCase() !== UNDETERMINED_LANGUAGE;
}

/** A language's own name with its tag, such as "русский (ru)"; the tag alone when unknown. */
export function languageName(tag: string): string {
  try {
    const name = new Intl.DisplayNames([tag], { type: "language" }).of(tag);
    return name && name.toLowerCase() !== tag.toLowerCase() ? `${name} (${tag})` : tag;
  } catch {
    return tag;
  }
}

/**
 * A target language to start a new project with: the interface language
 * when it can be one and is not the language the project translates from.
 */
export function suggestedTargetLanguage(interfaceLocale: string, sourceLanguage: string): string | null {
  const language = interfaceLocale.split("-")[0] ?? "";
  return isTargetLanguage(language) && language !== sourceLanguage ? language : null;
}
