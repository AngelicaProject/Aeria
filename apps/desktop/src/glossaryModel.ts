import type { GlossaryEntry, ReplacedTermDto, SheetTermDto, TermInput, TermSheetDto } from "./types";

/**
 * A term of `terms.csv` being edited. `term` is the headword, `forms` its
 * other forms in the source, and `matchCase` marks a term that matches only
 * with its case as written. `key` identifies the row while the glossary is
 * open; it is not its place in the file.
 */
export type GlossaryRow = { key: number; term: string; translation: string; forms: string[]; note: string; matchCase: boolean; category: string };

export type RowProblem = "emptyTerm" | "emptyTranslation" | "emptyCategory" | "duplicateTerm";

/** The key a term's use is counted by: its name and category, ignoring case. */
export function usageKey(term: string, category: string): string {
  return `${term.trim().toLowerCase()}\u0000${category.trim().toLowerCase()}`;
}

/** The key categories are grouped by: ignoring case and surrounding space. */
export function categoryKey(category: string): string {
  return category.trim().toLocaleLowerCase();
}

/** The declared sheets of a category, by its key, in their order. */
export function categorySheets(sheets: readonly TermSheetDto[], key: string): TermSheetDto[] {
  return sheets.filter((sheet) => categoryKey(sheet.category) === key);
}

/**
 * The note of a category: the notes of its declared sheets, each once, in
 * their order, each on its own line, as the terms of the project take it
 * before their own. Empty when none of them has one.
 */
export function categoryNote(sheets: readonly TermSheetDto[], category: string): string {
  const notes: string[] = [];
  for (const sheet of categorySheets(sheets, categoryKey(category))) {
    const note = (sheet.note ?? "").trim();
    if (note && !notes.includes(note)) notes.push(note);
  }
  return notes.join("\n");
}

/** The forms typed into a forms field: `;` separates several. */
export function splitForms(text: string): string[] {
  return text.split(";").map((form) => form.trim()).filter(Boolean);
}

/**
 * The entries as rows. A row of `previous` with an entry's name and category
 * gives it its key, so a term read again after a save or a change of the
 * file stays the one chosen; other entries take keys after every key of
 * `previous`.
 */
export function rowsFromEntries(entries: readonly GlossaryEntry[], previous: readonly GlossaryRow[] = []): GlossaryRow[] {
  const known = new Map<string, number[]>();
  for (const row of previous) {
    const name = usageKey(row.term, row.category);
    known.set(name, [...(known.get(name) ?? []), row.key]);
  }
  let next = previous.reduce((high, row) => Math.max(high, row.key + 1), 0);
  return entries.map((entry) => ({
    key: known.get(usageKey(entry.term, entry.category))?.shift() ?? next++,
    term: entry.term,
    translation: entry.translation,
    forms: [...(entry.forms ?? [])],
    note: entry.note ?? "",
    matchCase: entry.matchCase ?? false,
    category: entry.category,
  }));
}

export function inputsFromRows(rows: readonly GlossaryRow[]): TermInput[] {
  return rows.map((row) => ({
    term: row.term.trim(),
    translation: row.translation.trim(),
    forms: row.forms.map((form) => form.trim()).filter(Boolean),
    note: row.note.trim() || null,
    matchCase: row.matchCase,
    category: row.category.trim(),
  }));
}

/** Whether edited rows differ from the saved entries. */
export function rowsChanged(rows: readonly GlossaryRow[], entries: readonly GlossaryEntry[]): boolean {
  return JSON.stringify(inputsFromRows(rows)) !== JSON.stringify(inputsFromRows(rowsFromEntries(entries)));
}

/**
 * Problems per row key, matching the rules of Glossary Format v1: every
 * term has a category, and every form, headwords included, appears once in
 * the glossary, ignoring case, but for the meanings of one name, which share
 * it while their categories differ.
 */
