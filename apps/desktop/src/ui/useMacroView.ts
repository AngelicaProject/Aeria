import { useEffect, useState } from "react";
import { macroView } from "../ipc";
import type { MacroViewDto } from "../types";

export type MacroViewState = { text: string; view: MacroViewDto };

const DEBOUNCE_MS = 80;

/**
 * The Rust description of `text`: diagnostics, tags, and a game preview.
 * Requests are debounced while typing; the returned view names the text it
 * describes, so callers can ignore a view of older text.
 */
export function useMacroView(text: string | null): MacroViewState | null {
  const [state, setState] = useState<MacroViewState | null>(null);
  useEffect(() => {
    if (text === null) {
      setState(null);
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      macroView(text)
        .then((view) => { if (active) setState({ text, view }); })
        .catch(() => { if (active) setState(null); });
    }, DEBOUNCE_MS);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [text]);
  return state;
}
