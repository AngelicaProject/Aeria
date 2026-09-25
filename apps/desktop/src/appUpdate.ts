import type { RunningActivity, UpdateStatusDto } from "./types";

/** Why an update cannot be installed right now. */
export type UpdateBlocker = RunningActivity | "unsavedDraft";

/**
 * Work that installing would interrupt: an unsaved editor draft first, then
 * the backend's running activities in their reported order.
 */
export function installBlockers(status: UpdateStatusDto, unsavedDraft: boolean): UpdateBlocker[] {
  return [...(unsavedDraft ? ["unsavedDraft" as const] : []), ...status.runningActivities];
}

/** Share of the download received, or `null` while the size is unknown. */
export function downloadShare(download: UpdateStatusDto["download"]): number | null {
  if (!download) return null;
  if (download.ready) return 1;
  if (!download.total || download.total <= 0) return null;
  return Math.min(1, download.received / download.total);
}

/** Whether to show the update card: a version is offered and was not put off this session. */
export function shouldOfferUpdate(status: UpdateStatusDto | null, postponedVersion: string | null): boolean {
  return Boolean(status?.available) && status?.available?.version !== postponedVersion;
}

/**
 * A nightly build that follows the stable channel keeps running until a
 * stable release newer than it is published; Aeria never downgrades.
 */
export function awaitsNewerStable(status: UpdateStatusDto): boolean {
  return status.channel === "stable" && /-nightly\./.test(status.currentVersion) && !status.available;
}

/** Formats a byte count in megabytes with one decimal. */
export function formatMegabytes(bytes: number, locale: string): string {
  return (bytes / (1024 * 1024)).toLocaleString(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
}
