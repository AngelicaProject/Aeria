import type { GlossaryEntry, GlossaryEntryInput } from "./types";

/** One editable glossary row. `forbidden` is the `;`-separated text. */
export type GlossaryRow = { key: number; term: string; translation: string; note: string; forbidden: string };

export type RowProblem = "emptyTerm" | "emptyTranslation" | "duplicateTerm";

export function rowsFromEntries(entries: readonly GlossaryEntry[]): GlossaryRow[] {
  return entries.map((entry, index) => ({
    key: index,
    term: entry.term,
    translation: entry.translation,
    note: entry.note ?? "",
    forbidden: (entry.forbidden ?? []).join("; "),
  }));
}

export function inputsFromRows(rows: readonly GlossaryRow[]): GlossaryEntryInput[] {
  return rows.map((row) => ({
    term: row.term.trim(),
    translation: row.translation.trim(),
    note: row.note.trim() || null,
    forbidden: row.forbidden.split(";").map((variant) => variant.trim()).filter(Boolean),
  }));
}

/** Problems per row key, matching the rules of Glossary Format v1. */
export function rowProblems(rows: readonly GlossaryRow[]): Map<number, RowProblem> {
  const problems = new Map<number, RowProblem>();
  const seen = new Set<string>();
  for (const row of rows) {
    const term = row.term.trim();
    if (!term) problems.set(row.key, "emptyTerm");
    else if (!row.translation.trim()) problems.set(row.key, "emptyTranslation");
    else if (seen.has(term.toLowerCase())) problems.set(row.key, "duplicateTerm");
    if (term) seen.add(term.toLowerCase());
  }
  return problems;
}

/** Rows whose term, translation, or note contains the query, ignoring case. */
export function filterRows(rows: readonly GlossaryRow[], query: string): GlossaryRow[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [...rows];
  return rows.filter((row) => [row.term, row.translation, row.note, row.forbidden].some((field) => field.toLowerCase().includes(needle)));
}

/** Whether edited rows differ from the saved entries. */
export function rowsChanged(rows: readonly GlossaryRow[], entries: readonly GlossaryEntry[]): boolean {
  return JSON.stringify(inputsFromRows(rows)) !== JSON.stringify(inputsFromRows(rowsFromEntries(entries)));
}
