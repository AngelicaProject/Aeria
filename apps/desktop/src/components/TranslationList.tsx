import { rowKey } from "../binding";
import type { TranslationRowCursorDto, TranslationRowDto } from "../types";

type TranslationListProps = {
  rows: TranslationRowDto[];
  selectedRow: TranslationRowCursorDto | null;
  disabled: boolean;
  loading: boolean;
  refreshing: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  onSelect: (row: TranslationRowDto) => void;
  onLoadMore: () => void;
};

function translatedCount(row: TranslationRowDto): number {
  return row.cells.filter((cell) => cell.translation !== null).length;
}

export function TranslationList({
  rows,
  selectedRow,
  disabled,
  loading,
  refreshing,
  loadingMore,
  hasMore,
  onSelect,
  onLoadMore,
}: TranslationListProps) {
  return (
    <section className="pane entry-pane" aria-labelledby="entries-heading">
      <div className="pane-heading">
        <div>
          <p className="pane-kicker">Source order</p>
          <h2 id="entries-heading">Translation rows</h2>
        </div>
        <span className="pane-count">{rows.length}</span>
      </div>

      {loading ? (
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
          <div className={refreshing ? "entry-list is-refreshing" : "entry-list"}>
            {rows.map((row) => {
              const selected = selectedRow !== null && rowKey(row) === rowKey(selectedRow);
              const translated = translatedCount(row);
              const preview = row.cells[0]?.sourceMacro ?? "(empty source macro)";
              return (
                <button
                  className={selected ? "entry-row active" : "entry-row"}
                  type="button"
                  key={rowKey(row)}
                  aria-pressed={selected}
                  disabled={disabled}
                  onClick={() => onSelect(row)}
                >
                  <span className="entry-row-topline">
                    <code>
                      {row.rowId}:{row.subrowId}
                    </code>
                    <span className="status-pill">
                      {translated}/{row.cells.length} translated
                    </span>
                  </span>
                  {row.context[0] ? <span className="entry-context">{row.context[0].sourceMacro}</span> : null}
                  {row.cells.length > 1 ? <span className="entry-field-count">{row.cells.length} text fields</span> : null}
                  <span className="entry-preview">{preview}</span>
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
    </section>
  );
}
