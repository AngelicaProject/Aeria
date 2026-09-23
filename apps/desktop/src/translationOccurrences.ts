import { bindingKey, rowKey } from "./binding.ts";
import type { ReviewState, SourceBinding, TranslationRowCursorDto, TranslationRowDto } from "./types";

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
  targetMacro: string | null;
  reviewState: ReviewState | null;
  fieldIndexInRow: number;
  fieldCountInRow: number;
  firstInRow: boolean;
  lastInRow: boolean;
};

export function flattenTranslationRows(rows: readonly TranslationRowDto[]): TranslationOccurrenceView[] {
  return rows.flatMap((row) => {
    const fieldCountInRow = row.cells.length;
    return row.cells.map((cell, fieldIndexInRow) => ({
      rowKey: rowKey(row),
      binding: cell.sourceBinding,
      sourceMacro: cell.sourceMacro,
      targetMacro: cell.translation?.targetMacro ?? null,
      reviewState: cell.translation?.reviewState ?? null,
      fieldIndexInRow,
      fieldCountInRow,
      firstInRow: fieldIndexInRow === 0,
      lastInRow: fieldIndexInRow === fieldCountInRow - 1,
    }));
  });
}

export function occurrenceKey(occurrence: Pick<TranslationOccurrenceView, "binding">): string {
  return bindingKey(occurrence.binding);
}

export type OccurrenceStatusFilter = "all" | "untranslated" | "draft" | "needsReview" | "reviewed";

export type OccurrenceFilter = {
  status: OccurrenceStatusFilter;
  query: string;
};

export const emptyOccurrenceFilter: OccurrenceFilter = { status: "all", query: "" };

/** Filters already-loaded occurrences only; it never implies sheet-wide results. */
export function filterOccurrences(
  occurrences: readonly TranslationOccurrenceView[],
  filter: OccurrenceFilter,
): TranslationOccurrenceView[] {
  const query = filter.query.trim().toLocaleLowerCase();
  return occurrences.filter((occurrence) => {
    if (filter.status === "untranslated" ? occurrence.reviewState !== null : filter.status !== "all" && occurrence.reviewState !== filter.status) {
      return false;
    }
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
  const currentKey = current ? bindingKey(current) : null;
  const index = currentKey === null ? -1 : occurrences.findIndex((occurrence) => bindingKey(occurrence.binding) === currentKey);
  if (index < 0) return direction === 1 ? occurrences[0]! : occurrences.at(-1)!;
  return occurrences[index + direction] ?? null;
}

/**
 * Exclusive paging cursor that makes a page start at `rowId:subrowId`.
 * Cursors order by row then subrow, so the greatest subrow of the previous row
 * precedes every subrow of `rowId`. Returns null for the first possible row.
 */
export function cursorBefore(sheetName: string, rowId: number, subrowId: number): TranslationRowCursorDto | null {
  if (subrowId > 0) return { sheetName, rowId, subrowId: subrowId - 1 };
  if (rowId > 0) return { sheetName, rowId: rowId - 1, subrowId: 0xffff };
  return null;
}
