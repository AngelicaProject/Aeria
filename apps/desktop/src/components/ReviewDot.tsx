import type { MessageKey } from "../i18n/translate";
import type { ReviewState } from "../types";
import { useI18n } from "../ui/i18n";

export function reviewLabel(state: ReviewState | null): MessageKey {
  switch (state) {
    case "draft": return "review.draft";
    case "needsReview": return "review.needsReview";
    case "reviewed": return "review.reviewed";
    case null: return "review.untranslated";
  }
}

/** Colour-coded review marker. Use `decorative` when a visible label sits next to it. */
export function ReviewDot({ state, decorative = false }: { state: ReviewState | null; decorative?: boolean }) {
  const { t } = useI18n();
  return decorative
    ? <span className={`review-dot review-dot-${state ?? "none"}`} aria-hidden="true" />
    : <span className={`review-dot review-dot-${state ?? "none"}`} role="img" aria-label={t(reviewLabel(state))} title={t(reviewLabel(state))} />;
}
