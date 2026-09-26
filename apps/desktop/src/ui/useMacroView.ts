import { useEffect, useRef, useState } from "react";
import { macroView } from "../ipc";
import type { MacroViewDto } from "../types";

export type MacroViewState = { text: string; view: MacroViewDto };

/** Requests closer together than this are typing, and wait for a pause. */
const TYPING_MS = 300;
const DEBOUNCE_MS = 80;
const CACHE_SIZE = 200;

/** Views of recently shown texts, so returning to a string shows it at once. */
const cache = new Map<string, MacroViewDto>();

function remember(text: string, view: MacroViewDto): void {
  cache.delete(text);
  cache.set(text, view);
  if (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
}

/**
 * The Rust description of `text`: diagnostics, tags, and a game preview.
 *
 * The previous view stays until the new one arrives, so switching strings
 * does not flash an empty preview; the returned view names the text it
 * describes. A view already seen is returned at once, a new text is
 * requested at once, and only rapid changes such as typing are debounced.
 */
export function useMacroView(text: string | null): MacroViewState | null {
  const [state, setState] = useState<MacroViewState | null>(null);
  const lastRequest = useRef(0);
  const cached = text === null ? undefined : cache.get(text);
  useEffect(() => {
    if (text === null || cached) return;
    let active = true;
    const now = Date.now();
    const delay = now - lastRequest.current < TYPING_MS ? DEBOUNCE_MS : 0;
    const timer = window.setTimeout(() => {
      lastRequest.current = Date.now();
      macroView(text)
        .then((view) => {
          remember(text, view);
          if (active) setState({ text, view });
        })
        .catch(() => undefined);
    }, delay);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [text, cached]);
  if (text === null) return null;
  if (cached) return { text, view: cached };
  return state;
}
