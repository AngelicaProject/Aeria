import type { Translate } from "./i18n/translate";
import { chipSpecs, type ChipLookup, type ChipTone } from "./macroChips.ts";
import type { ChipPick } from "./components/macroChipsExtension";
import type { MacroRule, MacroTagDto } from "./types";

/**
 * The macros of a source string for the string guide: each part the editor
 * draws as a chip or a formatting marker, once, with what a translation may
 * do with it. Rust says what each macro is (`MacroTagDto.rule`); this module
 * only groups what the editor already draws.
 */

/** Groups in the order the guide lists them: what must stay first. */
export const ruleOrder: readonly MacroRule[] = ["keep", "condition", "letterCase", "formatting", "free"];

/** Which rule a part of several macros follows: the strictest of them. */
const strictness: Readonly<Record<MacroRule, number>> = { keep: 4, condition: 3, letterCase: 2, formatting: 1, free: 0 };

export type GuideMacro = {
  rule: MacroRule;
  /** How the editor draws the part; `format` for a formatting pair. */
  tone: ChipTone | "format";
  label: string;
  title?: string;
  icon?: number;
  /** The text color a formatting pair sets. */
  color?: string | null;
  /** What picking the part adds to the translation. */
  pick: ChipPick;
  /** The game data of the part, by its spelling in the source. */
  data: string[];
  /** How many times the source has the part. */
  count: number;
};

/** The kind of a formatting pair, by its first tag. */
export function formatKind(opening: string): "italic" | "bold" | "color" {
  const name = /^<([A-Za-z-]+)/.exec(opening)?.[1] ?? "";
  if (name === "i") return "italic";
  if (name === "b") return "bold";
  return "color";
}

/** The guide's parts of `text`, whose tags Rust described as `tags`. */
export function guideMacros(text: string, tags: readonly MacroTagDto[], lookup: ChipLookup, t: Translate): GuideMacro[] {
  const found: GuideMacro[] = [];
  const add = (macro: GuideMacro) => {
    const same = found.find((candidate) => candidate.rule === macro.rule && candidate.label === macro.label && candidate.tone === macro.tone);
    if (same) {
      same.count += 1;
      for (const spelling of macro.data) if (!same.data.includes(spelling)) same.data.push(spelling);
    } else {
      found.push(macro);
    }
  };
  for (const spec of chipSpecs(text, lookup, t)) {
    if (spec.kind === "marker") {
      if (spec.side !== "open" || !spec.wrap) continue;
      const kind = formatKind(spec.wrap[0]);
      add({ rule: "formatting", tone: "format", label: t(`hints.format.${kind}`), title: `${spec.wrap[0]}…${spec.wrap[1]}`, color: spec.color, pick: { wrap: spec.wrap }, data: [], count: 1 });
      continue;
    }
    if (spec.kind !== "chip") continue;
    const inner = tags.filter((tag) => tag.from >= spec.from && tag.to <= spec.to);
    const first = inner.find((tag) => tag.from === spec.from);
    // A `{240}` value, a speaker marker, and the separators and ends of a
    // condition are not parts of their own.
    if (!first || first.part === "close" || first.part === "separator") continue;
    const rule = first.rule === "condition"
      ? "condition"
      : inner.reduce<MacroRule>((strictest, tag) => strictness[tag.rule] > strictness[strictest] ? tag.rule : strictest, first.rule);
    const data = inner.filter((tag) => tag.rule === "keep" && tag.part !== "close" && tag.part !== "separator").map((tag) => text.slice(tag.from, tag.to));
    add({
      rule,
      tone: spec.tone,
      label: spec.label,
      ...(spec.title ? { title: spec.title } : {}),
      ...(spec.icon !== undefined ? { icon: spec.icon } : {}),
      pick: { insert: spec.insert },
      data,
      count: 1,
    });
  }
  return found.sort((left, right) => ruleOrder.indexOf(left.rule) - ruleOrder.indexOf(right.rule));
}

/** Whether a translation has a part: `missing` when it lacks game data of it. */
export function macroState(macro: GuideMacro, missing: readonly string[]): "present" | "missing" | null {
  if (macro.rule !== "keep" || macro.data.length === 0) return null;
  return macro.data.some((spelling) => missing.includes(spelling)) ? "missing" : "present";
}
