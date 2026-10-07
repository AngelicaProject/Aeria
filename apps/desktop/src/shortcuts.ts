import type { MessageKey } from "./i18n/translate";

export type ShortcutGroup = "general" | "translation" | "navigation" | "layout";

export const shortcutGroupLabels: Readonly<Record<ShortcutGroup, MessageKey>> = {
  general: "shortcut.group.general",
  translation: "shortcut.group.translation",
  navigation: "shortcut.group.navigation",
  layout: "shortcut.group.layout",
};

/** Documented keyboard shortcuts; the handlers live with the features that own them. */
export const keyboardShortcuts: ReadonlyArray<{ keys: string; action: MessageKey; group: ShortcutGroup }> = [
  { keys: "Ctrl+P", action: "shortcut.goToSheet", group: "general" },
  { keys: "Ctrl+Shift+P", action: "shortcut.showCommands", group: "general" },
  { keys: "Ctrl+G", action: "shortcut.goToRow", group: "general" },
  { keys: "Ctrl+,", action: "shortcut.openSettings", group: "general" },
  { keys: "Ctrl+S", action: "shortcut.saveTarget", group: "translation" },
  { keys: "Ctrl+Enter", action: "shortcut.saveAndNext", group: "translation" },
  { keys: "Ctrl+Shift+Enter", action: "shortcut.approveAndNext", group: "translation" },
  { keys: "Alt+Down", action: "shortcut.nextString", group: "navigation" },
  { keys: "Alt+Up", action: "shortcut.previousString", group: "navigation" },
  { keys: "Up / Down", action: "shortcut.moveInList", group: "navigation" },
  { keys: "Ctrl+F", action: "shortcut.filterSheets", group: "navigation" },
  { keys: "Ctrl+W", action: "shortcut.closeTab", group: "navigation" },
  { keys: "Ctrl+B", action: "shortcut.toggleLeft", group: "layout" },
  { keys: "Ctrl+J", action: "shortcut.toggleBottom", group: "layout" },
];

type KeyPress = { key: string; code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean };

/**
 * The key of a shortcut by its place on the keyboard, so Ctrl+P is the same
 * key in every layout: with a Russian layout `key` is "з" there, but `code`
 * stays "KeyP". Letters are lowercase; Enter is "enter".
 */
export function shortcutKey(event: Pick<KeyPress, "key" | "code">): string {
  const { code } = event;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (code === "Comma") return ",";
  if (code === "Enter" || code === "NumpadEnter") return "enter";
  return event.key.toLocaleLowerCase();
}

/** Ctrl with these keys edits text, so the web view keeps them. */
const EDITING_KEYS = new Set(["a", "c", "v", "x", "y", "z"]);

/**
 * Whether a key press would reach one of the web view's own browser
 * commands: print, reload, find, view source, save the page, history, going
 * back, and the like. Aeria is not a browser, so they are cancelled; its own
 * shortcuts run before this is asked. `devtools` keeps the developer tools
 * keys for development builds.
 */
export function isBrowserCommand(event: KeyPress, devtools: boolean): boolean {
  const key = shortcutKey(event);
  if (devtools && (key === "f12" || ((event.ctrlKey || event.metaKey) && event.shiftKey && (key === "i" || key === "j" || key === "c")))) return false;
  if (event.ctrlKey || event.metaKey) {
    if (event.altKey) return false;
    return /^[a-z0-9]$/.test(key) ? !EDITING_KEYS.has(key) : key === "f5" || key === "printscreen";
  }
  // Alt with an arrow goes back or forward in the web view's history.
  if (event.altKey) return key === "arrowleft" || key === "arrowright" || key === "home";
  return /^f([1-9]|1[0-2])$/.test(key) || key === "browserback" || key === "browserforward" || key === "browserrefresh";
}
