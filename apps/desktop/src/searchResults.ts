import { exceptionTerm } from "./issueText.ts";
import type { SearchEntryDto, SearchFileDto } from "./types";

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
