import type { ChipPick } from "./components/macroChipsExtension";
import type { MacroInsertionDto } from "./types";

/** What choosing an insertion adds to a translation, for one of its rows when it has rows. */
export function insertionPick(insertion: MacroInsertionDto, row?: number): ChipPick {
  const parts = insertion.parts.map((part) => row === undefined ? part : part.replaceAll("{row}", String(row)));
  if (insertion.form === "wrap") return { wrap: [parts[0] ?? "", parts[1] ?? ""] };
  if (insertion.form === "branches") return { branches: [parts[0] ?? "", parts[1] ?? "", parts[2] ?? ""] };
  return { insert: parts[0] ?? "" };
}
