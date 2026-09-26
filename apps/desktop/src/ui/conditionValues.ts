import { useSyncExternalStore } from "react";
import type { ConditionValues } from "../previewConditions";

/**
 * Values chosen for preview conditions, shared by every string and kept
 * between sessions: set the class and level once and read every tooltip of
 * that class. Stored locally; never project data.
 */

const STORAGE_KEY = "aeria.previewConditionValues";

function read(): ConditionValues {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}") as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return Object.fromEntries(Object.entries(parsed).filter((entry): entry is [string, number] => Number.isInteger(entry[1]) && (entry[1] as number) >= 0));
  } catch {
    return {};
  }
}

let values: ConditionValues = typeof window === "undefined" ? {} : read();
const listeners = new Set<() => void>();

function publish(next: ConditionValues) {
  values = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(values));
  } catch {
    // Storage is a convenience; the values still apply for this session.
  }
  for (const listener of listeners) listener();
}

export function setConditionValue(key: string, value: number | null) {
  const next = { ...values };
  if (value === null) delete next[key];
  else next[key] = value;
  publish(next);
}

export function resetConditionValues(keys: readonly string[]) {
  const next = { ...values };
  for (const key of keys) delete next[key];
  publish(next);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useConditionValues(): ConditionValues {
  return useSyncExternalStore(subscribe, () => values);
}
