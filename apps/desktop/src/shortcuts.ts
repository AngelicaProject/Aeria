export type ShortcutGroup = "General" | "Translation" | "Navigation" | "Layout";

/** Documented keyboard shortcuts; the handlers live with the features that own them. */
export const keyboardShortcuts: ReadonlyArray<{ keys: string; action: string; group: ShortcutGroup }> = [
  { keys: "Ctrl+P", action: "Go to sheet", group: "General" },
  { keys: "Ctrl+Shift+P", action: "Show all commands", group: "General" },
  { keys: "Ctrl+G", action: "Go to row in the current sheet", group: "General" },
  { keys: "Ctrl+,", action: "Open settings", group: "General" },
  { keys: "Ctrl+S", action: "Save target (in a note: save note)", group: "Translation" },
  { keys: "Ctrl+Enter", action: "Save target and go to the next string", group: "Translation" },
  { keys: "Alt+Down", action: "Next string", group: "Navigation" },
  { keys: "Alt+Up", action: "Previous string", group: "Navigation" },
  { keys: "Up / Down", action: "Move through the focused strings list", group: "Navigation" },
  { keys: "Ctrl+F", action: "Filter sheets", group: "Navigation" },
  { keys: "Ctrl+W", action: "Close the active sheet tab", group: "Navigation" },
  { keys: "Ctrl+B", action: "Toggle the left panel", group: "Layout" },
  { keys: "Ctrl+J", action: "Toggle the bottom panel", group: "Layout" },
];
