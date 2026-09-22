import { isTauri } from "@tauri-apps/api/core";

export function hasWindowsBackdrop(): boolean {
  return isTauri() && navigator.userAgent.includes("Windows");
}
