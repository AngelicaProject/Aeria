import { memo, useEffect, useMemo, useRef, type KeyboardEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { bindingKey, domKey } from "../binding";
import { segmentMacroText } from "../macroTokens";
import { emptyOccurrenceFilter, isOccurrenceFilterActive, type OccurrenceFilter, type OccurrenceKindFilter, type OccurrenceStatusFilter, type TranslationOccurrenceView } from "../translationOccurrences";
import type { SourceBinding, UnitChangeKind } from "../types";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ReviewDot } from "./ReviewDot";
import { usePreferences } from "../ui/preferences";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

const ROW_HEIGHT = { compact: 28, comfortable: 34 } as const;

type TranslationListProps = {
  occurrences: readonly TranslationOccurrenceView[];
  loadedOccurrenceCount: number;
  /** Sheet-wide coverage from the Workspace, not just loaded rows. */
  sheetProgress: { translated: number; reviewed: number; total: number } | null;
  filter: OccurrenceFilter;
  onFilterChange: (filter: OccurrenceFilter) => void;
  selectedBinding: SourceBinding | null;
  selectedSheetName: string | null;
  loadedSheetName: string | null;
  disabled: boolean;
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  onSelect: (occurrence: TranslationOccurrenceView) => void;
  onNavigate: (direction: 1 | -1) => void;
  onLoadMore: () => void;
  /** Uncommitted change kind per binding key, for Git markers. */
  changedKinds: ReadonlyMap<string, UnitChangeKind>;
  /** Set when the list was paged to a string mid-sheet. */
  listStart: { rowId: number; subrowId: number } | null;
  onLoadFromStart: () => void;
};

const statusOptions: ReadonlyArray<{ value: OccurrenceStatusFilter; label: MessageKey }> = [
  { value: "all", label: "list.all" },
  { value: "untranslated", label: "review.untranslated" },
  { value: "draft", label: "review.draft" },
  { value: "needsReview", label: "review.needsReview" },
  { value: "reviewed", label: "review.reviewed" },
];

/** A toggle pair: selecting the active kind again shows every kind. */
const kindOptions: ReadonlyArray<{ value: Exclude<OccurrenceKindFilter, "all">; label: MessageKey; title: MessageKey }> = [
  { value: "text", label: "list.kind.text", title: "list.kind.textHint" },
  { value: "formatting", label: "list.formattingTag", title: "list.kind.formattingHint" },
];

const changedLabels: Readonly<Record<UnitChangeKind, MessageKey>> = {
  added: "list.changed.added",
  modified: "list.changed.modified",
  removed: "list.changed.removed",
};

function MacroPreview({ text, empty }: { text: string | null; empty: string }) {
  const { t } = useI18n();
  const segments = useMemo(() => text ? segmentMacroText(text.replace(/\s*\n\s*/g, " ")) : [], [text]);
  if (text === null) return <span className="lens-empty">{empty}</span>;
  if (text.length === 0) return <span className="lens-empty">{t("common.empty")}</span>;
  return <>{segments.map((segment, index) => segment.kind === "macro" ? <span className="lens-macro" key={index}>{segment.text}</span> : <span key={index}>{segment.text}</span>)}</>;
}

