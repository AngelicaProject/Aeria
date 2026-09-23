/**
 * Presentation-only scanner for Lumina macro-string spellings such as
 * `<num(t_day)>`, `<if([gnum77==3],<num(t_day)>,x)>`, and `</color>`.
 *
 * It only locates delimiters for highlighting. Rust (`aeria-se`) remains the
 * authority for parsing, validation, and safety; unterminated or ambiguous
 * input is simply left as plain text here.
 */
export type MacroSpan = {
  from: number;
  to: number;
  /** Name ranges of this macro and every nested macro, in source order. */
  names: Array<[number, number]>;
};

export type MacroSegment = { kind: "text" | "macro"; text: string };

const NAME_CHAR = /[A-Za-z0-9_]/;
const NAME_START = /[A-Za-z/]/;

function scanMacro(text: string, start: number, names: Array<[number, number]>): number {
  let index = start + 1;
  const nameStart = index;
  if (text[index] === "/") index += 1;
  const identifierStart = index;
  while (index < text.length && NAME_CHAR.test(text[index]!)) index += 1;
  if (index === identifierStart) return -1;
  const nameIndex = names.length;
  names.push([nameStart, index]);

  let depth = 0;
  while (index < text.length) {
    const character = text[index]!;
    if (character === "\\") {
      index += 2;
    } else if (character === "(") {
      depth += 1;
      index += 1;
    } else if (character === ")") {
      depth -= 1;
      index += 1;
    } else if (character === "<" && NAME_START.test(text[index + 1] ?? "")) {
      const nestedEnd = scanMacro(text, index, names);
      if (nestedEnd < 0) break;
      index = nestedEnd;
    } else if (character === ">" && depth <= 0) {
      return index + 1;
    } else {
      index += 1;
    }
  }
  names.length = nameIndex;
  return -1;
}

export function scanMacros(text: string): MacroSpan[] {
  const spans: MacroSpan[] = [];
  let index = 0;
  while (index < text.length) {
    const character = text[index]!;
    if (character === "\\") {
      index += 2;
      continue;
    }
    if (character === "<" && NAME_START.test(text[index + 1] ?? "")) {
      const names: Array<[number, number]> = [];
      const end = scanMacro(text, index, names);
      if (end > 0) {
        spans.push({ from: index, to: end, names });
        index = end;
        continue;
      }
    }
    index += 1;
  }
  return spans;
}

export function segmentMacroText(text: string): MacroSegment[] {
  const segments: MacroSegment[] = [];
  let cursor = 0;
  for (const span of scanMacros(text)) {
    if (span.from > cursor) segments.push({ kind: "text", text: text.slice(cursor, span.from) });
    segments.push({ kind: "macro", text: text.slice(span.from, span.to) });
    cursor = span.to;
  }
  if (cursor < text.length) segments.push({ kind: "text", text: text.slice(cursor) });
  return segments;
}