export function rowProblems(rows: readonly GlossaryRow[]): Map<number, RowProblem> {
  const problems = new Map<number, RowProblem>();
  // Each lowercase form with the name and category of the rows that have it.
  const seen = new Map<string, { name: string; category: string }[]>();
  for (const row of rows) {
    const term = row.term.trim();
    const name = term.toLowerCase();
    const category = row.category.trim().toLowerCase();
    const forms = [term, ...row.forms.map((form) => form.trim())].filter(Boolean).map((form) => form.toLowerCase());
    const taken = (form: string) => (seen.get(form) ?? []).some((other) => other.name !== name || other.category === category);
    if (!term) problems.set(row.key, "emptyTerm");
    else if (!row.translation.trim()) problems.set(row.key, "emptyTranslation");
    else if (!category) problems.set(row.key, "emptyCategory");
    else if (forms.some((form, index) => taken(form) || forms.indexOf(form) !== index)) problems.set(row.key, "duplicateTerm");
    for (const form of new Set(forms)) seen.set(form, [...(seen.get(form) ?? []), { name, category }]);
  }
  return problems;
}

/**
 * One term of the glossary list: an entry of `terms.csv`, with the terms of
 * the sheets it takes the place of, or a term a declared sheet gives.
 * `search` is the lowercase text a query is looked for in.
 */
export type GlossaryItem =
  | { kind: "entry"; id: string; row: GlossaryRow; replaced: readonly SheetTermDto[]; problem: RowProblem | null; redundant: boolean; search: string }
  | { kind: "sheet"; id: string; term: SheetTermDto; search: string };

export function entryId(key: number): string {
  return `entry:${key}`;
}

export function sheetTermId(context: string): string {
  return `sheet:${context}`;
}

export function itemTerm(item: GlossaryItem): string {
  return item.kind === "entry" ? item.row.term.trim() : item.term.term;
}

export function itemTranslation(item: GlossaryItem): string {
  return item.kind === "entry" ? item.row.translation.trim() : item.term.translation;
}

export function itemCategory(item: GlossaryItem): string {
  return item.kind === "entry" ? item.row.category.trim() : item.term.category;
}

/** The sheet a term comes from; for an entry, the first sheet it takes the place of, if any. */
export function itemSheet(item: GlossaryItem): string | null {
  return item.kind === "sheet" ? item.term.sheet : item.replaced[0]?.sheet ?? null;
}

/** How many translations do not use each meaning of a term, by [`usageKey`]. */
export type Unapplied = ReadonlyMap<string, number>;

/** How many translations do not use the item's term in its meaning. */
export function unappliedOf(item: GlossaryItem, unapplied: Unapplied | null): number {
  return unapplied?.get(usageKey(itemTerm(item), itemCategory(item))) ?? 0;
}

/**
 * The entries of `terms.csv` as list items, with the terms of the sheets each
 * takes the place of. `replaced` names entries by their place in the file,
 * and `savedKeys` holds the key of the row read from each place; without it
 * the keys are the places.
 */
export function entryItems(
  rows: readonly GlossaryRow[],
  problems: ReadonlyMap<number, RowProblem>,
  replaced: readonly ReplacedTermDto[],
  redundant: readonly string[],
  savedKeys: readonly number[] | null = null,
): GlossaryItem[] {
  const replacedBy = new Map<number, SheetTermDto[]>();
  for (const { entry, term } of replaced) {
    const key = savedKeys === null ? entry : savedKeys[entry];
    if (key !== undefined) replacedBy.set(key, [...(replacedBy.get(key) ?? []), term]);
  }
  const same = new Set(redundant);
  return rows.map((row): GlossaryItem => {
    const replacedTerms = replacedBy.get(row.key) ?? [];
    return {
      kind: "entry",
      id: entryId(row.key),
      row,
      replaced: replacedTerms,
      problem: problems.get(row.key) ?? null,
      redundant: same.has(row.term),
      search: [row.term, row.translation, row.note, row.category, ...row.forms, ...replacedTerms.map((term) => term.translation)].join("\n").toLocaleLowerCase(),
    };
  });
}

/** The terms of the declared sheets as list items, in the order the sheets give them. */
export function sheetItems(sheetTerms: readonly SheetTermDto[]): GlossaryItem[] {
  return sheetTerms.map((term): GlossaryItem => ({
    kind: "sheet",
    id: sheetTermId(term.context),
    term,
    search: [term.term, term.translation, term.category, term.sheet, ...term.forms, ...term.variants.map((variant) => variant.translation)].join("\n").toLocaleLowerCase(),
  }));
}