export const TranslationList = memo(function TranslationList({
  occurrences,
  loadedOccurrenceCount,
  sheetProgress,
  filter,
  onFilterChange,
  selectedBinding,
  selectedSheetName,
  loadedSheetName,
  disabled,
  loading,
  loadingMore,
  hasMore,
  onSelect,
  onNavigate,
  onLoadMore,
  changedKinds,
  listStart,
  onLoadFromStart,
}: TranslationListProps) {
  const { t, formatNumber } = useI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  const rowHeight = ROW_HEIGHT[usePreferences().preferences.listDensity];
  const selectedKey = selectedBinding ? bindingKey(selectedBinding) : null;
  const showingPreviousSheet = loading && loadedOccurrenceCount > 0 && selectedSheetName !== loadedSheetName;
  const filtered = isOccurrenceFilterActive(filter);
  const itemCount = occurrences.length + (hasMore ? 1 : 0);

  const virtualizer = useVirtualizer({
    count: itemCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
  });

  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, virtualizer]);

  const selectedIndex = useMemo(
    () => selectedKey === null ? -1 : occurrences.findIndex((occurrence) => bindingKey(occurrence.binding) === selectedKey),
    [occurrences, selectedKey],
  );

  useEffect(() => {
    if (selectedIndex >= 0) virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
  }, [selectedIndex, virtualizer]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [loadedSheetName]);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      onNavigate(event.key === "ArrowDown" ? 1 : -1);
    }
  }

  const translatedShare = sheetProgress && sheetProgress.total > 0 ? Math.min(1, sheetProgress.translated / sheetProgress.total) : 0;
  const reviewedShare = sheetProgress && sheetProgress.total > 0 ? Math.min(1, sheetProgress.reviewed / sheetProgress.total) : 0;

  return (
    <section className="lens" aria-label={t("list.label")} aria-busy={loading}>
      <div className="lens-toolbar">
        <label className="search-field lens-search">
          <UiIcon icon="search" size="sm" />
          <input
            value={filter.query}
            onChange={(event) => onFilterChange({ ...filter, query: event.target.value })}
            onKeyDown={(event) => { if (event.key === "Escape" && filter.query) { event.stopPropagation(); onFilterChange({ ...filter, query: "" }); } }}
            placeholder={t("list.filterPlaceholder")}
            aria-label={t("list.filterPlaceholder")}
            spellCheck={false}
          />
        </label>
        <Segmented label={t("list.statusFilter")} value={filter.status} onChange={(status) => onFilterChange({ ...filter, status })} options={statusOptions.map((option) => ({ value: option.value, label: option.value === "all" ? t(option.label) : <><ReviewDot decorative state={option.value === "untranslated" ? null : option.value} />{t(option.label)}</> }))} />
        <div className="segmented segmented-sm" role="group" aria-label={t("list.kindFilter")}>
          {kindOptions.map((option) => {
            const pressed = filter.kind === option.value;
            return (
              <button key={option.value} type="button" aria-pressed={pressed} title={t(option.title)} className={`segmented-option${pressed ? " checked" : ""}`} onClick={() => onFilterChange({ ...filter, kind: pressed ? "all" : option.value })}>
                {t(option.label)}
              </button>
            );
          })}
        </div>
        <span className="spacer" />
        {sheetProgress ? (
          <div className="lens-stats" title={t("list.stats", { translated: sheetProgress.translated, reviewed: sheetProgress.reviewed, total: sheetProgress.total })}>
            <span className="mono">{Math.floor(translatedShare * 100)}%</span>
            <span className="lens-stats-of">{formatNumber(sheetProgress.translated)} / {formatNumber(sheetProgress.total)}</span>
            <span className="meter" aria-hidden="true">
              <span className="meter-translated" style={{ width: `${translatedShare * 100}%` }} />
              <span className="meter-reviewed" style={{ width: `${reviewedShare * 100}%` }} />
            </span>
          </div>
        ) : null}
      </div>

      {listStart ? (
        <div className="lens-banner" role="status">
          <UiIcon icon="info" size="sm" />
          <span>{t("list.showingFrom")} <span className="mono">{listStart.rowId}:{listStart.subrowId}</span></span>
          <button className="link-button" type="button" onClick={onLoadFromStart} disabled={disabled || loading}>{t("list.loadFromStart")}</button>
        </div>
      ) : null}

      <div className="lens-header" aria-hidden="true">
        <span />
        <span>{t("list.header.row")}</span>
        <span>{t("list.header.field")}</span>
        <span>{t("list.header.source")}</span>
        <span>{t("list.header.target")}</span>
      </div>

      {loading && loadedOccurrenceCount === 0 ? (
        <div className="lens-state" aria-live="polite"><span className="spinner" aria-hidden="true" />{t("list.loadingSheet")}</div>
      ) : occurrences.length === 0 ? (
        <div className="lens-state">
          <div className="empty-state">
            <UiIcon icon={filtered ? "listFilter" : "table2"} size="xl" />
            <strong>{t(filtered ? "list.noMatch" : hasMore ? "list.noPageText" : "list.noStrings")}</strong>
            <p>{t(filtered ? "list.noMatchHint" : hasMore ? "list.noPageTextHint" : "list.noStringsHint")}</p>
            <div className="empty-state-actions">
              {filtered ? <button className="button button-secondary" type="button" onClick={() => onFilterChange(emptyOccurrenceFilter)}>{t("list.clearFilter")}</button> : null}
              {hasMore ? <button className="button button-secondary" type="button" onClick={onLoadMore} disabled={disabled || loadingMore}>{t(loadingMore ? "common.loading" : "list.loadMore")}</button> : null}
            </div>
          </div>
        </div>
      ) : (
        <div
          ref={scrollRef}
          className="lens-scroll"
          role="listbox"
          tabIndex={0}
          aria-label={t("list.labelFor", { sheet: loadedSheetName ?? t("list.sheetFallback") })}
          aria-activedescendant={selectedKey && selectedIndex >= 0 ? `lens-${domKey(selectedKey)}` : undefined}
          onKeyDown={handleKeyDown}
        >
          <div className="lens-spacer" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((item) => {
              const occurrence = occurrences[item.index];
              if (!occurrence) {
                return (
                  <div className="lens-load-more" key="load-more" style={{ transform: `translateY(${item.start}px)`, height: item.size }}>
                    <button className="button button-ghost" type="button" onClick={onLoadMore} disabled={disabled || loadingMore}>
                      {loadingMore ? <><span className="spinner spinner-xs" />{t("common.loading")}</> : <><UiIcon icon="arrowDown" size="sm" />{t("list.loadMoreRows")}</>}
                    </button>
                  </div>
                );
              }
              const key = bindingKey(occurrence.binding);
              const selected = key === selectedKey;
              const continuation = !occurrence.firstInRow && occurrences[item.index - 1]?.rowKey === occurrence.rowKey;
              const changeKind = changedKinds.get(key);
              return (
                <div
                  id={`lens-${domKey(key)}`}
                  className={`lens-row${selected ? " selected" : ""}${continuation ? " continuation" : ""}${changeKind ? ` git-${changeKind}` : ""}`}
                  title={changeKind ? t(changedLabels[changeKind]) : undefined}
                  role="option"
                  aria-selected={selected}
                  aria-disabled={disabled || undefined}
                  key={key}
                  style={{ transform: `translateY(${item.start}px)`, height: item.size }}
                  onClick={() => { if (!disabled) onSelect(occurrence); }}
                >
                  <span className="lens-status"><ReviewDot state={occurrence.reviewState} /></span>
                  <span className="lens-coord mono">{continuation ? "" : `${occurrence.binding.rowId}:${occurrence.binding.subrowId}`}</span>
                  <span className="lens-field mono">{t("common.column", { column: String(occurrence.binding.columnIndex) })}</span>
                  <span className="lens-text lens-source" title={occurrence.sourceMacro}>{occurrence.formattingOnly ? <span className="lens-tag" title={t("list.formattingHint")}>{t("list.formattingTag")}</span> : null}<MacroPreview text={occurrence.sourceMacro} empty="" /></span>
                  <span className="lens-text lens-target" title={occurrence.targetMacro ?? t("review.untranslated")}><MacroPreview text={occurrence.targetMacro} empty={t("review.untranslated")} /></span>
                </div>
              );
            })}
          </div>
        </div>
      )}
      {showingPreviousSheet ? (
        <div className="lens-loading-layer" aria-live="polite"><span className="spinner" aria-hidden="true" />{t("list.loadingNamed", { sheet: selectedSheetName ?? "" })}</div>
      ) : null}
    </section>
  );
});
