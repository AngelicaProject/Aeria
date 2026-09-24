import { en, type Catalog, type Message, type MessageKey } from "./en.ts";
import { ru } from "./ru.ts";

/**
 * Renderer interface localization. Catalogs cover only Aeria's own chrome;
 * game text, project data, and backend diagnostics are never translated here.
 */
export const supportedLocales = ["en", "ru"] as const;
export type Locale = (typeof supportedLocales)[number];
export type LanguagePreference = "system" | Locale;
export const languagePreferences: readonly LanguagePreference[] = ["system", ...supportedLocales];

export type { MessageKey } from "./en.ts";
export type MessageParams = Readonly<Record<string, string | number>>;
export type Translate = (key: MessageKey, params?: MessageParams) => string;

const catalogs: Readonly<Record<Locale, Catalog>> = { en, ru };

/** Autonyms shown in the language picker regardless of the current locale. */
export const localeNames: Readonly<Record<Locale, string>> = { en: "English", ru: "Русский" };

function isLocale(value: string): value is Locale {
  return (supportedLocales as readonly string[]).includes(value);
}

/** Picks the explicit preference, else the first supported system language, else English. */
export function resolveLocale(preference: LanguagePreference, systemLanguages: readonly string[]): Locale {
  if (preference !== "system") return preference;
  for (const tag of systemLanguages) {
    const language = tag.split("-")[0]?.toLowerCase() ?? "";
    if (isLocale(language)) return language;
  }
  return "en";
}

function selectMessage(message: Message, plural: Intl.PluralRules, count: number | undefined): string {
  if (typeof message === "string") return message;
  if (count === undefined) return message.other;
  return message[plural.select(count)] ?? message.other;
}

/**
 * Builds `t(key, params)`. `{name}` placeholders are replaced from `params`;
 * numbers are formatted for the locale with digit grouping, so identifiers
 * such as row ids must be passed as strings. A `count` param selects the
 * plural form.
 */
export function createTranslator(locale: Locale): Translate {
  const catalog = catalogs[locale];
  const plural = new Intl.PluralRules(locale);
  const numbers = new Intl.NumberFormat(locale);
  return (key, params) => {
    const count = typeof params?.count === "number" ? params.count : undefined;
    const template = selectMessage(catalog[key], plural, count);
    if (!params) return template;
    return template.replace(/\{(\w+)\}/g, (placeholder, name: string) => {
      const value = params[name];
      if (value === undefined) return placeholder;
      return typeof value === "number" ? numbers.format(value) : value;
    });
  };
}
