import { categoryKey, entryItems, facetCounts, inputsFromRows, itemCategory, originsForRows, outdatedOf, rowProblems, type GlossaryItem, type GlossaryRow, type Outdated, type RowProblem } from "./glossaryModel.ts";
import type { ReplacedTermDto, SheetTermDto } from "./types";

type IndexedRow = { row: GlossaryRow; at: number; names: string[] };

function sameSources(a: readonly SheetTermDto[], b: readonly SheetTermDto[]): boolean {
  return a.length === b.length && a.every((old, at) => {
    const term = b[at]!;
    return old.sheet === term.sheet && old.context === term.context && old.term === term.term
      && old.translation === term.translation && old.category === term.category && old.strings === term.strings
      && old.forms.length === term.forms.length && old.forms.every((form, i) => form === term.forms[i])
      && old.variants.length === term.variants.length && old.variants.every((variant, i) => variant.translation === term.variants[i]!.translation && variant.strings === term.variants[i]!.strings);
  });
}

/** Incremental validation of immutable draft rows, with the same rules as rowProblems. */
function problemIndex() {
  const indexed = new Map<number, IndexedRow>();
  const owners = new Map<string, Set<number>>();
  let previous: readonly GlossaryRow[] = [];
  let problems = new Map<number, RowProblem>();
  return (rows: readonly GlossaryRow[]) => {
    const reset = rows.length !== previous.length || rows.some((row, at) => row.key !== previous[at]?.key);
    if (reset) { indexed.clear(); owners.clear(); problems = new Map(); }
    const affected = new Set<number>();
    const affect = (name: string) => { for (const key of owners.get(name) ?? []) affected.add(key); };
    rows.forEach((row, at) => {
      const old = indexed.get(row.key);
      if (old?.row === row) return;
      if (old && old.row.term === row.term && old.row.category === row.category && old.row.forms === row.forms
        && Boolean(old.row.translation.trim()) === Boolean(row.translation.trim())) {
        old.row = row;
        return;
      }
      affected.add(row.key);
      for (const name of old?.names ?? []) {
        affect(name);
        const taken = owners.get(name)!;
        taken.delete(row.key);
        if (taken.size === 0) owners.delete(name);
      }
      const names = [...new Set([row.term, ...row.forms].map((form) => form.trim().toLowerCase()).filter(Boolean))]
        .map((name) => JSON.stringify([categoryKey(row.category), name]));
      indexed.set(row.key, { row, at, names });
      for (const name of names) {
        const taken = owners.get(name) ?? new Set<number>();
        taken.add(row.key);
        owners.set(name, taken);
        affect(name);
      }
    });
    let next = problems;
    for (const key of affected) {
      const current = indexed.get(key)!;
      let problem = rowProblems([current.row]).get(key);
      if (!problem && current.names.some((name) => [...owners.get(name)!].some((other) => indexed.get(other)!.at < current.at))) problem = "duplicateTerm";
      if (problems.get(key) === problem) continue;
      if (next === problems) next = new Map(problems);
      if (problem) next.set(key, problem); else next.delete(key);
    }
    previous = rows;
    problems = next;
    return problems;
  };
}

/** One cache per open glossary. Rows are replaced, never mutated, by the editor. */
export function createGlossaryDraft() {
  const validate = problemIndex();
  const serialized = new WeakMap<GlossaryRow, string>();
  const text = (row: GlossaryRow) => {
    let value = serialized.get(row);
    if (value === undefined) { value = JSON.stringify(inputsFromRows([row])[0]); serialized.set(row, value); }
    return value;
  };
  let originRows: readonly GlossaryRow[] = [];
  let originIds: readonly string[] | undefined;
  let originData: readonly ReplacedTermDto[] | undefined;
  let originMerges: readonly { from: string; into: string }[] | undefined;
  let origins: ReplacedTermDto[] = [];
  let itemOrigins: readonly ReplacedTermDto[] | undefined;
  const byRow = new Map<number, Extract<GlossaryItem, { kind: "entry" }>>();
  let references = new Map<number, readonly SheetTermDto[]>();
  const noSources: readonly SheetTermDto[] = [];
  let countedItems: readonly GlossaryItem[] = [];
  let countedOutdated: Outdated | null = null;
  let counts: ReturnType<typeof facetCounts> | undefined;

  return {
    changed: (rows: readonly GlossaryRow[], saved: readonly GlossaryRow[]) => rows.length !== saved.length
      || rows.some((row, at) => row !== saved[at] && text(row) !== text(saved[at]!)),
    problems: validate,
    origins(rows: readonly GlossaryRow[], ids: readonly string[], data: readonly ReplacedTermDto[], merges: readonly { from: string; into: string }[]) {
      if (ids !== originIds || data !== originData || merges !== originMerges || rows.length !== originRows.length
        || rows.some((row, at) => row.key !== originRows[at]?.key || row.id !== originRows[at]?.id)) {
        origins = originsForRows(rows, ids, data, merges);
        originRows = rows; originIds = ids; originData = data; originMerges = merges;
      }
      return origins;
    },
    items(rows: readonly GlossaryRow[], problems: ReadonlyMap<number, RowProblem>, origins: readonly ReplacedTermDto[]) {
      if (origins !== itemOrigins) {
        itemOrigins = origins;
        const updated = new Map<number, SheetTermDto[]>();
        for (const { entry, term } of origins) {
          const key = rows[entry]!.key;
          updated.set(key, [...(updated.get(key) ?? []), term]);
        }
        references = new Map([...updated].map(([key, sources]) => {
          const old = byRow.get(key)?.replaced;
          return [key, old && sameSources(old, sources) ? old : sources];
        }));
        const keys = new Set(rows.map((row) => row.key));
        for (const key of byRow.keys()) if (!keys.has(key)) byRow.delete(key);
      }
      const items = rows.map((row) => {
        let item = byRow.get(row.key);
        const sources = references.get(row.key) ?? noSources;
        if (!item || item.row !== row || item.problem !== (problems.get(row.key) ?? null) || item.replaced !== sources) {
          const next = entryItems([row], problems, sources.map((term) => ({ entry: 0, term })), [], [row.key])[0] as Extract<GlossaryItem, { kind: "entry" }>;
          item = { ...next, replaced: sources };
          byRow.set(row.key, item);
        }
        return item;
      });
      return items;
    },
    counts(items: readonly GlossaryItem[], outdated: Outdated | null) {
      if (!counts || countedOutdated !== outdated || countedItems.length !== items.length || items.some((item, at) => {
        const old = countedItems[at]!;
        return item !== old && (item.kind !== "entry" || old.kind !== "entry" || itemCategory(item) !== itemCategory(old)
          || item.problem !== old.problem || item.replaced !== old.replaced || outdatedOf(item, outdated) !== outdatedOf(old, outdated));
      })) counts = facetCounts(items, outdated);
      countedItems = items; countedOutdated = outdated;
      return counts;
    },
  };
}
