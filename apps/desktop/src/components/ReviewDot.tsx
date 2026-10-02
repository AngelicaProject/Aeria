import type { MessageKey } from "../i18n/translate";
import type { StringState } from "../types";
import { useI18n } from "../ui/i18n";

export { stringState } from "../stringState";

export function stateLabel(state: StringState | null): MessageKey {
  switch (state) {
    case "translated": return "review.translated";
    case "fuzzy": return "review.fuzzy";
    case null: return "review.untranslated";
  }
}

/** Colour-coded string state marker. Use `decorative` when a visible label sits next to it. */
export function ReviewDot({ state, decorative = false }: { state: StringState | null; decorative?: boolean }) {
  const { t } = useI18n();
  return decorative
    ? <span className={`review-dot review-dot-${state ?? "none"}`} aria-hidden="true" />
    : <span className={`review-dot review-dot-${state ?? "none"}`} role="img" aria-label={t(stateLabel(state))} title={t(stateLabel(state))} />;
}
