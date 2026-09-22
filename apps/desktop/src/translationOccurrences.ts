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
