import { en } from "./i18n/en.ts";
import type { MessageKey, Translate } from "./i18n/translate";
import { scanMacros } from "./macroTokens.ts";
import type { MacroConditionDto, MacroOperandDto } from "./types";

/**
 * How the text editor shows macro text to a translator: tags become compact
 * chips, conditions read in words, formatting pairs disappear into styled
 * text with thin markers at their edges, `<br>` and the branches of
 * complex conditions start new lines, and `{240}` reads as the value it
 * is. The document stays the exact macro text; this module only decides
 * how each part is drawn. Rust remains the authority for parsing and
 * validation.
 */

export type ChipTone = "value" | "number" | "condition" | "break" | "icon" | "space" | "unknown";

export type ChipSpec =
  /** A tag or `{value}` drawn as one chip; `insert` is what picking it adds to a translation. */
  | { kind: "chip"; from: number; to: number; label: string; tone: ChipTone; icon?: number; insert: string }
  /** Adjacent opening or closing formatting tags, drawn as one thin marker; picking it wraps text in the pair. */
  | { kind: "marker"; from: number; to: number; side: "open" | "close"; color: string | null; wrap: readonly [string, string] | null }
  /** Text inside formatting pairs, drawn with their style. */
  | { kind: "style"; from: number; to: number; color: string | null; italic: boolean; bold: boolean }
  /**
   * A new visual line at `at`, indented by `indent` levels. `assoc` says on
   * which side of the break a cursor at `at` belongs: -1 at the end of the
   * line before it, 1 at the start of the line after it.
   */
  | { kind: "break"; at: number; indent: number; assoc: -1 | 1 };

/** What chips read from the latest Rust views of macro text. */
export type ChipLookup = {
  /** The color an opening color tag sets, by the tag's text. */
  colorOf: (tag: string) => string | null;
  /** What an opening `<if>` or `<switch>` tests, by the tag's text. */
  conditionOf: (tag: string) => MacroConditionDto | null;
};

/** Tags that open and close a formatting pair. */
const FORMATTING = new Set(["color", "edge-color", "ui-color", "ui-edge-color", "shadow-color", "i", "b"]);
/** Formatting tags whose color is the text color. */
const FOREGROUND = new Set(["color", "ui-color"]);
const BLOCKS = new Set(["if", "switch", "if-gender", "if-self", "if-name", "josa", "josa-ro", "ruby"]);
const SEPARATORS = new Set(["else", "case", "rt"]);
const NUMBERS = new Set(["num", "num2", "hex", "kilo", "byte", "float", "digit", "ordinal", "sec", "time"]);
const REFERENCES = new Set(["sheet", "sheet-sub", "noun-ja", "noun-en", "noun-de", "noun-fr", "noun-zh"]);
const OPERATORS: Readonly<Record<string, string>> = { "==": "=", "!=": "≠", "<": "<", "<=": "≤", ">": ">", ">=": "≥" };

type Tag = { from: number; to: number; name: string; closing: boolean; args: string[]; text: string };
type Item = { type: "tag"; tag: Tag } | { type: "brace"; from: number; to: number; inner: string };

/** Splits tag arguments at spaces outside quotes and parentheses. */
function splitArgs(text: string): string[] {
  const args: string[] = [];
  let current = "";
  let depth = 0;
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index]!;
    if (character === "\\") {
      current += character + (text[index + 1] ?? "");
      index += 1;
      continue;
    }
    if (character === "\"") quoted = !quoted;
    else if (!quoted && character === "(") depth += 1;
    else if (!quoted && character === ")") depth = Math.max(0, depth - 1);
    if (!quoted && depth === 0 && /\s/.test(character)) {
      if (current) args.push(current);
      current = "";
    } else {
      current += character;
    }
  }
  if (current) args.push(current);
  return args;
}

/** `{value}` spans in text between tags; `\{` is a written brace. */
function braces(text: string, from: number, to: number): Item[] {
  const items: Item[] = [];
  for (let index = from; index < to; index += 1) {
    const character = text[index];
    if (character === "\\") {
      index += 1;
    } else if (character === "{") {
      const end = text.indexOf("}", index + 1);
      if (end < 0 || end >= to) break;
      items.push({ type: "brace", from: index, to: end + 1, inner: text.slice(index + 1, end).trim() });
      index = end;
    }
  }
  return items;
}

function itemsOf(text: string): Item[] {
  const items: Item[] = [];
  let cursor = 0;
  for (const span of scanMacros(text)) {
    items.push(...braces(text, cursor, span.from));
    const [nameFrom, nameTo] = span.names[0]!;
    const raw = text.slice(nameFrom, nameTo);
    const closing = raw.startsWith("/");
    items.push({
      type: "tag",
      tag: {
        from: span.from,
        to: span.to,
        name: closing ? raw.slice(1) : raw,
        closing,
        args: splitArgs(text.slice(nameTo, span.to - 1)),
        text: text.slice(span.from, span.to),
      },
    });
    cursor = span.to;
  }
  items.push(...braces(text, cursor, text.length));
  return items;
}

