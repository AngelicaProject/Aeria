import { createContext, useContext, useLayoutEffect, useMemo, type PropsWithChildren } from "react";
import { createTranslator, resolveLocale, type Locale, type Translate } from "../i18n/translate";
import { usePreferences } from "./preferences";

export type { MessageKey, Translate } from "../i18n/translate";

type I18nContextValue = {
  locale: Locale;
  t: Translate;
  formatNumber: (value: number) => string;
};

const I18nContext = createContext<I18nContextValue | null>(null);

function systemLanguages(): readonly string[] {
  return navigator.languages?.length ? navigator.languages : [navigator.language];
}

/** Resolves the interface locale from preferences; must be inside PreferencesProvider. */
export function I18nProvider({ children }: PropsWithChildren) {
  const { preferences } = usePreferences();
  const locale = resolveLocale(preferences.language, systemLanguages());

  useLayoutEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  const value = useMemo(() => {
    const numbers = new Intl.NumberFormat(locale);
    return { locale, t: createTranslator(locale), formatNumber: (number: number) => numbers.format(number) };
  }, [locale]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) throw new Error("useI18n must be used inside I18nProvider");
  return context;
}
