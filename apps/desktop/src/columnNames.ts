import type { SheetColumnNamesDto } from "./types";
import type { Translate } from "./i18n/translate";

/** A column's label, with its EXDSchema name when one is known, and a hint naming the schema version. */
export function columnLabel(
  t: Translate,
  columnIndex: number,
  names: SheetColumnNamesDto | null | undefined,
): { label: string; hint: string | undefined } {
  const column = String(columnIndex);
  const name = names?.names[columnIndex];
  if (!names || !name) return { label: t("common.column", { column }), hint: undefined };
  return {
    label: t("common.columnNamed", { column, name }),
    hint: t("common.columnSchema", { version: names.schemaVersion }),
  };
}