function unquote(value: string): string {
  return value.length >= 2 && value.startsWith("\"") && value.endsWith("\"") ? value.slice(1, -1) : value;
}

function operandLabel(operand: MacroOperandDto, t: Translate): string {
  switch (operand.kind) {
    case "int":
      return operand.name ?? String(operand.value);
    case "parameter": {
      const key = operand.meaning ? `condition.global.${operand.meaning}` : null;
      return key && key in en ? t(key as MessageKey) : operand.code;
    }
    case "time": {
      const key = `condition.time.${operand.name}`;
      return key in en ? t(key as MessageKey) : `$${operand.name}`;
    }
    case "other":
      return operand.text;
  }
}

/** A condition in words, such as "class = monk" or "level ≥ 94". */
export function conditionLabel(condition: MacroConditionDto, t: Translate): string {
  const left = operandLabel(condition.left, t);
  if (!condition.operator || !condition.right) return left;
  return `${left} ${OPERATORS[condition.operator] ?? condition.operator} ${operandLabel(condition.right, t)}`;
}

function blockLabel(tag: Tag, lookup: ChipLookup, t: Translate): string {
  const condition = lookup.conditionOf(tag.text);
  const written = tag.args.join(" ");
  switch (tag.name) {
    case "if": return t("chip.if", { condition: condition ? conditionLabel(condition, t) : written });
    case "switch": return t("chip.switch", { value: condition ? conditionLabel(condition, t) : written });
    case "if-gender": return t("chip.ifGender");
    case "if-self": return t("chip.ifSelf");
    case "if-name": return t("chip.ifName");
    case "ruby": return t("chip.ruby");
    default: return t("chip.josa");
  }
}

function chipOf(tag: Tag, t: Translate): { label: string; tone: ChipTone; icon?: number } {
  const first = tag.args[0] ?? "";
  switch (tag.name) {
    case "br": return { label: "↵", tone: "break" };
    case "nbsp": return { label: "⍽", tone: "space" };
    case "shy": return { label: "-", tone: "space" };
    case "hyphen": return { label: "‑", tone: "space" };
    case "player-name": return { label: t("preview.value.playerName"), tone: "value" };
    case "icon":
    case "icon2": {
      const icon = Number.parseInt(first, 10);
      return Number.isInteger(icon) ? { label: first, tone: "icon", icon } : { label: tag.text, tone: "unknown" };
    }
  }
  if (REFERENCES.has(tag.name)) {
    const row = tag.args[tag.name.startsWith("noun") ? 2 : 1] ?? "";
    return { label: `${unquote(first)} · ${row}`, tone: "value" };
  }
  if (NUMBERS.has(tag.name) || tag.name === "string" || tag.name === "split") return { label: first || tag.name, tone: "value" };
  if (tag.name.startsWith("code:") || tag.name === "raw") return { label: tag.text, tone: "unknown" };
  return { label: tag.name, tone: "unknown" };
}

type Block = { open: number; close: number | null; complex: boolean; parent: number | null };

/**
 * Pairs condition blocks. A block is complex when it holds another block or
 * a line break; its tags then start lines, indented by their nesting.
 */
function blocksOf(items: readonly Item[]): { blocks: Block[]; blockOf: Map<number, number> } {
  const blocks: Block[] = [];
  const blockOf = new Map<number, number>();
  const stack: number[] = [];
  const markComplex = () => {
    for (const index of stack) blocks[index]!.complex = true;
  };
  items.forEach((item, index) => {
    if (item.type !== "tag") return;
    const { tag } = item;
    if (BLOCKS.has(tag.name) && !tag.closing) {
      markComplex();
      blocks.push({ open: index, close: null, complex: false, parent: stack.at(-1) ?? null });
      stack.push(blocks.length - 1);
      blockOf.set(index, blocks.length - 1);
    } else if (SEPARATORS.has(tag.name) && !tag.closing && stack.length > 0) {
      blockOf.set(index, stack.at(-1)!);
    } else if (BLOCKS.has(tag.name) && tag.closing && stack.length > 0) {
      const block = stack.pop()!;
      blocks[block]!.close = index;
      blockOf.set(index, block);
    } else if (tag.name === "br") {
      markComplex();
    }
  });
  return { blocks, blockOf };
}

/** How many complex blocks enclose `block`, not counting itself. */
function depthOf(blocks: readonly Block[], block: number | null): number {
  let depth = 0;
  for (let current = block === null ? null : blocks[block]!.parent; current !== null; current = blocks[current]!.parent) {
    if (blocks[current]!.complex) depth += 1;
  }
  return depth;
}

/** Closing tags for a group of opening formatting tags, innermost first. */
function closingTags(open: string): string {
  return [...open.matchAll(/<([A-Za-z-]+)/g)].map((match) => `</${match[1]}>`).reverse().join("");
}

