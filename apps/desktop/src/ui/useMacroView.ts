import { useEffect, useRef, useState } from "react";
import { macroView } from "../ipc";
import type { MacroViewDto } from "../types";
import type { PreviewValues } from "./previewValues";

export type MacroViewState = { text: string; view: MacroViewDto };

/** Requests closer together than this are typing, and wait for a pause. */
const TYPING_MS = 300;
const DEBOUNCE_MS = 80;
const CACHE_SIZE = 200;

/** Views of recently shown texts, so returning to a string shows it at once. */
const cache = new Map<string, MacroViewDto>();

function remember(key: string, view: MacroViewDto): void {
  cache.delete(key);
  cache.set(key, view);
  if (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
}

function valuesKey(values: PreviewValues): string {
  return JSON.stringify(Object.keys(values).sort().map((key) => [key, values[key]]));
}

/**
 * The Rust description of `text`: diagnostics, tags, and a game preview
 * evaluated with the preview variable `values`.
 *
 * The previous view stays until the new one arrives, so switching strings
 * or changing a value does not flash an empty preview; the returned view
 * names the text it describes. A view already seen is returned at once, a
 * new text is requested at once, and only rapid changes such as typing are
 * debounced.
 */
export function useMacroView(text: string | null, values: PreviewValues): MacroViewState | null {
  const [state, setState] = useState<MacroViewState | null>(null);
  const lastRequest = useRef(0);
  const key = text === null ? null : `${valuesKey(values)}\n${text}`;
  const cached = key === null ? undefined : cache.get(key);
  useEffect(() => {
    if (text === null || key === null || cached) return;
    let active = true;
    const now = Date.now();
    const delay = now - lastRequest.current < TYPING_MS ? DEBOUNCE_MS : 0;
    const timer = window.setTimeout(() => {
      lastRequest.current = Date.now();
      macroView(text, values)
        .then((view) => {
          remember(key, view);
          if (active) setState({ text, view });
        })
        .catch(() => undefined);
    }, delay);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
    // `key` covers `text` and `values`.
  }, [key, cached]);
  if (text === null) return null;
  if (cached) return { text, view: cached };
  return state;
}
