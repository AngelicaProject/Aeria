export type DiffPart = { kind: "same" | "added" | "removed"; text: string };

const MAX_CELLS = 250_000;

function tokenize(text: string): string[] {
  return text.match(/<[^<>]*>|\s+|[\p{L}\p{N}_]+|./gsu) ?? [];
}

function push(parts: DiffPart[], kind: DiffPart["kind"], text: string) {
  const last = parts.at(-1);
  if (last && last.kind === kind) last.text += text;
  else parts.push({ kind, text });
}

/**
 * Word-level diff for presentation. Macro spellings, whitespace runs, and
 * words are atomic tokens. Very long inputs fall back to a whole replacement.
 */
export function diffWords(before: string, after: string): DiffPart[] {
  if (before === after) return before ? [{ kind: "same", text: before }] : [];
  const left = tokenize(before);
  const right = tokenize(after);
  if (left.length * right.length > MAX_CELLS) {
    return [...(before ? [{ kind: "removed" as const, text: before }] : []), ...(after ? [{ kind: "added" as const, text: after }] : [])];
  }
  const rows = left.length + 1;
  const columns = right.length + 1;
  const lengths = new Uint32Array(rows * columns);
  for (let i = left.length - 1; i >= 0; i -= 1) {
    for (let j = right.length - 1; j >= 0; j -= 1) {
      lengths[i * columns + j] = left[i] === right[j]
        ? lengths[(i + 1) * columns + j + 1]! + 1
        : Math.max(lengths[(i + 1) * columns + j]!, lengths[i * columns + j + 1]!);
    }
  }
  const parts: DiffPart[] = [];
  let i = 0;
  let j = 0;
  while (i < left.length && j < right.length) {
    if (left[i] === right[j]) {
      push(parts, "same", left[i]!);
      i += 1;
      j += 1;
    } else if (lengths[(i + 1) * columns + j]! >= lengths[i * columns + j + 1]!) {
      push(parts, "removed", left[i]!);
      i += 1;
    } else {
      push(parts, "added", right[j]!);
      j += 1;
    }
  }
  while (i < left.length) push(parts, "removed", left[i++]!);
  while (j < right.length) push(parts, "added", right[j++]!);
  return parts;
}
