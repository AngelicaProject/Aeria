const units: Array<[Intl.RelativeTimeFormatUnit, number]> = [
  ["year", 365 * 24 * 3600_000],
  ["month", 30 * 24 * 3600_000],
  ["week", 7 * 24 * 3600_000],
  ["day", 24 * 3600_000],
  ["hour", 3600_000],
  ["minute", 60_000],
];

/**
 * Coarse "3 days ago" label; anything under a minute is `justNow`. Callers
 * pass the interface locale rather than the OS locale.
 */
export function formatRelativeTime(timestampMs: number, nowMs: number, locale: string, justNow: string): string {
  const elapsed = nowMs - timestampMs;
  if (Math.abs(elapsed) < 60_000) return justNow;
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  for (const [unit, size] of units) {
    if (Math.abs(elapsed) >= size) return formatter.format(-Math.round(elapsed / size), unit);
  }
  return formatter.format(-Math.round(elapsed / 60_000), "minute");
}
