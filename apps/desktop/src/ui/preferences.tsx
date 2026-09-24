import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useState, type PropsWithChildren } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { defaultPreferences, parsePreferences, type Preferences } from "./preferencesModel";

export * from "./preferencesModel";

const storageKey = "aeria.preferences";

function readPreferences(): Preferences {
  try {
    return parsePreferences(localStorage.getItem(storageKey));
  } catch {
    return defaultPreferences;
  }
}

type PreferencesContextValue = {
  preferences: Preferences;
  setPreference: <K extends keyof Preferences>(key: K, value: Preferences[K]) => void;
  resetPreferences: () => void;
};

const PreferencesContext = createContext<PreferencesContextValue | null>(null);

export function PreferencesProvider({ children }: PropsWithChildren) {
  const [preferences, setPreferences] = useState(readPreferences);

  useEffect(() => {
    function handleStorage(event: StorageEvent) {
      if (event.key === storageKey) setPreferences(readPreferences());
    }
    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, []);

  const store = useCallback((next: Preferences) => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(next));
    } catch {
      // Preferences are a convenience and must never block the editor.
    }
  }, []);

  const setPreference = useCallback(<K extends keyof Preferences>(key: K, value: Preferences[K]) => {
    setPreferences((current) => {
      const next = { ...current, [key]: value };
      store(next);
      return next;
    });
  }, [store]);

  // The interface language is not an editor setting, so a reset keeps it.
  const resetPreferences = useCallback(() => {
    setPreferences((current) => {
      const next = { ...defaultPreferences, language: current.language };
      store(next);
      return next;
    });
  }, [store]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--editor-font-size", `${preferences.editorFontSize}px`);
    root.dataset.listDensity = preferences.listDensity;
  }, [preferences.editorFontSize, preferences.listDensity]);

  useEffect(() => {
    if (!isTauri()) return;
    void getCurrentWebview().setZoom(preferences.interfaceZoom).catch(() => undefined);
  }, [preferences.interfaceZoom]);

  const value = useMemo(() => ({ preferences, setPreference, resetPreferences }), [preferences, resetPreferences, setPreference]);
  return <PreferencesContext.Provider value={value}>{children}</PreferencesContext.Provider>;
}

export function usePreferences(): PreferencesContextValue {
  const context = useContext(PreferencesContext);
  if (!context) throw new Error("usePreferences must be used inside PreferencesProvider");
  return context;
}
