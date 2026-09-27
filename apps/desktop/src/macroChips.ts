import type { Translate } from "./i18n/translate";
import { scanMacros } from "./macroTokens.ts";

/**
 * How the text editor shows macro text to a translator: tags become compact
 * chips, formatting pairs disappear into styled text with thin markers at
 * their edges, and `<br>` becomes a line break. The document stays the
 * exact macro text; this module only decides how each tag is drawn.
 * Rust remains the authority for parsing and validation.
 */

export type ChipTone = "value" | "condition" | "break" | "icon" | "space" | "unknown";

export type ChipSpec =
  /** A tag drawn as one chip. */
  | { kind: "chip"; from: number; to: number; label: string; tone: ChipTone; icon?: number }
  /** Adjacent opening or closing formatting tags, drawn as one thin marker. */
  | { kind: "marker"; from: number; to: number; side: "open" | "close"; color: string | null }
  /** Text inside formatting pairs, drawn with their style. */
  | { kind: "style"; from: number; to: number; color: string | null; italic: boolean; bold: boolean };

/** Tags that open and close a formatting pair. */
const FORMATTING = new Set(["color", "edge-color", "ui-color", "ui-edge-color", "shadow-color", "i", "b"]);
/** Formatting tags whose color is the text color. */
const FOREGROUND = new Set(["color", "ui-color"]);
const BLOCKS = new Set(["if", "switch", "if-gender", "if-self", "if-name", "josa", "josa-ro", "ruby"]);
const SEPARATORS = new Set(["else", "case", "rt"]);
const NUMBERS = new Set(["num", "num2", "hex", "kilo", "byte", "float", "digit", "ordinal", "sec", "time"]);
const REFERENCES = new Set(["sheet", "sheet-sub", "noun-ja", "noun-en", "noun-de", "noun-fr", "noun-zh"]);

type Tag = { from: number; to: number; name: string; closing: boolean; args: string[]; text: string };

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

function tagsOf(text: string): Tag[] {
  return scanMacros(text).map((span) => {
    const [nameFrom, nameTo] = span.names[0]!;
    const raw = text.slice(nameFrom, nameTo);
    const closing = raw.startsWith("/");
    return {
      from: span.from,
      to: span.to,
      name: closing ? raw.slice(1) : raw,
      closing,
      args: splitArgs(text.slice(nameTo, span.to - 1)),
      text: text.slice(span.from, span.to),
    };
  });
}

function unquote(value: string): string {
  return value.length >= 2 && value.startsWith("\"") && value.endsWith("\"") ? value.slice(1, -1) : value;
}

function blockLabel(tag: Tag, t: Translate): string {
  if (tag.closing) return t("chip.end");
  const condition = tag.args.join(" ");
  switch (tag.name) {
    case "if": return t("chip.if", { condition });
    case "switch": return t("chip.switch", { value: condition });
    case "if-gender": return t("chip.ifGender");
    case "if-self": return t("chip.ifSelf");
    case "if-name": return t("chip.ifName");
    case "ruby": return t("chip.ruby");
    default: return t("chip.josa");
  }
}

function chipOf(tag: Tag, t: Translate): { label: string; tone: ChipTone; icon?: number } {
  const first = tag.args[0] ?? "";
  if (BLOCKS.has(tag.name)) return { label: blockLabel(tag, t), tone: "condition" };
  if (SEPARATORS.has(tag.name) && !tag.closing) return { label: t(`chip.${tag.name}` as "chip.else"), tone: "condition" };
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

/**
 * How to draw every tag of `text`. `colorOf` gives the color an opening
 * color tag sets, from the latest Rust view, or null when it is not known.
 */
export function chipSpecs(text: string, colorOf: (tag: string) => string | null, t: Translate): ChipSpec[] {
  const specs: ChipSpec[] = [];
  const open: { name: string; color: string | null }[] = [];
  let cursor = 0;
  const style = (from: number, to: number) => {
    if (from >= to || open.length === 0) return;
    const color = [...open].reverse().find((entry) => FOREGROUND.has(entry.name))?.color ?? null;
    const italic = open.some((entry) => entry.name === "i");
    const bold = open.some((entry) => entry.name === "b");
    if (color || italic || bold) specs.push({ kind: "style", from, to, color, italic, bold });
  };
  for (const tag of tagsOf(text)) {
    style(cursor, tag.from);
    cursor = tag.to;
    if (!FORMATTING.has(tag.name)) {
      specs.push({ kind: "chip", from: tag.from, to: tag.to, ...chipOf(tag, t) });
      continue;
    }
    const side = tag.closing ? "close" : "open";
    const color = !tag.closing && FOREGROUND.has(tag.name) ? colorOf(tag.text) : null;
    if (tag.closing) {
      const index = open.map((entry) => entry.name).lastIndexOf(tag.name);
      if (index >= 0) open.splice(index, 1);
    } else {
      open.push({ name: tag.name, color });
    }
    // Adjacent tags of one side, such as a color and its outline, are one marker.
    const previous = specs.at(-1);
    if (previous?.kind === "marker" && previous.side === side && previous.to === tag.from) {
      previous.to = tag.to;
      previous.color ??= color;
    } else {
      specs.push({ kind: "marker", from: tag.from, to: tag.to, side, color });
    }
  }
  style(cursor, text.length);
  return specs;
}
