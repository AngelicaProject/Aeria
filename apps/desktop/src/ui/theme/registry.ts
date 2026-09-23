import { editorThemes } from "./editorThemes.ts";

export type ThemeAppearance = "light" | "dark" | "highContrast";

export type ThemeTokens = {
  crust: string;
  mantle: string;
  base: string;
  surface0: string;
  surface1: string;
  surface2: string;
  text: string;
  subtext: string;
  overlay: string;
  accent: string;
  accentForeground: string;
  danger: string;
  warning: string;
  success: string;
  /** Macro token color in source/target editors; derived from accent when absent. */
  macro?: string;
};

export type ThemeDefinition = {
  id: string;
  displayName: string;
  family?: string;
  appearance: ThemeAppearance;
  tokens: ThemeTokens;
  defaultAccent?: string;
  source: "builtin" | "themePack";
};

const macchiato: ThemeTokens = {
  crust: "#181926",
  mantle: "#1e2030",
  base: "#24273a",
  surface0: "#363a4f",
  surface1: "#494d64",
  surface2: "#5b6078",
  text: "#cad3f5",
  subtext: "#b8c0e0",
  overlay: "#8087a2",
  accent: "#c6a0f6",
  accentForeground: "#181926",
  danger: "#ed8796",
  warning: "#f5a97f",
  success: "#a6da95",
};

const latte: ThemeTokens = {
  crust: "#dce0e8",
  mantle: "#e6e9ef",
  base: "#eff1f5",
  surface0: "#ccd0da",
  surface1: "#bcc0cc",
  surface2: "#acb0be",
  text: "#4c4f69",
  subtext: "#5c5f77",
  overlay: "#7c7f93",
  accent: "#8839ef",
  accentForeground: "#eff1f5",
  danger: "#d20f39",
  warning: "#fe640b",
  success: "#40a02b",
};

const frappe: ThemeTokens = {
  crust: "#232634",
  mantle: "#292c3c",
  base: "#303446",
  surface0: "#414559",
  surface1: "#51576d",
  surface2: "#626880",
  text: "#c6d0f5",
  subtext: "#b5bfe2",
  overlay: "#838ba7",
  accent: "#ca9ee6",
  accentForeground: "#232634",
  danger: "#e78284",
  warning: "#ef9f76",
  success: "#a6d189",
};

const mocha: ThemeTokens = {
  crust: "#11111b",
  mantle: "#181825",
  base: "#1e1e2e",
  surface0: "#313244",
  surface1: "#45475a",
  surface2: "#585b70",
  text: "#cdd6f4",
  subtext: "#bac2de",
  overlay: "#7f849c",
  accent: "#cba6f7",
  accentForeground: "#11111b",
  danger: "#f38ba8",
  warning: "#fab387",
  success: "#a6e3a1",
};

/**
 * Aeria's own default: cool ink neutrals with a teal accent and periwinkle
 * macro tokens, tuned to sit on top of the Windows Acrylic backdrop.
 */
const tide: ThemeTokens = {
  crust: "#0c0f14",
  mantle: "#11151b",
  base: "#161a21",
  surface0: "#212631",
  surface1: "#2e3441",
  surface2: "#3d4454",
  text: "#e3e7ee",
  subtext: "#adb5c3",
  overlay: "#737c8e",
  accent: "#5ec4bd",
  accentForeground: "#05201d",
  danger: "#f0808d",
  warning: "#e8b46b",
  success: "#7fcf9a",
  macro: "#a3b3ff",
};

export const themeRegistry: readonly ThemeDefinition[] = [
  ...editorThemes,
  { id: "aeria-tide", displayName: "Tide", family: "Aeria", appearance: "dark", source: "builtin", tokens: tide },
  {
    id: "aeria-graphite",
    displayName: "Graphite",
    family: "Aeria",
    appearance: "dark",
    source: "builtin",
    tokens: {
      ...macchiato,
      crust: "#101015",
      mantle: "#16161d",
      base: "#1d1d24",
      surface0: "#2a2a31",
      surface1: "#41414b",
      surface2: "#50505b",
      text: "#eeeeF3",
      subtext: "#c3c3cd",
      overlay: "#7a7a85",
      accent: "#9898a2",
      accentForeground: "#101015",
    },
  },
  {
    id: "aeria-night-violet",
    displayName: "Night Violet",
    family: "Aeria",
    appearance: "dark",
    source: "builtin",
    tokens: {
      ...macchiato,
      crust: "#101015",
      mantle: "#131319",
      base: "#16161d",
      surface0: "#2a2a31",
      surface1: "#41414b",
      surface2: "#50505b",
      text: "#eeeeF3",
      subtext: "#c3c3cd",
      overlay: "#7a7a85",
      accent: "#8e8e9b",
      accentForeground: "#101015",
    },
  },
  { id: "catppuccin-latte", displayName: "Latte", family: "Catppuccin", appearance: "light", source: "builtin", tokens: { ...latte, macro: "#1e66f5" } },
  { id: "catppuccin-frappe", displayName: "Frappé", family: "Catppuccin", appearance: "dark", source: "builtin", tokens: { ...frappe, macro: "#8caaee" } },
  { id: "catppuccin-macchiato", displayName: "Macchiato", family: "Catppuccin", appearance: "dark", source: "builtin", tokens: { ...macchiato, macro: "#8aadf4" } },
  { id: "catppuccin-mocha", displayName: "Mocha", family: "Catppuccin", appearance: "dark", source: "builtin", tokens: { ...mocha, macro: "#89b4fa" } },
  {
    id: "high-contrast-dark",
    displayName: "High Contrast Dark",
    family: "Accessibility",
    appearance: "highContrast",
    source: "builtin",
    tokens: {
      crust: "#000000",
      mantle: "#000000",
      base: "#000000",
      surface0: "#1f1f1f",
      surface1: "#5f5f5f",
      surface2: "#8f8f8f",
      text: "#ffffff",
      subtext: "#f0f0f0",
      overlay: "#c0c0c0",
      accent: "#00ffff",
      accentForeground: "#000000",
      danger: "#ff6b6b",
      warning: "#ffff00",
      success: "#00ff00",
      macro: "#ffff00",
    },
  },
];

export const defaultThemeId = "one-dark-pro";

export function findTheme(themeId: string): ThemeDefinition {
  return themeRegistry.find((theme) => theme.id === themeId) ?? themeRegistry[0]!;
}
