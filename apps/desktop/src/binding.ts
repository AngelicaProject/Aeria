import type { SourceBinding } from "./types";

export function bindingKey(binding: SourceBinding): string {
  return `${binding.sheetName}\u0000${binding.rowId}\u0000${binding.subrowId}\u0000${binding.columnIndex}`;
}
