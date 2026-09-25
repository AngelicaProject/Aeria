import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { updateStatus } from "../ipc";
import type { UpdateStatusDto } from "../types";

/** The backend emits the full status whenever the update state changes. */
const STATUS_EVENT = "update://status";

export type AppUpdateSnapshot = {
  status: UpdateStatusDto | null;
  /** The editor holds a draft that installing would lose. */
  unsavedDraft: boolean;
  /** The version the user put off with "Later" in this session. */
  postponedVersion: string | null;
};

let snapshot: AppUpdateSnapshot = { status: null, unsavedDraft: false, postponedVersion: null };
const listeners = new Set<() => void>();
let started = false;

function publish(next: Partial<AppUpdateSnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of listeners) listener();
}

function start(): void {
  if (started) return;
  started = true;
  void refreshUpdateStatus();
  void listen<UpdateStatusDto>(STATUS_EVENT, ({ payload }) => publish({ status: payload })).catch(() => undefined);
}

function subscribe(listener: () => void): () => void {
  start();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Re-reads the status, for example to notice that running work finished. */
export async function refreshUpdateStatus(): Promise<void> {
  try {
    publish({ status: await updateStatus() });
  } catch {
    // Outside the desktop shell there is no updater.
  }
}

export function setUpdateStatus(status: UpdateStatusDto): void {
  publish({ status });
}

export function reportUnsavedDraft(unsavedDraft: boolean): void {
  if (snapshot.unsavedDraft !== unsavedDraft) publish({ unsavedDraft });
}

/** Hides the update card until the next launch or a newer version. */
export function postponeUpdate(version: string): void {
  publish({ postponedVersion: version });
}

export function reopenUpdate(): void {
  publish({ postponedVersion: null });
}

export function useAppUpdate(): AppUpdateSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}
