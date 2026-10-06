import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useReducer, useRef, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { defaultRangeExtractor, useVirtualizer, type Range } from "@tanstack/react-virtual";
import { describeIssue, exceptionTerm } from "../issueText";
import { nextRow, type Chosen, type ResultRow, type SheetGroup } from "../searchResults";
import type { SearchHitDto } from "../types";
import { useI18n } from "../ui/i18n";
import { NameSheetMark } from "../ui/NameSheetMark";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";

/** Characters of a long text shown from its first match on. */
const SNIPPET = 160;
/** Characters before the first match kept for context: a narrow dock shows about forty in all. */
const LEAD = 16;

/** Splits `text` into plain and highlighted parts by UTF-16 `ranges`, shortened to start near the first match. */
export function highlight(text: string, ranges: readonly (readonly [number, number])[]): ReactNode[] {
  const first = ranges[0]?.[0] ?? 0;
  const start = first > LEAD * 2 ? first - LEAD : 0;
  const end = Math.min(text.length, start + SNIPPET);
  const parts: ReactNode[] = [];
  let at = start;
  if (start > 0) parts.push("…");
  for (const [from, to] of ranges) {
    if (to <= at || from >= end) continue;
    const begin = Math.max(from, at);
    if (begin > at) parts.push(text.slice(at, begin));
    parts.push(<mark key={`${from}:${to}`}>{text.slice(begin, Math.min(to, end))}</mark>);
    at = Math.min(to, end);
  }
  if (at < end) parts.push(text.slice(at, end));
  if (end < text.length) parts.push("…");
  return parts;
}

/** Where a string is in its sheet: row and subrow, and the column when it is not the first. */
function coordinateOf(hit: SearchHitDto): string {
  const binding = hit.binding;
  if (!binding) return "";
  return binding.columnIndex === 0 ? `${binding.rowId}:${binding.subrowId}` : `${binding.rowId}:${binding.subrowId} · ${binding.columnIndex}`;
}

/** The key that adds to a choice with a click: Cmd on macOS, Ctrl elsewhere. */
export const chooseKey = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "Cmd" : "Ctrl";

export type SearchResultListHandle = {
  /** Moves the keyboard into the list, on the first string. */
  focus: () => void;
};

type SearchResultListProps = {
  rows: readonly ResultRow[];
  chosen: Chosen;
  /** How many strings of the files are chosen. */
  chosenOf: (paths: readonly string[]) => number;
  nameSheets: ReadonlySet<string>;
  replacing: boolean;
  disabled: boolean;
  onToggleGroup: (group: SheetGroup) => void;
  onShowAll: (group: SheetGroup) => void;
  onReveal: (hit: SearchHitDto) => void;
  onChoose: (hits: readonly SearchHitDto[], on: boolean) => void;
  onChooseGroup: (group: SheetGroup, on: boolean) => void;
  onReplaceOne: (hit: SearchHitDto) => void;
  onReplaceGroup: (group: SheetGroup) => void;
  onExcept: (hit: SearchHitDto, term: string) => void;
  onRetranslate: (hit: SearchHitDto) => void;
  onFix: (hit: SearchHitDto) => void;
  /** Leaves the list for the search field. */
  onExit: () => void;
  /** Strings were chosen with Ctrl or Shift: the person knows the keys. */
  onKeysUsed: () => void;
};

/**
 * The strings found, as one virtualized list grouped by sheet. The sheet a
 * string is in stays at the top while its strings scroll. The keyboard moves
 * through the list: Up and Down, Enter opens a string or a sheet, Space
 * chooses, Left and Right close and open a sheet, Escape goes back to the
 * search field.
 */
