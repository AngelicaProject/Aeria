import type { MessageKey, Translate } from "./i18n/translate";
import { en } from "./i18n/en.ts";
import type { MacroParameterDto, MacroTagDto } from "./types";

/**
 * Localized labels for macro text: tag descriptions for editor hovers.
 * Rust describes the macros; this module only words them for the interface
 * language.
 */

function knownKey(key: string): MessageKey | null {
  return key in en ? (key as MessageKey) : null;
}

/** `$n1` → "number parameter 1 of the string", for hovers. */
export function parameterLabel(parameter: MacroParameterDto, t: Translate): string {
  return t(`preview.param.${parameter.prefix}` as MessageKey, { index: parameter.index });
}

/** A tag's description for an editor hover: a title and lines of detail. */
export function describeTag(tag: MacroTagDto, t: Translate): { title: string; lines: string[] } {
  if (tag.part === "raw") return { title: "<raw>", lines: [t("macro.raw")] };
  if (tag.part === "generic") {
    return { title: `<${tag.name}>`, lines: [t("macro.unknown"), ...tag.args.map((arg) => arg.value)] };
  }
  const summaryKey = knownKey(`macro.summary.${tag.name}`);
  const summary = summaryKey ? t(summaryKey) : "";
  if (tag.part === "close") {
    const detail = tag.family === "formatting" ? t("macro.part.pairClose", { name: tag.name }) : t("macro.part.close", { name: tag.name });
    return { title: `</${tag.name}>`, lines: [detail, summary].filter(Boolean) };
  }
  if (tag.part === "separator") {
    return { title: `<${tag.name}>`, lines: [t("macro.part.separator", { name: tag.name }), summary].filter(Boolean) };
  }
  const lines = [summary];
  for (const arg of tag.args) {
    const nameKey = knownKey(`macro.arg.${arg.name}`);
    const name = nameKey ? t(nameKey) : arg.name;
    const meaning = arg.parameter ? ` — ${parameterLabel(arg.parameter, t)}` : "";
    lines.push(`${name}: ${arg.value}${meaning}`);
  }
  const familyKey = tag.family ? knownKey(`macro.family.${tag.family}`) : null;
  if (familyKey) lines.push(t(familyKey));
  return { title: `<${tag.name}>`, lines: lines.filter(Boolean) };
}

/** The tag that covers `position`, if any. */
export function tagAt(tags: readonly MacroTagDto[], position: number): MacroTagDto | null {
  let found: MacroTagDto | null = null;
  for (const tag of tags) {
    if (tag.from <= position && position <= tag.to) {
      if (!found || tag.to - tag.from < found.to - found.from) found = tag;
    }
  }
  return found;
}
