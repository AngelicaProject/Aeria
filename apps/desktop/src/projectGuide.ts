import type { GlossaryEntry, TermCandidateDto, TermInput } from "./types";

/**
 * One editable term row. `forbidden` is the `;`-separated text; `settled`
 * marks a term a person decided, which agents do not change without asking;
 * `matchCase` a term that matches only with its case as written.
 */
export type GlossaryRow = { key: number; term: string; translation: string; note: string; forbidden: string; settled: boolean; matchCase: boolean };

export type RowProblem = "emptyTerm" | "emptyTranslation" | "duplicateTerm";

export function rowsFromEntries(entries: readonly GlossaryEntry[]): GlossaryRow[] {
  return entries.map((entry, index) => ({
    key: index,
    term: entry.term,
    translation: entry.translation,
    note: entry.note ?? "",
    forbidden: (entry.forbidden ?? []).join("; "),
    settled: entry.settled ?? false,
    matchCase: entry.matchCase ?? false,
  }));
}

export function inputsFromRows(rows: readonly GlossaryRow[]): TermInput[] {
  return rows.map((row) => ({
    term: row.term.trim(),
    translation: row.translation.trim(),
    note: row.note.trim() || null,
    forbidden: row.forbidden.split(";").map((variant) => variant.trim()).filter(Boolean),
    settled: row.settled,
    matchCase: row.matchCase,
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
/** A term without case and a leading "the", as the glossary compares terms. */
function termKey(term: string): string {
  return term.trim().toLowerCase().replace(/^the\s+/, "");
}

/**
 * The row a candidate becomes: the chosen rendering as the translation and
 * the others as forbidden variants. A name matches with its case, so a
 * common word spelled alike (`eye` beside `the Eye`) is not the term.
 */
export function rowFromCandidate(candidate: TermCandidateDto, chosen: number, key: number): GlossaryRow {
  const renderings = candidate.renderings.map((rendering) => rendering.words.join(" "));
  return {
    key,
    term: candidate.phrase,
    translation: renderings[chosen] ?? "",
    note: "",
    forbidden: renderings.filter((_, index) => index !== chosen).join("; "),
    settled: true,
    matchCase: true,
  };
}

/** The candidates still to decide: not terms of `rows`, not skipped. */
export function openCandidates(candidates: readonly TermCandidateDto[], rows: readonly GlossaryRow[], skipped: ReadonlySet<string>): TermCandidateDto[] {
  const terms = new Set(rows.map((row) => termKey(row.term)));
  return candidates.filter((candidate) => !terms.has(termKey(candidate.phrase)) && !skipped.has(candidate.phrase));
}

export function filterRows(rows: readonly GlossaryRow[], query: string): GlossaryRow[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [...rows];
  return rows.filter((row) => [row.term, row.translation, row.note, row.forbidden].some((field) => field.toLowerCase().includes(needle)));
}

/** Whether edited rows differ from the saved entries. */
export function rowsChanged(rows: readonly GlossaryRow[], entries: readonly GlossaryEntry[]): boolean {
  return JSON.stringify(inputsFromRows(rows)) !== JSON.stringify(inputsFromRows(rowsFromEntries(entries)));
}
