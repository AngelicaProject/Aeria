const WINDOWS_DEVICE_PATH_PREFIX = /^\\\\[?.]\\/;

/**
 * Keeps Windows device-path details out of user-facing labels while leaving
 * the canonical path untouched for IPC and filesystem operations.
 */
export function displayPath(path: string): string {
  return path.replace(WINDOWS_DEVICE_PATH_PREFIX, "");
}

export function displayPathName(path: string): string {
  const normalized = displayPath(path);
  return normalized.split(/[\\/]/).filter(Boolean).at(-1) ?? normalized;
}

