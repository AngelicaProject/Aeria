import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useState, type PropsWithChildren } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { defaultThemeId, findTheme, type ThemeDefinition } from "./registry";
import { hasWindowsBackdrop } from "./windowBackdrop";

type AppearancePreferences = {
  themeId: string;
  accentOverride: string | null;
  reduceTransparency: boolean;
};

type ThemeContextValue = {
  theme: ThemeDefinition;
  setThemeId: (themeId: string) => void;
  accentOverride: string | null;
  setAccentOverride: (accent: string | null) => void;
  reduceTransparency: boolean;
  setReduceTransparency: (reduce: boolean) => void;
};

const preferencesKey = "aeria.appearance";
const defaultPreferences: AppearancePreferences = { themeId: defaultThemeId, accentOverride: null, reduceTransparency: false };

/** Appearance is a per-machine renderer convenience, never project data. */
function readPreferences(): AppearancePreferences {
  try {
    const stored = JSON.parse(localStorage.getItem(preferencesKey) ?? "null") as Partial<AppearancePreferences> | null;
    if (!stored) return defaultPreferences;
    return {
      themeId: typeof stored.themeId === "string" ? stored.themeId : defaultThemeId,
      accentOverride: typeof stored.accentOverride === "string" ? stored.accentOverride : null,
      reduceTransparency: stored.reduceTransparency === true,
    };
  } catch {
    return defaultPreferences;
  }
}

function writePreferences(preferences: AppearancePreferences): void {
  try {
    localStorage.setItem(preferencesKey, JSON.stringify(preferences));
  } catch {
    // Appearance persistence must never block the editor.
  }
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: PropsWithChildren) {
  const [preferences, setPreferences] = useState(readPreferences);
  const theme = findTheme(preferences.themeId);
  const nativeBackdrop = hasWindowsBackdrop();

  useEffect(() => {
    function handleStorage(event: StorageEvent) {
      if (event.key === preferencesKey) setPreferences(readPreferences());
    }
    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, []);

  useEffect(() => {
    if (!nativeBackdrop) return;
    void getCurrentWindow().setTheme(theme.appearance === "light" ? "light" : "dark").catch(() => undefined);
  }, [nativeBackdrop, theme.appearance]);

  const update = useCallback((patch: Partial<AppearancePreferences>) => {
    setPreferences((current) => {
      const next = { ...current, ...patch };
      writePreferences(next);
      return next;
    });
  }, []);

  const tokens = useMemo(() => {
    const { tokens } = theme;
    return {
      "--color-crust": tokens.crust,
      "--color-mantle": tokens.mantle,
      "--color-base": tokens.base,
      "--color-surface-0": tokens.surface0,
      "--color-surface-1": tokens.surface1,
      "--color-surface-2": tokens.surface2,
      "--color-text": tokens.text,
      "--color-subtext": tokens.subtext,
      "--color-overlay": tokens.overlay,
      "--color-accent": preferences.accentOverride ?? tokens.accent,
      "--color-accent-fg": tokens.accentForeground,
      "--color-danger": tokens.danger,
      "--color-warning": tokens.warning,
      "--color-success": tokens.success,
      "--color-macro": tokens.macro ?? null,
    } satisfies Record<string, string | null>;
  }, [preferences.accentOverride, theme]);

  const value = useMemo<ThemeContextValue>(() => ({
    theme,
    setThemeId: (themeId) => update({ themeId }),
    accentOverride: preferences.accentOverride,
    setAccentOverride: (accentOverride) => update({ accentOverride }),
    reduceTransparency: preferences.reduceTransparency,
    setReduceTransparency: (reduceTransparency) => update({ reduceTransparency }),
  }), [preferences.accentOverride, preferences.reduceTransparency, theme, update]);

  const translucent = nativeBackdrop && !preferences.reduceTransparency && theme.appearance !== "highContrast";

  // Tokens live on <html> so Radix portals (menus, dialogs, tooltips) that
  // render outside the React root still resolve theme variables.
  useLayoutEffect(() => {
    const root = document.documentElement;
    for (const [name, tokenValue] of Object.entries(tokens)) {
      if (tokenValue === null) root.style.removeProperty(name);
      else root.style.setProperty(name, tokenValue);
    }
    root.dataset.themeId = theme.id;
    root.dataset.themeAppearance = theme.appearance;
    if (translucent) root.dataset.nativeBackdrop = "true";
    else delete root.dataset.nativeBackdrop;
  }, [theme.appearance, theme.id, tokens, translucent]);

  return (
    <ThemeContext.Provider value={value}>
      <div className="theme-root">{children}</div>
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error("useTheme must be used inside ThemeProvider");
  }
  return context;
}
