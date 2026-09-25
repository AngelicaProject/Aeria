export type Rect = { left: number; top: number; width: number; height: number };
export type Size = { width: number; height: number };

const GAP = 6;
const MARGIN = 8;

/**
 * Where a tooltip goes: centered below its anchor, above it when there is no
 * room below, and always kept inside the viewport.
 */
export function placeTooltip(anchor: Rect, tooltip: Size, viewport: Size): { left: number; top: number; side: "top" | "bottom" } {
  const below = anchor.top + anchor.height + GAP;
  const above = anchor.top - GAP - tooltip.height;
  const side = below + tooltip.height > viewport.height - MARGIN && above >= MARGIN ? "top" : "bottom";
  const centered = anchor.left + anchor.width / 2 - tooltip.width / 2;
  const left = Math.max(MARGIN, Math.min(centered, viewport.width - MARGIN - tooltip.width));
  const top = side === "top" ? above : Math.min(below, viewport.height - MARGIN - tooltip.height);
  return { left: Math.round(left), top: Math.round(Math.max(MARGIN, top)), side };
}
