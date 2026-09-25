/**
 * A tiny external store for a count that changes often, such as strings read
 * while a sheet streams in. Only components that subscribe re-render.
 */
export type LoadProgress = {
  current: () => number;
  set: (value: number) => void;
  subscribe: (listener: () => void) => () => void;
};

export function createLoadProgress(): LoadProgress {
  let value = 0;
  const listeners = new Set<() => void>();
  return {
    current: () => value,
    set: (next) => {
      if (next === value) return;
      value = next;
      for (const listener of listeners) listener();
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}
