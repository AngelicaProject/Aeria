import type { SourceBinding, TranslationRowCursorDto } from "./types";

export function bindingKey(binding: SourceBinding): string {
  return `${binding.sheetName}\u0000${binding.rowId}\u0000${binding.subrowId}\u0000${binding.columnIndex}`;
}

export function rowKey(row: Pick<TranslationRowCursorDto, "sheetName" | "rowId" | "subrowId">): string {
  return `${row.sheetName}\u0000${row.rowId}\u0000${row.subrowId}`;
}

export function domKey(value: string): string {
  return value.replaceAll("\u0000", "-");
}
