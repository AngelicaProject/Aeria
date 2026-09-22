import { memo, useMemo } from "react";
import { bindingKey } from "../binding";
import { flattenTranslationRows, type TranslationOccurrenceView } from "../translationOccurrences";
import type { SourceBinding, TranslationRowDto } from "../types";

type TranslationListProps = {
  rows: TranslationRowDto[];
  selectedBinding: SourceBinding | null;
  selectedSheetName: string | null;
  loadedSheetName: string | null;
  disabled: boolean;
  loading: boolean;
  refreshing: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  onSelect: (occurrence: TranslationOccurrenceView) => void;
  onLoadMore: () => void;
};

function rowCoordinate(occurrence: TranslationOccurrenceView): string {
  return `${occurrence.binding.rowId}:${occurrence.binding.subrowId}`;
}

function fieldLabel(occurrence: TranslationOccurrenceView): string {
  return `col ${occurrence.binding.columnIndex}`;
}

export const TranslationList = memo(function TranslationList({
  rows,
  selectedBinding,
  selectedSheetName,
  loadedSheetName,
  disabled,
  loading,
  refreshing,
  loadingMore,
  hasMore,
  onSelect,
  onLoadMore,
}: TranslationListProps) {
  const occurrences = useMemo(() => flattenTranslationRows(rows), [rows]);
  const selectedKey = selectedBinding ? bindingKey(selectedBinding) : null;
  const showingPreviousSheet = loading && rows.length > 0 && selectedSheetName !== loadedSheetName;

  return (
    <section className="pane entry-pane" aria-labelledby="entries-heading" aria-busy={loading}>
      <div className="pane-heading">
        <div>
          <h2 id="entries-heading">Strings Lens</h2>
          <span className="pane-subtitle">{selectedSheetName ?? "Select a sheet"}</span>
        </div>
        <span className="pane-count">{rows.length} rows loaded</span>
      </div>

      {loading && rows.length === 0 ? (
        <div className="list-state" aria-live="polite">
          <span className="spinner" aria-hidden="true" />
          Loading sheet…
        </div>
      ) : occurrences.length === 0 ? (
        <div className="empty-pane">
          <strong>{hasMore ? "No translatable text in this page" : "No source strings"}</strong>
          <p>{hasMore ? "Load more to continue through the source rows." : "This sheet has no entries to translate."}</p>
          {hasMore ? (
            <button className="secondary-button load-more" type="button" onClick={onLoadMore} disabled={disabled || loadingMore || refreshing}>
              {loadingMore ? "Loading…" : "Load more"}
            </button>
          ) : null}
        </div>
      ) : (
        <>
          <div className="entry-list-header" aria-hidden="true">
            <span>Row</span>
            <span>Field</span>
            <span>Source</span>
            <span>Target</span>
            <span>Review</span>
          </div>
          <div className={refreshing ? "entry-list is-refreshing" : "entry-list"}>
            {occurrences.map((occurrence) => {
              const selected = selectedKey === bindingKey(occurrence.binding);
              const reviewLabel = occurrence.reviewState === "needsReview" ? "Needs review" : occurrence.reviewState === "reviewed" ? "Reviewed" : occurrence.reviewState === "draft" ? "Draft" : "—";
              return (
                <button
                  className={`entry-row ${occurrence.firstInRow ? "row-group-first" : "row-group-middle"} ${occurrence.lastInRow ? "row-group-last" : ""} ${selected ? "active" : ""}`}
                  type="button"
                  key={bindingKey(occurrence.binding)}
                  aria-pressed={selected}
                  disabled={disabled}
                  onClick={() => onSelect(occurrence)}
                >
                  <span className="entry-row-coordinate">{occurrence.firstInRow ? rowCoordinate(occurrence) : ""}</span>
                  <span className="entry-field-cell">
                    <span className="entry-field-label">{fieldLabel(occurrence)}</span>
                    {occurrence.firstInRow && occurrence.fieldCountInRow > 1 ? <small>{occurrence.fieldCountInRow} fields</small> : null}
                  </span>
                  <span className="entry-preview" title={occurrence.sourceMacro}>{occurrence.sourceMacro || "(empty source macro)"}</span>
                  <span className={occurrence.targetMacro === null ? "entry-target-preview empty" : "entry-target-preview"} title={occurrence.targetMacro ?? "Untranslated"}>{occurrence.targetMacro ?? "—"}</span>
                  <span className={`entry-review ${occurrence.reviewState ? `review-${occurrence.reviewState}` : "entry-review-empty"}`} aria-label={reviewLabel}>{reviewLabel}</span>
                </button>
              );
            })}
          </div>
          {hasMore ? (
            <div className="load-more-wrap">
              <button className="secondary-button load-more" type="button" onClick={onLoadMore} disabled={disabled || loadingMore || refreshing}>
                {loadingMore ? "Loading…" : "Load more"}
              </button>
            </div>
          ) : null}
        </>
      )}
      {showingPreviousSheet ? (
        <div className="pane-loading-layer" aria-live="polite">
          <span className="spinner" aria-hidden="true" />
          Loading {selectedSheetName}…
        </div>
      ) : null}
    </section>
  );
});
