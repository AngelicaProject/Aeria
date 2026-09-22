import { createContext, useContext, useEffect, useMemo, useState, type PropsWithChildren } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { defaultThemeId, findTheme, type ThemeDefinition } from "./registry";
import { hasWindowsBackdrop } from "./windowBackdrop";

type ThemeContextValue = {
  theme: ThemeDefinition;
  setThemeId: (themeId: string) => void;
  accentOverride: string | null;
  setAccentOverride: (accent: string | null) => void;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: PropsWithChildren) {
  const [themeId, setThemeId] = useState(defaultThemeId);
  const [accentOverride, setAccentOverride] = useState<string | null>(null);
  const theme = findTheme(themeId);
  const nativeBackdrop = hasWindowsBackdrop();
  useEffect(() => {
    if (!nativeBackdrop) return;
    void getCurrentWindow().setTheme(theme.appearance === "light" ? "light" : "dark").catch(() => undefined);
  }, [nativeBackdrop, theme.appearance]);
  const style = useMemo(() => {
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
      "--color-accent": accentOverride ?? tokens.accent,
      "--color-accent-fg": tokens.accentForeground,
      "--color-danger": tokens.danger,
      "--color-warning": tokens.warning,
      "--color-success": tokens.success,
      "--font-ui": '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
      "--font-content": '"Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo", "Microsoft YaHei UI", "Malgun Gothic", sans-serif',
      "--font-mono": '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
      "--font-size-ui": "12px",
      "--font-size-compact": "11px",
      "--font-size-status": "10.5px",
      "--font-size-content": "12px",
      "--font-size-editor": "13px",
      "--font-size-mono": "10.5px",
      "--font-size-macro": "11.5px",
    } as React.CSSProperties;
  }, [accentOverride, theme]);

  return (
    <ThemeContext.Provider value={{ theme, setThemeId, accentOverride, setAccentOverride }}>
      <div className="theme-root" data-theme-id={theme.id} data-theme-appearance={theme.appearance} data-native-backdrop={nativeBackdrop ? "true" : undefined} style={style}>
        {children}
      </div>
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