export const SearchResultList = forwardRef<SearchResultListHandle, SearchResultListProps>(function SearchResultList(props, ref) {
  const { rows, chosen, chosenOf, nameSheets, replacing, disabled } = props;
  const { t } = useI18n();
  const listRef = useRef<HTMLDivElement | null>(null);
  const activeKey = useRef<string | null>(null);
  // The active row is kept in a ref, and this renders the list when it moves.
  const [, renderActive] = useReducer((tick: number) => tick + 1, 0);

  const sheetIndexes = useMemo(() => rows.flatMap((row, index) => (row.kind === "sheet" ? [index] : [])), [rows]);
  const stickyIndex = useRef(-1);
  // A sheet's line stays at the top only while its strings scroll under it:
  // the line at the top is one of its strings, not a sheet of its own.
  const rangeExtractor = useCallback((range: Range) => {
    let sticky = -1;
    if (rows[range.startIndex]?.kind !== "sheet") {
      for (const index of sheetIndexes) {
        if (index > range.startIndex) break;
        sticky = index;
      }
    }
    stickyIndex.current = sticky;
    const indexes = defaultRangeExtractor(range);
    return sticky < 0 || indexes.includes(sticky) ? indexes : [sticky, ...indexes];
  }, [rows, sheetIndexes]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => listRef.current,
    estimateSize: (index) => (rows[index]?.kind === "hit" ? 48 : 26),
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 12,
    rangeExtractor,
  });

  const activeIndex = activeKey.current === null ? -1 : rows.findIndex((row) => row.key === activeKey.current);
  const choosing = chosen.size > 0;
  // Where a range chosen with Shift starts: the string last clicked or chosen.
  const anchorKey = useRef<string | null>(null);

  const hitsBetween = (from: number, to: number): SearchHitDto[] => {
    const [start, end] = from <= to ? [from, to] : [to, from];
    return rows.slice(start, end + 1).flatMap((row) => (row.kind === "hit" ? [row.hit] : []));
  };

  /** Ctrl or Cmd chooses or unchooses one string; Shift chooses every string from the anchor. */
  const clickHit = (event: MouseEvent, index: number, row: Extract<ResultRow, { kind: "hit" }>) => {
    activeKey.current = row.key;
    renderActive();
    if (event.shiftKey || event.ctrlKey || event.metaKey) props.onKeysUsed();
    if (event.shiftKey) {
      const anchor = anchorKey.current === null ? -1 : rows.findIndex((other) => other.key === anchorKey.current);
      props.onChoose(hitsBetween(anchor >= 0 ? anchor : index, index), true);
      return;
    }
    anchorKey.current = row.key;
    if (event.ctrlKey || event.metaKey) props.onChoose([row.hit], !chosen.has(row.key));
    else props.onReveal(row.hit);
  };

  const activate = useCallback((index: number) => {
    const row = rows[index];
    if (!row) return;
    activeKey.current = row.key;
    renderActive();
    virtualizer.scrollToIndex(index, { align: "auto" });
  }, [rows, renderActive, virtualizer]);

  useImperativeHandle(ref, () => ({
    focus: () => {
      listRef.current?.focus();
      const start = activeIndex >= 0 ? activeIndex : rows.findIndex((row) => row.kind === "hit");
      activate(start >= 0 ? start : 0);
    },
  }), [activate, activeIndex, rows]);

  // A result read again keeps the active string when it is still there.
  useEffect(() => {
    if (activeKey.current !== null && !rows.some((row) => row.key === activeKey.current)) {
      activeKey.current = null;
      renderActive();
    }
  }, [rows, renderActive]);

  const open = (row: ResultRow) => {
    if (row.kind === "hit") props.onReveal(row.hit);
    else if (row.kind === "sheet") props.onToggleGroup(row.group);
    else if (row.kind === "more") props.onShowAll(row.group);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const row = rows[activeIndex];
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp": {
        const step = event.key === "ArrowDown" ? 1 : -1;
        const next = nextRow(rows, activeIndex, step);
        if (step < 0 && (activeIndex <= 0 || next === activeIndex)) {
          props.onExit();
          break;
        }
        // Shift extends the choice to the strings moved over.
        if (event.shiftKey && activeIndex >= 0) {
          props.onChoose(hitsBetween(activeIndex, next), true);
          props.onKeysUsed();
        }
        activate(next);
        break;
      }
      case "Home":
        activate(0);
        break;
      case "End":
        activate(nextRow(rows, rows.length, -1));
        break;
      case "Enter":
        if (row) open(row);
        break;
      case " ":
        if (row?.kind === "hit") {
          anchorKey.current = row.key;
          props.onChoose([row.hit], !chosen.has(row.key));
        }
        else if (row?.kind === "sheet") props.onChooseGroup(row.group, chosenOf(row.group.files.map((file) => file.path)) < row.group.count);
        break;
      case "ArrowLeft":
        if (row?.kind === "sheet" && row.open) props.onToggleGroup(row.group);
        else if (row && row.kind !== "sheet") activate(rows.findIndex((other) => other.kind === "sheet" && other.group.key === row.group.key));
        break;
      case "ArrowRight":
        if (row?.kind === "sheet" && !row.open) props.onToggleGroup(row.group);
        break;
      case "Escape":
        props.onExit();
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const items = virtualizer.getVirtualItems();
  return (
    <div
      ref={listRef}
      className={choosing ? "search-list choosing" : "search-list"}
      role="listbox"
      tabIndex={0}
      aria-label={t("search.listLabel")}
      aria-multiselectable
      aria-activedescendant={activeIndex >= 0 ? `search-row-${activeIndex}` : undefined}
      onKeyDown={onKeyDown}
    >
      <div className="search-list-inner" style={{ height: virtualizer.getTotalSize() }}>
        {items.map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          const sticky = item.index === stickyIndex.current;
          const active = item.index === activeIndex;
          const style = sticky
            ? { position: "sticky" as const, top: 0, zIndex: 2 }
            : { position: "absolute" as const, top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` };
          const common = {
            id: `search-row-${item.index}`,
            "data-index": item.index,
            ref: sticky ? undefined : virtualizer.measureElement,
            style,
          };
          if (row.kind === "sheet") {
            const { group } = row;
            const paths = group.files.map((file) => file.path);
            const chosenHere = chosenOf(paths);
            const whole = group.count > 0 && chosenHere >= group.count;
            const folder = group.files.length > 1 ? null : group.files[0]?.path.replace(/\.po$/, "");
            return (
              <div key={row.key} {...common} role="option" aria-selected={whole} aria-expanded={row.open} className={`search-sheet${active ? " active" : ""}${sticky ? " sticky" : ""}${whole ? " chosen" : chosenHere > 0 ? " partly" : ""}`}>
                <button
                  type="button"
                  tabIndex={-1}
                  className="search-sheet-toggle"
                  title={paths.length > 1 ? paths.join("\n") : undefined}
                  onClick={(event) => {
                    activeKey.current = row.key;
                    renderActive();
                    if ((event.ctrlKey || event.metaKey) && !disabled) {
                      props.onChooseGroup(group, !whole);
                      props.onKeysUsed();
                    } else props.onToggleGroup(group);
                  }}
                >
                  <UiIcon icon={row.open ? "chevronDown" : "chevronRight"} size="xs" />
                  <span className="search-sheet-name">{group.sheet}</span>
                  {nameSheets.has(group.sheet) ? <NameSheetMark /> : null}
                  {group.files.length > 1 ? (
                    <span className="search-sheet-path">{t("search.files", { count: group.files.length })}</span>
                  ) : folder && folder !== group.sheet ? <span className="search-sheet-path">{folder}</span> : null}
                  <span className="search-sheet-count">{chosenHere > 0 ? `${chosenHere} / ${group.count}` : group.count}</span>
                </button>
                {replacing ? <IconButton icon="replaceAll" size="xs" tabIndex={-1} className="search-row-action" label={t("search.replaceInSheet")} disabled={disabled} onClick={() => props.onReplaceGroup(group)} /> : null}
                <IconButton
                  icon={whole ? "circleCheck" : "circle"}
                  size="xs"
                  tabIndex={-1}
                  className="search-row-action"
                  label={whole ? t("search.unchooseSheet") : t("search.chooseSheet")}
                  shortcut={t("search.modClick", { key: chooseKey })}
                  disabled={disabled}
                  onClick={() => props.onChooseGroup(group, !whole)}
                />
              </div>
            );
          }
          if (row.kind === "hit") {
            const { hit } = row;
            const translationRanges = hit.matches.find((found) => found.field === "translation")?.ranges ?? [];
            const sourceRanges = hit.matches.find((found) => found.field === "source")?.ranges ?? [];
            const other = hit.matches.find((found) => found.field === "note" || found.field === "context");
            const findings = hit.findings.map((issue) => describeIssue(issue, t));
            const advice = hit.findings[0]?.advice ?? false;
            const term = hit.findings.map(exceptionTerm).find((found) => found !== null) ?? null;
            const isChosen = chosen.has(row.key);
            return (
              <div key={row.key} {...common} role="option" aria-selected={isChosen} className={`search-hit${isChosen ? " chosen" : ""}${active ? " active" : ""}`}>
                <button
                  type="button"
                  tabIndex={-1}
                  className="search-hit-main"
                  title={hit.context}
                  // Shift would select the text of the list instead of strings.
                  onMouseDown={(event) => { if (event.shiftKey) event.preventDefault(); }}
                  onClick={(event) => clickHit(event, item.index, row)}
                >
                  <span className="search-hit-line">
                    <span className="search-hit-text">{hit.translation ? highlight(hit.translation, translationRanges) : <em className="search-hit-untranslated">{t("search.untranslated")}</em>}</span>
                    {hit.fuzzy ? <span className="chip chip-warn">{t("search.fuzzy")}</span> : null}
                    <span className="search-hit-coordinate">{coordinateOf(hit)}</span>
                  </span>
                  <span className="search-hit-source">{highlight(hit.source, sourceRanges)}</span>
                  {other ? (
                    <span className="search-hit-other">
                      <span className="search-hit-label">{t(other.field === "note" ? "search.field.note" : "search.field.context")}</span>
                      {highlight(other.field === "note" ? hit.note ?? "" : hit.context, other.ranges)}
                    </span>
                  ) : null}
                  {findings.length > 0 ? (
                    <span className={advice ? "search-hit-findings advice" : "search-hit-findings"} title={findings.join("\n")}>
                      <UiIcon icon={advice ? "info" : "circleAlert"} size="xs" />
                      <span className="search-hit-finding">{findings[0]}</span>
                      {findings.length > 1 ? <span className="search-issue-count">{t("search.moreFindings", { count: findings.length - 1 })}</span> : null}
                    </span>
                  ) : null}
                </button>
                <span className="search-hit-actions">
                  <IconButton
                    icon={isChosen ? "circleCheck" : "circle"}
                    size="xs"
                    tabIndex={-1}
                    label={isChosen ? t("search.unchoose") : t("search.choose")}
                    shortcut={t("search.modClick", { key: chooseKey })}
                    onClick={() => { anchorKey.current = row.key; props.onChoose([hit], !isChosen); }}
                  />
                  {replacing && translationRanges.length > 0 ? <IconButton icon="replace" size="xs" tabIndex={-1} label={t("search.replaceOne")} disabled={disabled} onClick={() => props.onReplaceOne(hit)} /> : null}
                  {term !== null ? <IconButton icon="bookX" size="xs" tabIndex={-1} label={t("search.exceptOne", { term })} disabled={disabled} onClick={() => props.onExcept(hit, term)} /> : null}
                  {hit.translation ? <IconButton icon="wand" size="xs" tabIndex={-1} label={t("search.fixOne")} disabled={disabled} onClick={() => props.onFix(hit)} /> : null}
                  {hit.translation ? <IconButton icon="sparkles" size="xs" tabIndex={-1} label={t("search.retranslateOne")} disabled={disabled} onClick={() => props.onRetranslate(hit)} /> : null}
                </span>
              </div>
            );
          }
          if (row.kind === "more") {
            return (
              <div key={row.key} {...common} role="option" aria-selected={false} className={`search-note${active ? " active" : ""}`}>
                <button type="button" tabIndex={-1} className="link-button" onClick={() => props.onShowAll(row.group)}>
                  {row.shown === 0 ? t("search.showSheet", { count: row.group.count }) : t("search.showAll", { count: row.group.count })}
                </button>
              </div>
            );
          }
          return (
            <div key={row.key} {...common} className="search-note muted">
              {row.kind === "loading" ? <><span className="spinner spinner-xs" />{t("search.loadingFile")}</> : t("search.sheetCut", { shown: row.shown, count: row.group.count })}
            </div>
          );
        })}
      </div>
    </div>
  );
});
