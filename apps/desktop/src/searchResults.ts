import { exceptionTerm } from "./issueText.ts";
import type { SearchFileDto, SearchHitDto } from "./types";

/** A string of a search result, by file and `msgctxt`. */
export function hitKey(hit: SearchHitDto): string {
  return `${hit.path}|${hit.context}`;
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
export function commonTerm(hits: readonly SearchHitDto[]): string | null {
  const [first, ...rest] = hits;
  if (!first) return null;
  const termsOf = (hit: SearchHitDto) => new Set(hit.findings.map(exceptionTerm).filter((term): term is string => term !== null));
  return [...termsOf(first)].find((term) => rest.every((hit) => termsOf(hit).has(term))) ?? null;
}

/**
 * The chosen strings after the result was read again: a chosen string still
 * found takes its new state, one gone from a file whose strings are all
 * known (`complete`) is no longer chosen, and the others stay. Returns
 * `chosen` itself when nothing changed.
 */
export function reconcileChosen(
  chosen: ReadonlyMap<string, SearchHitDto>,
  shown: readonly SearchHitDto[],
  complete: (path: string) => boolean,
): ReadonlyMap<string, SearchHitDto> {
  if (chosen.size === 0) return chosen;
  const byKey = new Map(shown.map((hit) => [hitKey(hit), hit]));
  let changed = false;
  const next = new Map<string, SearchHitDto>();
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
