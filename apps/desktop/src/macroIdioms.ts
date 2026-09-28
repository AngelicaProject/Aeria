import { useSyncExternalStore } from "react";
import { macroIdioms } from "./ipc";
import type { Idioms } from "./macroTokens";

/**
 * The idioms of `aeria_se::catalog`: constructs of several macros that read
 * as one value, such as the player's first name. Read once from Rust, the
 * authority for them; until then text shows their macros.
 */

let idioms: Idioms = new Map();
let requested = false;
const listeners = new Set<() => void>();

function load() {
  if (requested) return;
  requested = true;
  macroIdioms().then(
    (list) => {
      idioms = new Map(list.map((idiom) => [idiom.text, { name: idiom.name, summary: idiom.summary }]));
      for (const listener of listeners) listener();
    },
    () => { requested = false; },
  );
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The idioms by their exact macro text; empty until they are read. */
export function useMacroIdioms(): Idioms {
  load();
  return useSyncExternalStore(subscribe, () => idioms);
}
