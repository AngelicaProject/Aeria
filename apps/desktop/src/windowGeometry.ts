export type SavedGeometry = { width: number; height: number; maximized: boolean };
export type Size = { width: number; height: number };

/**
 * A saved workbench size worth restoring: a number pair no smaller than the
 * workbench minimum. A minimized window on Windows reports about 160×28, so a
 * size saved while minimized (or otherwise too small) is rejected rather than
 * restored as an unusable window.
 */
export function usableGeometry(value: unknown, minimum: Size): SavedGeometry | null {
  if (typeof value !== "object" || value === null) return null;
  const { width, height, maximized } = value as Partial<SavedGeometry>;
  if (typeof width !== "number" || typeof height !== "number" || typeof maximized !== "boolean") return null;
  if (!Number.isFinite(width) || !Number.isFinite(height)) return null;
  if (!maximized && (width < minimum.width || height < minimum.height)) return null;
  return { width, height, maximized };
}

/** The saved size, shrunk to fit the monitor when one is known. */
export function fitToMonitor(geometry: SavedGeometry, monitor: Size | null): Size {
  if (!monitor) return { width: geometry.width, height: geometry.height };
  return { width: Math.min(geometry.width, monitor.width), height: Math.min(geometry.height, monitor.height) };
}
