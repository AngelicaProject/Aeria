import type { ReviewState } from "../types";

export function reviewLabel(state: ReviewState | null): string {
  switch (state) {
    case "draft": return "Draft";
    case "needsReview": return "Needs review";
    case "reviewed": return "Reviewed";
    case null: return "Untranslated";
  }
}

/** Colour-coded review marker. Use `decorative` when a visible label sits next to it. */
export function ReviewDot({ state, decorative = false }: { state: ReviewState | null; decorative?: boolean }) {
  return decorative
    ? <span className={`review-dot review-dot-${state ?? "none"}`} aria-hidden="true" />
    : <span className={`review-dot review-dot-${state ?? "none"}`} role="img" aria-label={reviewLabel(state)} title={reviewLabel(state)} />;
}
