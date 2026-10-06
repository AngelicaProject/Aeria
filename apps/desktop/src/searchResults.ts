import { exceptionTerm } from "./issueText.ts";
import type { SearchEntryDto, SearchFileDto, SearchHitDto } from "./types";

/** A string of a search result, by file and `msgctxt`. */
export function hitKey(hit: Pick<SearchEntryDto, "path" | "context">): string {
  return `${hit.path}|${hit.context}`;
}

/** The chosen strings, by `hitKey`; a hit of a result is one too. */
export type Chosen = ReadonlyMap<string, SearchEntryDto>;

/** `chosen` with `strings` chosen (`on`) or not. */
export function choose(chosen: Chosen, strings: readonly SearchEntryDto[], on: boolean): Chosen {
  const next = new Map(chosen);
  for (const string of strings) {
    if (on) next.set(hitKey(string), string);
    else next.delete(hitKey(string));
  }
  return next;
}

/** `chosen` without the strings of `paths`. */
export function unchooseFiles(chosen: Chosen, paths: ReadonlySet<string>): Chosen {
  return new Map([...chosen].filter(([, string]) => !paths.has(string.path)));
}

/** How many strings are chosen in each file. */
export function chosenByPath(chosen: Chosen): ReadonlyMap<string, number> {
  const counts = new Map<string, number>();
  for (const string of chosen.values()) counts.set(string.path, (counts.get(string.path) ?? 0) + 1);
  return counts;
}

/** The files of one sheet, which follow each other in a result. */
export type SheetGroup = { key: string; sheet: string; files: SearchFileDto[]; count: number };

/** Groups the files of a result by sheet: `Item/10000.po` and `Item/12000.po` are one `Item`. */
export function groupBySheet(files: readonly SearchFileDto[]): SheetGroup[] {
  const groups: SheetGroup[] = [];
  for (const file of files) {
    const sheet = file.sheet || file.path;
    const last = groups[groups.length - 1];
    if (last && last.sheet === sheet) {
      last.files.push(file);
      last.count += file.count;
    } else {
      groups.push({ key: file.path, sheet, files: [file], count: file.count });
    }
  }
  return groups;
}

/** The term every hit has a term finding for, or null. */
export function commonTerm(hits: readonly SearchEntryDto[]): string | null {
  const [first, ...rest] = hits;
  if (!first) return null;
  const termsOf = (hit: SearchEntryDto) => new Set(hit.findings.map(exceptionTerm).filter((term): term is string => term !== null));
  return [...termsOf(first)].find((term) => rest.every((hit) => termsOf(hit).has(term))) ?? null;
}

/**
 * The chosen strings after the result was read again: a chosen string still
 * found takes its new state, one gone from a file whose strings are all
 * known (`complete`) is no longer chosen, and the others stay. Returns
 * `chosen` itself when nothing changed.
 */
export function reconcileChosen(
  chosen: Chosen,
  shown: readonly SearchEntryDto[],
  complete: (path: string) => boolean,
): Chosen {
  if (chosen.size === 0) return chosen;
  const byKey = new Map(shown.map((hit) => [hitKey(hit), hit]));
  let changed = false;
  const next = new Map<string, SearchEntryDto>();
  for (const [key, hit] of chosen) {
    const now = byKey.get(key);
    if (now) {
      next.set(key, now);
      if (now.translation !== hit.translation || now.fuzzy !== hit.fuzzy) changed = true;
    } else if (complete(hit.path)) {
      changed = true;
    } else {
      next.set(key, hit);
    }
  }
  return changed ? next : chosen;
}

/** What is at hand of a sheet's strings. */
export type GroupHits = {
  hits: readonly SearchHitDto[];
  /** Every string of the sheet the search found is in `hits`. */
  complete: boolean;
  /** Its strings are being read. */
  loading: boolean;
  /** More strings than one search returns: `hits` are the first of them. */
  cut: boolean;
};

/** A line of the result list. */
export type ResultRow =
  | { kind: "sheet"; key: string; group: SheetGroup; open: boolean }
  | { kind: "hit"; key: string; group: SheetGroup; hit: SearchHitDto }
  /** The sheet has strings not at hand, which can be read. */
  | { kind: "more"; key: string; group: SheetGroup; shown: number }
  | { kind: "loading"; key: string; group: SheetGroup }
  /** The sheet has more strings than one search returns. */
  | { kind: "cut"; key: string; group: SheetGroup; shown: number };

/**
 * The lines of the result list: each sheet, and under an open one its
 * strings at hand and what is known of the rest.
 */
export function resultRows(
  groups: readonly SheetGroup[],
  isOpen: (group: SheetGroup) => boolean,
  hitsOf: (group: SheetGroup) => GroupHits,
): ResultRow[] {
  const rows: ResultRow[] = [];
  for (const group of groups) {
    const open = isOpen(group);
    rows.push({ kind: "sheet", key: `sheet|${group.key}`, group, open });
    if (!open) continue;
    const { hits, complete, loading, cut } = hitsOf(group);
    for (const hit of hits) rows.push({ kind: "hit", key: hitKey(hit), group, hit });
    if (loading) rows.push({ kind: "loading", key: `loading|${group.key}`, group });
    else if (cut) rows.push({ kind: "cut", key: `cut|${group.key}`, group, shown: hits.length });
    else if (!complete) rows.push({ kind: "more", key: `more|${group.key}`, group, shown: hits.length });
  }
  return rows;
}

/** The row to move to from `index` by `step` (1 down, -1 up), stopping at the ends; rows of messages are skipped. */
export function nextRow(rows: readonly ResultRow[], index: number, step: 1 | -1): number {
  for (let at = index + step; at >= 0 && at < rows.length; at += step) {
    const kind = rows[at]?.kind;
    if (kind === "sheet" || kind === "hit" || kind === "more") return at;
  }
  return index;
}

/**
 * Whether a sheet of the result is open: by default when the search sent
 * strings of it (none after "Collapse all"), unless the person toggled it.
 * Strings read later do not change the default, or a sheet opened to read
 * them would close again once they arrive.
 */
export function sheetOpen(group: SheetGroup, sent: (path: string) => number, collapsed: boolean, toggled: ReadonlySet<string>): boolean {
  const byDefault = !collapsed && group.files.some((file) => sent(file.path) > 0);
  return byDefault !== toggled.has(group.key);
}
