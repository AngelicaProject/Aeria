import { memo } from "react";
import { rowKey } from "../binding";
import type { ReviewState, TranslationRowCursorDto, TranslationRowDto } from "../types";

type TranslationListProps = {
  rows: TranslationRowDto[];
  selectedRow: TranslationRowCursorDto | null;
  selectedSheetName: string | null;
  loadedSheetName: string | null;
  disabled: boolean;
  loading: boolean;
  refreshing: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  onSelect: (row: TranslationRowDto) => void;
  onLoadMore: () => void;
};

const reviewLabels: Record<ReviewState, string> = {
  draft: "Draft",
  reviewed: "Reviewed",
  needsReview: "Needs review",
};

function rowPreview(row: TranslationRowDto, target: boolean): string {
  return row.cells
    .map((cell) => target ? cell.translation?.targetMacro ?? "—" : cell.sourceMacro)
    .join(" · ");
}

export const TranslationList = memo(function TranslationList({
  rows,
  selectedRow,
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
  const showingPreviousSheet = loading && rows.length > 0 && selectedSheetName !== loadedSheetName;

  return (
    <section className="pane entry-pane" aria-labelledby="entries-heading" aria-busy={loading}>
      <div className="pane-heading">
        <div>
          <h2 id="entries-heading">Translation rows</h2>
          <span className="pane-subtitle">{selectedSheetName ?? "Select a sheet"}</span>
        </div>
        <span className="pane-count">{rows.length}</span>
      </div>

      {loading && rows.length === 0 ? (
        <div className="list-state" aria-live="polite">
          <span className="spinner" aria-hidden="true" />
          Loading sheet…
        </div>
      ) : rows.length === 0 ? (
        <div className="empty-pane">
          <strong>{hasMore ? "No translatable text in this page" : "No source strings"}</strong>
          <p>
            {hasMore
              ? "Load more to continue through the source rows."
              : "This sheet has no entries to translate."}
          </p>
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
            <span>Source</span>
            <span>Target</span>
            <span>Review</span>
          </div>
          <div className={refreshing ? "entry-list is-refreshing" : "entry-list"}>
            {rows.map((row) => {
              const selected = selectedRow !== null && rowKey(row) === rowKey(selectedRow);
              const source = rowPreview(row, false) || "(empty source macro)";
              const target = rowPreview(row, true);
              const hasTarget = row.cells.some((cell) => Boolean(cell.translation?.targetMacro));
              const reviewStates = [...new Set(row.cells.flatMap((cell) => cell.translation ? [cell.translation.reviewState] : []))];
              return (
                <button
                  className={selected ? "entry-row active" : "entry-row"}
                  type="button"
                  key={rowKey(row)}
                  aria-pressed={selected}
                  disabled={disabled}
                  onClick={() => onSelect(row)}
                >
                  <code className="entry-row-coordinate">{row.rowId}:{row.subrowId}</code>
                  <span className="entry-preview" title={source}>
                    {source}
                    {row.cells.length > 1 ? <small>{row.cells.length} text fields</small> : null}
                  </span>
                  <span className={hasTarget ? "entry-target-preview" : "entry-target-preview empty"} title={hasTarget ? target : "No target saved"}>
                    {target || "—"}
                    {row.cells.length > 1 ? <small>{row.cells.length} text fields</small> : null}
                  </span>
                  <span className="entry-review" aria-label={reviewStates.map((state) => reviewLabels[state]).join(", ") || "No review state"}>
                    {reviewStates.length > 0 ? reviewStates.map((state) => <small className={`review-chip review-${state}`} key={state}>{reviewLabels[state]}</small>) : <small className="entry-review-empty">—</small>}
                  </span>
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
