import { useSyncExternalStore } from "react";
import type { PreviewValue } from "../types";

/**
 * Values chosen for the variables of game previews, shared by every string
 * and kept between sessions: set the class and level once and read every
 * tooltip of that class. Stored locally; never project data. Rust evaluates
 * the strings with them.
 */

export type PreviewValues = Readonly<Record<string, PreviewValue>>;

const STORAGE_KEY = "aeria.previewValues";

function isValue(value: unknown): value is PreviewValue {
  return typeof value === "string" || (typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff);
}

function read(): PreviewValues {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}") as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return Object.fromEntries(Object.entries(parsed).filter((entry): entry is [string, PreviewValue] => isValue(entry[1])));
  } catch {
    return {};
  }
}

let values: PreviewValues = typeof window === "undefined" ? {} : read();
const listeners = new Set<() => void>();

function publish(next: PreviewValues) {
  values = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(values));
  } catch {
    // Storage is a convenience; the values still apply for this session.
  }
  for (const listener of listeners) listener();
}

/** Sets a variable's value; `null` returns it to its default. */
export function setPreviewValue(key: string, value: PreviewValue | null) {
  const next = { ...values };
  if (value === null) delete next[key];
  else next[key] = value;
  publish(next);
}

export function resetPreviewValues(keys: readonly string[]) {
  const next = { ...values };
  for (const key of keys) delete next[key];
  publish(next);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePreviewValues(): PreviewValues {
  return useSyncExternalStore(subscribe, () => values);
}
