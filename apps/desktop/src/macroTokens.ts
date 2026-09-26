/**
 * Presentation-only scanner for macro text tags such as `<num $n1>`,
 * `<if ($gn77 == 3)>`, `<else>`, and `</color>`.
 *
 * It only locates tags for highlighting. Rust (`aeria-se`) remains the
 * authority for parsing, validation, and safety; unterminated or ambiguous
 * input is simply left as plain text here.
 */

export type MacroSpan = {
  from: number;
  to: number;
  /** Name ranges of this tag and of tags nested in its quoted arguments, in source order. */
  names: Array<[number, number]>;
};

export type MacroSegment = { kind: "text" | "macro"; text: string };

const NAME_CHAR = /[A-Za-z0-9:-]/;
const NAME_START = /[A-Za-z/]/;

/** Scans one tag at `start` (a `<`); returns its end, or -1 when it is not a complete tag. */
function scanTag(text: string, start: number, names: Array<[number, number]>): number {
  let index = start + 1;
  const nameStart = index;
  if (text[index] === "/") index += 1;
  const identifierStart = index;
  while (index < text.length && NAME_CHAR.test(text[index]!)) index += 1;
  if (index === identifierStart) return -1;
  const nameIndex = names.length;
  names.push([nameStart, index]);

  let quoted = false;
  while (index < text.length) {
    const character = text[index]!;
    if (character === "\\") {
      index += 2;
    } else if (character === "\"") {
      quoted = !quoted;
      index += 1;
    } else if (quoted && character === "<" && NAME_START.test(text[index + 1] ?? "")) {
      const nestedEnd = scanTag(text, index, names);
      if (nestedEnd < 0) break;
      index = nestedEnd;
    } else if (!quoted && character === "<") {
      break;
    } else if (!quoted && character === ">") {
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
      const end = scanTag(text, index, names);
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