/** What the list shows besides a category and a sheet. */
export type GlossaryView = "all" | "entries" | "sheets" | "problems" | "unapplied" | "redundant";

export const GLOSSARY_VIEWS: readonly GlossaryView[] = ["all", "entries", "sheets", "problems", "unapplied", "redundant"];

export function inView(item: GlossaryItem, view: GlossaryView, unapplied: Unapplied | null): boolean {
  switch (view) {
    case "all": return true;
    case "entries": return item.kind === "entry";
    case "sheets": return item.kind === "sheet";
    case "problems": return item.kind === "entry" && item.problem !== null;
    case "unapplied": return unappliedOf(item, unapplied) > 0;
    case "redundant": return item.kind === "entry" && item.redundant;
  }
}

/** The list's filter: a view, a category key, a declared sheet, and a query. */
export type GlossaryFilter = { view: GlossaryView; category: string | null; sheet: string | null; query: string };

export const emptyGlossaryFilter: GlossaryFilter = { view: "all", category: null, sheet: null, query: "" };

/**
 * The items the filter shows. The item `keep` is shown whatever the filter,
 * so a term being edited does not leave the list.
 */
export function filterItems(items: readonly GlossaryItem[], filter: GlossaryFilter, unapplied: Unapplied | null = null, keep: string | null = null): GlossaryItem[] {
  const needle = filter.query.trim().toLocaleLowerCase();
  return items.filter((item) => item.id === keep || (
    inView(item, filter.view, unapplied)
    && (filter.category === null || categoryKey(itemCategory(item)) === filter.category)
    && (filter.sheet === null || (item.kind === "sheet" ? item.term.sheet === filter.sheet : item.replaced.some((term) => term.sheet === filter.sheet)))
    && (needle === "" || item.search.includes(needle))));
}

export type CategoryCount = { key: string; label: string; count: number };

/** How many terms each view, category, and sheet holds, over every term. */
export function facetCounts(items: readonly GlossaryItem[], unapplied: Unapplied | null = null): { views: Record<GlossaryView, number>; categories: CategoryCount[]; sheets: ReadonlyMap<string, number> } {
  const views = Object.fromEntries(GLOSSARY_VIEWS.map((view) => [view, 0])) as Record<GlossaryView, number>;
  const categories = new Map<string, CategoryCount>();
  const sheets = new Map<string, number>();
  for (const item of items) {
    for (const view of GLOSSARY_VIEWS) if (inView(item, view, unapplied)) views[view] += 1;
    const label = itemCategory(item);
    const key = categoryKey(label);
    if (key) {
      const known = categories.get(key);
      if (known) known.count += 1;
      else categories.set(key, { key, label, count: 1 });
    }
    if (item.kind === "sheet") sheets.set(item.term.sheet, (sheets.get(item.term.sheet) ?? 0) + 1);
  }
  return { views, categories: [...categories.values()].sort((a, b) => b.count - a.count || a.label.localeCompare(b.label)), sheets };
}

/** The column the list is ordered by; `order` keeps the file and the sheets' order. */
export type SortKey = "order" | "term" | "translation" | "category" | "source" | "uses";

export type GlossarySort = { key: SortKey; descending: boolean };

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/** The items ordered by a column; equals keep their order. */
export function sortItems(items: readonly GlossaryItem[], sort: GlossarySort, usage: ReadonlyMap<string, number> | null): GlossaryItem[] {
  if (sort.key === "order") return sort.descending ? [...items].reverse() : [...items];
  const uses = (item: GlossaryItem) => usage?.get(usageKey(itemTerm(item), itemCategory(item))) ?? 0;
  const text = (item: GlossaryItem): string => {
    switch (sort.key) {
      case "term": return itemTerm(item);
      case "translation": return itemTranslation(item);
      case "category": return itemCategory(item);
      default: return itemSheet(item) ?? "";
    }
  };
  const sign = sort.descending ? -1 : 1;
  return items
    .map((item, at) => ({ item, at }))
    .sort((a, b) => {
      const order = sort.key === "uses" ? uses(a.item) - uses(b.item) : collator.compare(text(a.item), text(b.item));
      return order * sign || a.at - b.at;
    })
    .map(({ item }) => item);
}
