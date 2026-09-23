import type { ThemeDefinition, ThemeTokens } from "./registry.ts";

/**
 * Adaptations of widely used editor color themes. Each maps the upstream
 * palette onto Aeria's layered tokens: `crust`/`mantle` are the window and
 * sidebar backgrounds, `base` is the editor background, `surface0..2` are
 * raised fills, selections, and borders, and `overlay` is the comment color.
 */
type Palette = ThemeTokens;

function theme(id: string, displayName: string, family: string, appearance: ThemeDefinition["appearance"], tokens: Palette): ThemeDefinition {
  return { id, displayName, family, appearance, tokens, source: "builtin" };
}

export const editorThemes: readonly ThemeDefinition[] = [
  theme("one-dark-pro", "One Dark Pro", "One Dark", "dark", {
    crust: "#1b1e23", mantle: "#21252b", base: "#282c34",
    surface0: "#2c313a", surface1: "#3e4451", surface2: "#4b5263",
    text: "#d7dae0", subtext: "#abb2bf", overlay: "#7f848e",
    accent: "#61afef", accentForeground: "#1b1e23",
    danger: "#e06c75", warning: "#e5c07b", success: "#98c379", macro: "#c678dd",
  }),
  theme("dracula", "Dracula", "Dracula", "dark", {
    crust: "#191a21", mantle: "#21222c", base: "#282a36",
    surface0: "#343746", surface1: "#44475a", surface2: "#565a70",
    text: "#f8f8f2", subtext: "#d8d8e0", overlay: "#7b86b8",
    accent: "#bd93f9", accentForeground: "#191a21",
    danger: "#ff5555", warning: "#ffb86c", success: "#50fa7b", macro: "#ff79c6",
  }),
  theme("tokyo-night", "Tokyo Night", "Tokyo Night", "dark", {
    crust: "#12131a", mantle: "#16161e", base: "#1a1b26",
    surface0: "#232433", surface1: "#2f3549", surface2: "#414868",
    text: "#c0caf5", subtext: "#a9b1d6", overlay: "#6b7294",
    accent: "#7aa2f7", accentForeground: "#16161e",
    danger: "#f7768e", warning: "#e0af68", success: "#9ece6a", macro: "#bb9af7",
  }),
  theme("tokyo-night-storm", "Tokyo Night Storm", "Tokyo Night", "dark", {
    crust: "#1b1e2d", mantle: "#1f2335", base: "#24283b",
    surface0: "#292e42", surface1: "#3b4261", surface2: "#545c7e",
    text: "#c0caf5", subtext: "#a9b1d6", overlay: "#737aa2",
    accent: "#7aa2f7", accentForeground: "#1f2335",
    danger: "#f7768e", warning: "#e0af68", success: "#9ece6a", macro: "#bb9af7",
  }),
  theme("github-dark", "GitHub Dark", "GitHub", "dark", {
    crust: "#010409", mantle: "#090c10", base: "#0d1117",
    surface0: "#161b22", surface1: "#21262d", surface2: "#30363d",
    text: "#e6edf3", subtext: "#c9d1d9", overlay: "#7d8590",
    accent: "#4493f8", accentForeground: "#ffffff",
    danger: "#f85149", warning: "#d29922", success: "#3fb950", macro: "#d2a8ff",
  }),
  theme("github-dark-dimmed", "GitHub Dark Dimmed", "GitHub", "dark", {
    crust: "#1c2128", mantle: "#1f242b", base: "#22272e",
    surface0: "#2d333b", surface1: "#373e47", surface2: "#444c56",
    text: "#cdd9e5", subtext: "#adbac7", overlay: "#768390",
    accent: "#539bf5", accentForeground: "#ffffff",
    danger: "#e5534b", warning: "#c69026", success: "#57ab5a", macro: "#dcbdfb",
  }),
  theme("github-light", "GitHub Light", "GitHub", "light", {
    crust: "#e7ebef", mantle: "#f6f8fa", base: "#ffffff",
    surface0: "#eaeef2", surface1: "#d0d7de", surface2: "#afb8c1",
    text: "#1f2328", subtext: "#424a53", overlay: "#656d76",
    accent: "#0969da", accentForeground: "#ffffff",
    danger: "#d1242f", warning: "#9a6700", success: "#1a7f37", macro: "#8250df",
  }),
  theme("vscode-dark-modern", "Dark Modern", "Visual Studio Code", "dark", {
    crust: "#141414", mantle: "#181818", base: "#1f1f1f",
    surface0: "#2b2b2b", surface1: "#3c3c3c", surface2: "#4a4a4a",
    text: "#cccccc", subtext: "#b5b5b5", overlay: "#9d9d9d",
    accent: "#0078d4", accentForeground: "#ffffff",
    danger: "#f85149", warning: "#cca700", success: "#2ea043", macro: "#c586c0",
  }),
  theme("vscode-light-modern", "Light Modern", "Visual Studio Code", "light", {
    crust: "#ececec", mantle: "#f8f8f8", base: "#ffffff",
    surface0: "#e5e5e5", surface1: "#cecece", surface2: "#b5b5b5",
    text: "#3b3b3b", subtext: "#4f4f4f", overlay: "#767676",
    accent: "#005fb8", accentForeground: "#ffffff",
    danger: "#cd3131", warning: "#bf8803", success: "#388a34", macro: "#af00db",
  }),
  theme("nord", "Nord", "Nord", "dark", {
    crust: "#242933", mantle: "#292e39", base: "#2e3440",
    surface0: "#3b4252", surface1: "#434c5e", surface2: "#4c566a",
    text: "#eceff4", subtext: "#d8dee9", overlay: "#8791a6",
    accent: "#88c0d0", accentForeground: "#2e3440",
    danger: "#bf616a", warning: "#ebcb8b", success: "#a3be8c", macro: "#b48ead",
  }),
  theme("gruvbox-dark", "Gruvbox Dark", "Gruvbox", "dark", {
    crust: "#1d2021", mantle: "#232425", base: "#282828",
    surface0: "#32302f", surface1: "#504945", surface2: "#665c54",
    text: "#ebdbb2", subtext: "#d5c4a1", overlay: "#928374",
    accent: "#fabd2f", accentForeground: "#282828",
    danger: "#fb4934", warning: "#fe8019", success: "#b8bb26", macro: "#83a598",
  }),
  theme("monokai-pro", "Monokai Pro", "Monokai", "dark", {
    crust: "#19181a", mantle: "#221f22", base: "#2d2a2e",
    surface0: "#363337", surface1: "#403e41", surface2: "#5b595c",
    text: "#fcfcfa", subtext: "#c1c0c0", overlay: "#939293",
    accent: "#ffd866", accentForeground: "#2d2a2e",
    danger: "#ff6188", warning: "#fc9867", success: "#a9dc76", macro: "#ab9df2",
  }),
  theme("night-owl", "Night Owl", "Night Owl", "dark", {
    crust: "#010e1a", mantle: "#01111d", base: "#011627",
    surface0: "#0b2942", surface1: "#1d3b53", surface2: "#2c4a63",
    text: "#d6deeb", subtext: "#b2bfd1", overlay: "#6b8ba6",
    accent: "#82aaff", accentForeground: "#011627",
    danger: "#ef5350", warning: "#ecc48d", success: "#addb67", macro: "#c792ea",
  }),
  theme("rose-pine", "Rosé Pine", "Rosé Pine", "dark", {
    crust: "#12101a", mantle: "#16141f", base: "#191724",
    surface0: "#26233a", surface1: "#403d52", surface2: "#524f67",
    text: "#e0def4", subtext: "#908caa", overlay: "#6e6a86",
    accent: "#c4a7e7", accentForeground: "#191724",
    danger: "#eb6f92", warning: "#f6c177", success: "#9ccfd8", macro: "#ebbcba",
  }),
  theme("rose-pine-moon", "Rosé Pine Moon", "Rosé Pine", "dark", {
    crust: "#1c1a2c", mantle: "#201e31", base: "#232136",
    surface0: "#2a273f", surface1: "#44415a", surface2: "#56526e",
    text: "#e0def4", subtext: "#908caa", overlay: "#6e6a86",
    accent: "#c4a7e7", accentForeground: "#232136",
    danger: "#eb6f92", warning: "#f6c177", success: "#9ccfd8", macro: "#ea9a97",
  }),
  theme("rose-pine-dawn", "Rosé Pine Dawn", "Rosé Pine", "light", {
    crust: "#ebe2d8", mantle: "#f2e9e1", base: "#faf4ed",
    surface0: "#f0e7de", surface1: "#dfdad9", surface2: "#cecacd",
    text: "#575279", subtext: "#797593", overlay: "#9893a5",
    accent: "#286983", accentForeground: "#faf4ed",
    danger: "#b4637a", warning: "#ea9d34", success: "#56949f", macro: "#907aa9",
  }),
  theme("ayu-mirage", "Ayu Mirage", "Ayu", "dark", {
    crust: "#171b24", mantle: "#1c212b", base: "#242936",
    surface0: "#2b3140", surface1: "#394050", surface2: "#4a5263",
    text: "#cccac2", subtext: "#b3b1a9", overlay: "#707a8c",
    accent: "#ffcc66", accentForeground: "#242936",
    danger: "#f28779", warning: "#ffad66", success: "#87d96c", macro: "#dfbfff",
  }),
  theme("ayu-light", "Ayu Light", "Ayu", "light", {
    crust: "#e7e8e9", mantle: "#f3f4f5", base: "#fcfcfc",
    surface0: "#eceef0", surface1: "#d8dadd", surface2: "#c4c8cc",
    text: "#5c6166", subtext: "#6f757b", overlay: "#8a9199",
    accent: "#e59a2b", accentForeground: "#2b1d00",
    danger: "#e65050", warning: "#fa8d3e", success: "#6d9400", macro: "#a37acc",
  }),
  theme("solarized-dark", "Solarized Dark", "Solarized", "dark", {
    crust: "#00212b", mantle: "#002631", base: "#002b36",
    surface0: "#073642", surface1: "#0e4655", surface2: "#2b5763",
    text: "#c6cfcf", subtext: "#93a1a1", overlay: "#6b8189",
    accent: "#268bd2", accentForeground: "#fdf6e3",
    danger: "#dc322f", warning: "#b58900", success: "#859900", macro: "#6c71c4",
  }),
  theme("solarized-light", "Solarized Light", "Solarized", "light", {
    crust: "#e6dec6", mantle: "#eee8d5", base: "#fdf6e3",
    surface0: "#f0e9d5", surface1: "#ddd6c1", surface2: "#c9c2ab",
    text: "#3f545b", subtext: "#586e75", overlay: "#839496",
    accent: "#268bd2", accentForeground: "#fdf6e3",
    danger: "#dc322f", warning: "#b58900", success: "#859900", macro: "#6c71c4",
  }),
  theme("material-palenight", "Palenight", "Material", "dark", {
    crust: "#1b1e2b", mantle: "#202331", base: "#292d3e",
    surface0: "#32374d", surface1: "#444267", surface2: "#4e5579",
    text: "#d0d6f0", subtext: "#a6accd", overlay: "#7179a3",
    accent: "#c792ea", accentForeground: "#1b1e2b",
    danger: "#f07178", warning: "#ffcb6b", success: "#c3e88d", macro: "#89ddff",
  }),
  theme("kanagawa", "Kanagawa Wave", "Kanagawa", "dark", {
    crust: "#16161d", mantle: "#1a1a22", base: "#1f1f28",
    surface0: "#2a2a37", surface1: "#363646", surface2: "#54546d",
    text: "#dcd7ba", subtext: "#c8c093", overlay: "#727169",
    accent: "#7e9cd8", accentForeground: "#16161d",
    danger: "#e46876", warning: "#ffa066", success: "#98bb6c", macro: "#957fb8",
  }),
  theme("everforest-dark", "Everforest Dark", "Everforest", "dark", {
    crust: "#1e2326", mantle: "#232a2e", base: "#2d353b",
    surface0: "#343f44", surface1: "#475258", surface2: "#4f585e",
    text: "#d3c6aa", subtext: "#9da9a0", overlay: "#859289",
    accent: "#a7c080", accentForeground: "#2d353b",
    danger: "#e67e80", warning: "#dbbc7f", success: "#83c092", macro: "#d699b6",
  }),
];
