import { useSyncExternalStore } from "react";
import type { ChipPick } from "../components/macroChipsExtension";
import type { SourceBinding } from "../types";

/**
 * The string the translation editor shows, for views beside it such as the
 * string guide: its source, the translation being typed, and a way to add
 * text to that translation at its cursor. The editor publishes it; readers
 * subscribe without re-rendering the workbench on every keystroke.
 */
export type EditorFocus = {
  binding: SourceBinding;
  source: string;
  draft: string;
  /** A save of the string is running; picks are refused meanwhile. */
  busy: boolean;
  /** Adds text to the translation at its cursor, as picking a source chip does. */
  apply: (pick: ChipPick) => void;
};

let focus: EditorFocus | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Publishes the editor's string, or `null` when it shows none. */
export function setEditorFocus(next: EditorFocus | null): void {
  if (next === focus) return;
  focus = next;
  for (const listener of listeners) listener();
}

/** Clears the focus only when `owner` still holds it, so a newer editor's focus stays. */
export function clearEditorFocus(owner: EditorFocus | null): void {
  if (owner !== null && focus === owner) setEditorFocus(null);
}

/** The editor's string; changes as the translation is typed. */
export function useEditorFocus(): EditorFocus | null {
  return useSyncExternalStore(subscribe, () => focus);
}
