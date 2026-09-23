export type FuzzyMatch = {
  score: number;
  /** Indices in the candidate that matched query characters. */
  indices: number[];
};

function isBoundary(text: string, index: number): boolean {
  if (index === 0) return true;
  const previous = text[index - 1]!;
  const current = text[index]!;
  return /[\s/_\-.:·]/.test(previous) || (previous === previous.toLowerCase() && current !== current.toLowerCase());
}

/**
 * Subsequence match in the style of editor quick-open: every query character
 * must appear in order. Consecutive runs, word starts, and early matches score
 * higher. Returns null when the query does not match.
 */
export function fuzzyMatch(query: string, candidate: string): FuzzyMatch | null {
  const needle = query.trim().toLocaleLowerCase().replace(/\s+/g, "");
  if (!needle) return { score: 0, indices: [] };
  const haystack = candidate.toLocaleLowerCase();
  const indices: number[] = [];
  let score = 0;
  let searchFrom = 0;
  let previous = -2;
  for (const character of needle) {
    let found = -1;
    // Prefer a word-boundary occurrence close ahead over the first raw hit.
    for (let index = searchFrom; index < haystack.length; index += 1) {
      if (haystack[index] !== character) continue;
      if (found < 0) found = index;
      if (index === previous + 1 || isBoundary(candidate, index)) { found = index; break; }
    }
    if (found < 0) return null;
    score += found === previous + 1 ? 6 : isBoundary(candidate, found) ? 4 : 1;
    indices.push(found);
    previous = found;
    searchFrom = found + 1;
  }
  score -= indices[0]! * 0.05 + candidate.length * 0.01;
  if (haystack.startsWith(needle)) score += 8;
  if (haystack === needle) score += 20;
  return { score, indices };
}

export function fuzzyFilter<T>(query: string, items: readonly T[], text: (item: T) => string, limit = 100): Array<{ item: T; match: FuzzyMatch }> {
  const results: Array<{ item: T; match: FuzzyMatch }> = [];
  for (const item of items) {
    const match = fuzzyMatch(query, text(item));
    if (match) results.push({ item, match });
  }
  if (query.trim()) results.sort((left, right) => right.match.score - left.match.score);
  return results.slice(0, limit);
}