/** How to draw every part of `text`. */
export function chipSpecs(text: string, lookup: ChipLookup, t: Translate): ChipSpec[] {
  const items = itemsOf(text);
  const { blocks, blockOf } = blocksOf(items);
  const specs: ChipSpec[] = [];
  const open: { name: string; color: string | null }[] = [];
  const cases = new Map<number, number>();
  /** Complex blocks enclosing the current position. */
  const inside: number[] = [];
  let cursor = 0;
  const style = (from: number, to: number) => {
    if (from >= to || open.length === 0) return;
    const color = [...open].reverse().find((entry) => FOREGROUND.has(entry.name))?.color ?? null;
    const italic = open.some((entry) => entry.name === "i");
    const bold = open.some((entry) => entry.name === "b");
    if (color || italic || bold) specs.push({ kind: "style", from, to, color, italic, bold });
  };
  // A break at the very end only after `<br>`: the cursor after a new line
  // break needs the line it starts.
  const lineBreak = (at: number, indent: number, assoc: -1 | 1, atEnd = false) => {
    if (at <= 0 || at > text.length || (at === text.length && !atEnd)) return;
    const previous = specs.at(-1);
    if (previous?.kind === "break" && previous.at === at) {
      previous.indent = indent;
      previous.assoc = assoc;
      return;
    }
    specs.push({ kind: "break", at, indent, assoc });
  };

  items.forEach((item, index) => {
    style(cursor, item.type === "brace" ? item.from : item.tag.from);
    if (item.type === "brace") {
      specs.push({ kind: "chip", from: item.from, to: item.to, label: item.inner, tone: "number", insert: text.slice(item.from, item.to) });
      cursor = item.to;
      return;
    }
    const { tag } = item;
    cursor = tag.to;
    const blockIndex = blockOf.get(index);
    if (blockIndex !== undefined) {
      const block = blocks[blockIndex]!;
      const depth = depthOf(blocks, blockIndex);
      if (block.complex) lineBreak(tag.from, depth, -1);
      let label: string;
      let insert = tag.text;
      if (!tag.closing && BLOCKS.has(tag.name)) {
        label = blockLabel(tag, lookup, t);
        const close = block.close === null ? null : items[block.close];
        if (close?.type === "tag") insert = text.slice(tag.from, close.tag.to);
        if (block.complex) inside.push(blockIndex);
      } else if (tag.closing) {
        label = t("chip.end");
        if (inside.at(-1) === blockIndex) inside.pop();
      } else if (tag.name === "case") {
        const count = (cases.get(blockIndex) ?? 0) + 1;
        cases.set(blockIndex, count);
        label = t("chip.case", { n: count });
      } else {
        label = t(`chip.${tag.name}` as "chip.else");
      }
      specs.push({ kind: "chip", from: tag.from, to: tag.to, label, tone: "condition", insert });
      if (tag.closing && block.complex) {
        const next = items[index + 1];
        const nextIsBreak = next?.type === "tag" && next.tag.name === "br" && next.tag.from === tag.to;
        if (!nextIsBreak) lineBreak(tag.to, depth, 1);
      }
      return;
    }
    if (!FORMATTING.has(tag.name)) {
      specs.push({ kind: "chip", from: tag.from, to: tag.to, ...chipOf(tag, t), insert: tag.text });
      if (tag.name === "br") lineBreak(tag.to, inside.length, 1, true);
      return;
    }
    const side = tag.closing ? "close" : "open";
    const color = !tag.closing && FOREGROUND.has(tag.name) ? lookup.colorOf(tag.text) : null;
    if (tag.closing) {
      const openIndex = open.map((entry) => entry.name).lastIndexOf(tag.name);
      if (openIndex >= 0) open.splice(openIndex, 1);
    } else {
      open.push({ name: tag.name, color });
    }
    // Adjacent tags of one side, such as a color and its outline, are one marker.
    const previous = specs.at(-1);
    if (previous?.kind === "marker" && previous.side === side && previous.to === tag.from) {
      previous.to = tag.to;
      previous.color ??= color;
    } else {
      specs.push({ kind: "marker", from: tag.from, to: tag.to, side, color, wrap: null });
    }
  });
  style(cursor, text.length);

  // Pair markers so picking either edge wraps text in the whole pair.
  const openMarkers: Extract<ChipSpec, { kind: "marker" }>[] = [];
  for (const spec of specs) {
    if (spec.kind !== "marker") continue;
    if (spec.side === "open") {
      openMarkers.push(spec);
      continue;
    }
    const partner = openMarkers.pop();
    if (!partner) continue;
    const pair = [text.slice(partner.from, partner.to), text.slice(spec.from, spec.to)] as const;
    partner.wrap = pair;
    spec.wrap = pair;
  }
  for (const spec of openMarkers) {
    const opening = text.slice(spec.from, spec.to);
    spec.wrap = [opening, closingTags(opening)];
  }
  return specs;
}
