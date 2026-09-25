import { bindingKey, rowKey } from "./binding.ts";
import type { ReviewState, SourceBinding, TranslationRowDto } from "./types";

/**
 * Renderer-only projection of one translatable source cell.
 *
 * The IPC contract stays row based.  This projection deliberately makes the
 * cell, rather than the FFXIV row, the Lens selection unit.
 */
export type TranslationOccurrenceView = {
  rowKey: string;
  binding: SourceBinding;
  sourceMacro: string;
  formattingOnly: boolean;
  targetMacro: string | null;
  reviewState: ReviewState | null;
  fieldIndexInRow: number;
  fieldCountInRow: number;
  firstInRow: boolean;
  lastInRow: boolean;
};

/**
 * Row DTOs are immutable (an overlay update replaces the row object), so each
 * row's occurrences are cached by identity. Re-flattening a growing sheet then
 * only builds views for rows it has not seen.
 */
const rowOccurrences = new WeakMap<TranslationRowDto, readonly TranslationOccurrenceView[]>();

export function flattenTranslationRows(rows: readonly TranslationRowDto[]): TranslationOccurrenceView[] {
  const occurrences: TranslationOccurrenceView[] = [];
  for (const row of rows) {
    let views = rowOccurrences.get(row);
    if (!views) {
      views = rowViews(row);
      rowOccurrences.set(row, views);
    }
    for (const view of views) occurrences.push(view);
  }
  return occurrences;
}

function rowViews(row: TranslationRowDto): TranslationOccurrenceView[] {
  const fieldCountInRow = row.cells.length;
  const key = rowKey(row);
  return row.cells.map((cell, fieldIndexInRow) => ({
    rowKey: key,
    binding: cell.sourceBinding,
    sourceMacro: cell.sourceMacro,
    formattingOnly: cell.formattingOnly,
    targetMacro: cell.translation?.targetMacro ?? null,
    reviewState: cell.translation?.reviewState ?? null,
    fieldIndexInRow,
    fieldCountInRow,
    firstInRow: fieldIndexInRow === 0,
    lastInRow: fieldIndexInRow === fieldCountInRow - 1,
  }));
}

export function occurrenceKey(occurrence: Pick<TranslationOccurrenceView, "binding">): string {
  return bindingKey(occurrence.binding);
}

export type OccurrenceStatusFilter = "all" | "untranslated" | "draft" | "needsReview" | "reviewed";

/** Prose strings, or formatting-only strings (punctuation, digits, number formatting). */
export type OccurrenceKindFilter = "all" | "text" | "formatting";

export type OccurrenceFilter = {
  status: OccurrenceStatusFilter;
  kind: OccurrenceKindFilter;
  query: string;
};

export const emptyOccurrenceFilter: OccurrenceFilter = { status: "all", kind: "all", query: "" };

export function isOccurrenceFilterActive(filter: OccurrenceFilter): boolean {
  return filter.status !== "all" || filter.kind !== "all" || filter.query.trim().length > 0;
}

/** Filters the loaded occurrences; once the sheet has finished loading this covers the whole sheet. */
export function filterOccurrences(
  occurrences: readonly TranslationOccurrenceView[],
  filter: OccurrenceFilter,
): readonly TranslationOccurrenceView[] {
  if (!isOccurrenceFilterActive(filter)) return occurrences;
  const query = filter.query.trim().toLocaleLowerCase();
  return occurrences.filter((occurrence) => {
    if (filter.status === "untranslated" ? occurrence.reviewState !== null : filter.status !== "all" && occurrence.reviewState !== filter.status) {
      return false;
    }
    if (filter.kind !== "all" && occurrence.formattingOnly !== (filter.kind === "formatting")) return false;
    if (!query) return true;
    return occurrence.sourceMacro.toLocaleLowerCase().includes(query)
      || (occurrence.targetMacro?.toLocaleLowerCase().includes(query) ?? false)
      || `${occurrence.binding.rowId}:${occurrence.binding.subrowId}`.startsWith(query);
  });
}

/** Returns the occurrence `direction` steps from `current`, clamped to the list. */
export function adjacentOccurrence(
  occurrences: readonly TranslationOccurrenceView[],
  current: SourceBinding | null,
  direction: 1 | -1,
): TranslationOccurrenceView | null {
  if (occurrences.length === 0) return null;
  const index = occurrenceIndex(occurrences, current);
  if (index >= 0) return occurrences[index + direction] ?? null;
  if (current === null) return direction === 1 ? occurrences[0]! : occurrences.at(-1)!;
  // The current string left a filtered list, for example after approving
  // it: continue from its place in sheet order.
  const after = (occurrence: TranslationOccurrenceView) => compareBindings(occurrence.binding, current) > 0;
  return direction === 1
    ? occurrences.find(after) ?? null
    : occurrences.filter((occurrence) => !after(occurrence)).at(-1) ?? null;
}

/** Index of the occurrence for `binding`, compared by field so large lists build no keys. */
export function occurrenceIndex(occurrences: readonly TranslationOccurrenceView[], binding: SourceBinding | null): number {
  if (binding === null) return -1;
  return occurrences.findIndex(({ binding: candidate }) =>
    candidate.rowId === binding.rowId
    && candidate.subrowId === binding.subrowId
    && candidate.columnIndex === binding.columnIndex
    && candidate.sheetName === binding.sheetName);
}

function compareBindings(left: SourceBinding, right: SourceBinding): number {
  return left.sheetName.localeCompare(right.sheetName)
    || left.rowId - right.rowId
    || left.subrowId - right.subrowId
    || left.columnIndex - right.columnIndex;
}
