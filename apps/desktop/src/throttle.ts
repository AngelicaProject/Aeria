/**
 * Runs `run` for a stream of triggers at most once every `interval`
 * milliseconds: a trigger after a quiet interval runs at once, and the
 * triggers that come sooner run once, when the interval has passed. The
 * last trigger is never lost.
 */
export function throttled(run: () => void, interval: number): { trigger: () => void; cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let last = -Infinity;
  return {
    trigger() {
      if (timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        last = Date.now();
        run();
      }, Math.max(0, last + interval - Date.now()));
    },
    cancel() {
      clearTimeout(timer);
      timer = undefined;
    },
  };
}
