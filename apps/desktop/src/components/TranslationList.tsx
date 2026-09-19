import { bindingKey } from "../binding";
import type { SourceBinding, TranslationEntryDto } from "../types";

type TranslationListProps = {
  entries: TranslationEntryDto[];
  selectedBinding: SourceBinding | null;
  disabled: boolean;
  loading: boolean;
  refreshing: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  onSelect: (entry: TranslationEntryDto) => void;
  onLoadMore: () => void;
};

function reviewLabel(entry: TranslationEntryDto): string {
  if (!entry.translation) {
    return "Untranslated";
  }

  switch (entry.translation.reviewState) {
    case "reviewed":
      return "Reviewed";
    case "needsReview":
      return "Needs review";
    case "draft":
      return "Draft";
  }
}

export function TranslationList({
  entries,
  selectedBinding,
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
          <h2 id="entries-heading">Translation entries</h2>
        </div>
        <span className="pane-count">{entries.length}</span>
      </div>

      {loading ? (
        <div className="list-state" aria-live="polite">
          <span className="spinner" aria-hidden="true" />
          Loading sheet…
        </div>
      ) : entries.length === 0 ? (
        <div className="empty-pane">
          <strong>No source strings</strong>
          <p>This sheet has no entries to translate.</p>
        </div>
      ) : (
        <>
          <div className={refreshing ? "entry-list is-refreshing" : "entry-list"}>
            {entries.map((entry) => {
              const selected = selectedBinding !== null && bindingKey(entry.sourceBinding) === bindingKey(selectedBinding);
              return (
                <button
                  className={selected ? "entry-row active" : "entry-row"}
                  type="button"
                  key={bindingKey(entry.sourceBinding)}
                  aria-pressed={selected}
                  disabled={disabled}
                  onClick={() => onSelect(entry)}
                >
                  <span className="entry-row-topline">
                    <code>
                      {entry.sourceBinding.rowId}:{entry.sourceBinding.subrowId}:{entry.sourceBinding.columnIndex}
                    </code>
                    <span className={entry.translation ? "status-pill translated" : "status-pill"}>
                      {reviewLabel(entry)}
                    </span>
                  </span>
                  <span className="entry-preview">{entry.sourceMacro || "(empty source macro)"}</span>
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
