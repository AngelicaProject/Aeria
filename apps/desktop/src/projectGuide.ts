import type { GlossaryEntry, TermCandidateDto, TermInput } from "./types";

/**
 * One editable term row. `term` is the headword and `forms` the term's other
 * forms in the source; `folder` is its folder path, `/`-separated and empty
 * at the top; `matchCase` marks a term that matches only with its case as
 * written.
 */
export type GlossaryRow = { key: number; term: string; translation: string; forms: string[]; note: string; folder: string; matchCase: boolean };

export type RowProblem = "emptyTerm" | "emptyTranslation" | "duplicateTerm";

/** A folder path in its canonical form: trimmed names, no empty ones. */
export function folderPath(folder: string): string {
  return folder.split("/").map((name) => name.trim()).filter(Boolean).join("/");
}

/** The forms typed into a forms field: `;` separates several. */
export function splitForms(text: string): string[] {
  return text.split(";").map((form) => form.trim()).filter(Boolean);
}

export function rowsFromEntries(entries: readonly GlossaryEntry[]): GlossaryRow[] {
  return entries.map((entry, index) => ({
    key: index,
    term: entry.term,
    translation: entry.translation,
    forms: [...(entry.forms ?? [])],
    note: entry.note ?? "",
    folder: entry.folder ?? "",
    matchCase: entry.matchCase ?? false,
  }));
}

export function inputsFromRows(rows: readonly GlossaryRow[]): TermInput[] {
  return rows.map((row) => ({
    term: row.term.trim(),
    translation: row.translation.trim(),
    forms: row.forms.map((form) => form.trim()).filter(Boolean),
    note: row.note.trim() || null,
    folder: folderPath(row.folder),
    matchCase: row.matchCase,
  }));
}

/**
 * Problems per row key, matching the rules of Glossary Format v1: every
 * form, headwords included, appears once in the glossary, ignoring case.
 */
export function rowProblems(rows: readonly GlossaryRow[]): Map<number, RowProblem> {
  const problems = new Map<number, RowProblem>();
  const seen = new Set<string>();
  for (const row of rows) {
    const term = row.term.trim();
    const forms = [term, ...row.forms.map((form) => form.trim())].filter(Boolean).map((form) => form.toLowerCase());
    if (!term) problems.set(row.key, "emptyTerm");
    else if (!row.translation.trim()) problems.set(row.key, "emptyTranslation");
    else if (forms.some((form, index) => seen.has(form) || forms.indexOf(form) !== index)) problems.set(row.key, "duplicateTerm");
    for (const form of forms) seen.add(form);
  }
  return problems;
}

/** A term without case and a leading "the", as the glossary compares terms. */
function termKey(term: string): string {
  return term.trim().toLowerCase().replace(/^the\s+/, "");
}

/**
 * The row a candidate becomes: the chosen rendering as the translation, in
 * `folder`. A name matches with its case, so a common word spelled alike
 * (`eye` beside `the Eye`) is not the term.
 */
export function rowFromCandidate(candidate: TermCandidateDto, chosen: number, key: number, folder = ""): GlossaryRow {
  return {
    key,
    term: candidate.phrase,
    translation: candidate.renderings[chosen]?.words.join(" ") ?? "",
    forms: [],
    note: "",
    folder,
    matchCase: true,
  };
}

/** The candidates still to decide: not terms of `rows`, not skipped. */
export function openCandidates(candidates: readonly TermCandidateDto[], rows: readonly GlossaryRow[], skipped: ReadonlySet<string>): TermCandidateDto[] {
  const terms = new Set(rows.flatMap((row) => [row.term, ...row.forms]).map(termKey));
  return candidates.filter((candidate) => !terms.has(termKey(candidate.phrase)) && !skipped.has(candidate.phrase));
}

/** Whether `folder` is `parent` or lies inside it. */
export function inFolder(folder: string, parent: string): boolean {
  return parent === "" || folder === parent || folder.startsWith(`${parent}/`);
}

/**
 * Rows in `folder` (with its subfolders; `null` for every row) whose term,
 * forms, translation, or note contains the query, ignoring case.
 */
export function filterRows(rows: readonly GlossaryRow[], query: string, folder: string | null = null): GlossaryRow[] {
  const needle = query.trim().toLowerCase();
  return rows.filter((row) =>
    (folder === null || inFolder(folderPath(row.folder), folder))
    && (!needle || [row.term, row.translation, row.note, ...row.forms].some((field) => field.toLowerCase().includes(needle))));
}

/** One folder of the tree: its path, its own name, and its depth from the top. */
export type FolderNode = { path: string; name: string; depth: number; count: number };

/**
 * The folders of `rows` and of `extra` (folders made but still empty), with
 * every parent, in tree order; `count` is the terms in the folder and its
 * subfolders.
 */
export function folderTree(rows: readonly GlossaryRow[], extra: readonly string[] = []): FolderNode[] {
  const paths = new Set<string>();
  for (const folder of [...rows.map((row) => row.folder), ...extra]) {
    const names = folderPath(folder).split("/").filter(Boolean);
    names.forEach((_, index) => paths.add(names.slice(0, index + 1).join("/")));
  }
  const folders = rows.map((row) => folderPath(row.folder));
  return [...paths]
    .sort((left, right) => {
      const a = left.split("/");
      const b = right.split("/");
      for (let index = 0; index < Math.min(a.length, b.length); index += 1) {
        const order = a[index]!.localeCompare(b[index]!);
        if (order !== 0) return order;
      }
      return a.length - b.length;
    })
    .map((path) => {
      const names = path.split("/");
      return { path, name: names[names.length - 1]!, depth: names.length - 1, count: folders.filter((folder) => inFolder(folder, path)).length };
    });
}

/** `path` moved from under `from` to under `to`. */
function moved(path: string, from: string, to: string): string {
  const folder = folderPath(path);
  if (!inFolder(folder, from)) return path;
  return folderPath(`${to}/${folder.slice(from.length)}`);
}

/** Rows with the folder `from` and its subfolders moved to `to`. */
export function moveFolder(rows: readonly GlossaryRow[], from: string, to: string): GlossaryRow[] {
  return rows.map((row) => {
    const folder = moved(row.folder, from, to);
    return folder === row.folder ? row : { ...row, folder };
  });
}

/** Folder paths with `from` and its subfolders moved to `to`. */
export function moveFolders(folders: readonly string[], from: string, to: string): string[] {
  return [...new Set(folders.map((folder) => moved(folder, from, to)).filter(Boolean))];
}

/** The parent of a folder path; the top is `""`. */
export function parentFolder(path: string): string {
  return path.split("/").slice(0, -1).join("/");
}

/** Whether edited rows differ from the saved entries. */
export function rowsChanged(rows: readonly GlossaryRow[], entries: readonly GlossaryEntry[]): boolean {
  return JSON.stringify(inputsFromRows(rows)) !== JSON.stringify(inputsFromRows(rowsFromEntries(entries)));
}
