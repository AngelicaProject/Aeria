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

/** An idiom's name and meaning, by its exact macro text (see `macroIdioms.ts`). */
export type Idioms = ReadonlyMap<string, { name: string; summary: string }>;

export type MacroSegment =
  | { kind: "text" | "macro"; text: string }
  /** Macros that read as one value, such as the player's first name; `text` is their exact macro text. */
  | { kind: "idiom"; text: string; name: string; summary: string };

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

  // Comparisons such as `($n1 >= 2)` sit in parentheses, where `<` and `>` are operators.
  let quoted = false;
  let depth = 0;
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
    } else if (!quoted && character === "(") {
      depth += 1;
      index += 1;
    } else if (!quoted && character === ")") {
      depth = Math.max(0, depth - 1);
      index += 1;
    } else if (!quoted && depth === 0 && character === "<") {
      break;
    } else if (!quoted && depth === 0 && character === ">") {
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

/** A range of macro text. */
export type TextRange = { from: number; to: number };

/**
 * The markers of the speaker name a line starts with, `(-name-)`: the game
 * shows the name in place of the speaking character's own. Mirrors
 * `aeria_se::speaker` for highlighting; `null` when the text has none.
 */
export function speakerMarkers(text: string, tags: readonly MacroSpan[] = scanMacros(text)): { open: TextRange; close: TextRange } | null {
  if (!text.startsWith("(-")) return null;
  let from = 2;
  for (;;) {
    const at = text.indexOf("-)", from);
    if (at < 0) return null;
    const tag = tags.find((span) => span.from < at + 2 && at < span.to);
    if (!tag) return { open: { from: 0, to: 2 }, close: { from: at, to: at + 2 } };
    from = tag.to;
  }
}

/** Tags and the speaker name's markers, in text order: the parts drawn as syntax. */
export function syntaxSpans(text: string): TextRange[] {
  const tags = scanMacros(text);
  const speaker = speakerMarkers(text, tags);
  const spans: TextRange[] = [...tags];
  if (speaker) spans.push(speaker.open, speaker.close);
  return spans.sort((left, right) => left.from - right.from);
}

/**
 * Where idioms occur in `text`, by where they start: only whole runs of
 * tags, from the start of one tag to the end of another.
 */
export function idiomRanges(text: string, spans: readonly TextRange[], idioms: Idioms): Map<number, { to: number; name: string; summary: string }> {
  const ranges = new Map<number, { to: number; name: string; summary: string }>();
  if (idioms.size === 0) return ranges;
  const starts = new Set(spans.map((span) => span.from));
  const ends = new Set(spans.map((span) => span.to));
  for (const [idiom, meaning] of idioms) {
    for (let at = text.indexOf(idiom); at >= 0; at = text.indexOf(idiom, at + 1)) {
      if (starts.has(at) && ends.has(at + idiom.length) && !ranges.has(at)) ranges.set(at, { to: at + idiom.length, ...meaning });
    }
  }
  return ranges;
}

export function segmentMacroText(text: string, idioms: Idioms = new Map()): MacroSegment[] {
  const segments: MacroSegment[] = [];
  const spans = syntaxSpans(text);
  const ranges = idiomRanges(text, spans, idioms);
  let cursor = 0;
  for (const span of spans) {
    if (span.from < cursor) continue;
    if (span.from > cursor) segments.push({ kind: "text", text: text.slice(cursor, span.from) });
    const idiom = ranges.get(span.from);
    if (idiom) {
      segments.push({ kind: "idiom", text: text.slice(span.from, idiom.to), name: idiom.name, summary: idiom.summary });
      cursor = idiom.to;
      continue;
    }
    segments.push({ kind: "macro", text: text.slice(span.from, span.to) });
    cursor = span.to;
  }
  if (cursor < text.length) segments.push({ kind: "text", text: text.slice(cursor) });
  return segments;
}
