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
  { keys: "Alt+Down", action: "shortcut.nextString", group: "navigation" },
  { keys: "Alt+Up", action: "shortcut.previousString", group: "navigation" },
  { keys: "Up / Down", action: "shortcut.moveInList", group: "navigation" },
  { keys: "Ctrl+F", action: "shortcut.filterSheets", group: "navigation" },
  { keys: "Ctrl+W", action: "shortcut.closeTab", group: "navigation" },
  { keys: "Ctrl+B", action: "shortcut.toggleLeft", group: "layout" },
  { keys: "Ctrl+J", action: "shortcut.toggleBottom", group: "layout" },
];
