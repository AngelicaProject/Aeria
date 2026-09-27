import type { MessageKey, Translate } from "./i18n/translate";
import { en } from "./i18n/en.ts";
import type { PreviewVariableDto } from "./types";

/**
 * Words for the variables of game previews. Rust lists the variables a
 * string reads and evaluates the string; this module only names them for
 * the interface language.
 */

function knownKey(key: string): MessageKey | null {
  return key in en ? (key as MessageKey) : null;
}

/** `$gn68` as written in macro text, or null for a character property. */
export function variableCode(variable: PreviewVariableDto): string | null {
  if (variable.parameter) return `$${variable.parameter.prefix}${variable.parameter.index}`;
  return variable.kind === "time" ? `$${variable.key}` : null;
}

/** A short name: the meaning when it is established, else the code. */
export function variableLabel(variable: PreviewVariableDto, t: Translate): string {
  const global = variable.global ? knownKey(`preview.global.${variable.global}`) : null;
  if (global) return t(global);
  if (variable.kind === "character") {
    const key = knownKey(`preview.var.${variable.key}`);
    if (key) return t(key);
  }
  if (variable.kind === "time") {
    const key = knownKey(`preview.time.${variable.key}`);
    if (key) return t(key);
  }
  return variableCode(variable) ?? variable.key;
}

/** What the variable is, in full words, with its code. */
export function variableHint(variable: PreviewVariableDto, t: Translate): string {
  const code = variableCode(variable);
  const summary = variable.global ? knownKey(`preview.globalSummary.${variable.global}`) : null;
  if (summary && code) return t("preview.globalHint", { summary: t(summary), code });
  if (variable.kind === "character") {
    const key = knownKey(`preview.varHint.${variable.key}`);
    return key ? t(key) : variable.key;
  }
  if (variable.kind === "time") {
    return t("preview.var.time", { name: variableLabel(variable, t) });
  }
  const parameter = variable.parameter;
  if (!parameter || !code) return variable.key;
  const base = t(`preview.paramHint.${parameter.prefix}` as MessageKey, { code });
  return variable.sheet ? `${base}. ${t("preview.var.rowOf", { sheet: variable.sheet })}` : base;
}

/** The variables of several previews, each once, in order of first use. */
export function mergeVariables(...lists: readonly (readonly PreviewVariableDto[] | null | undefined)[]): PreviewVariableDto[] {
  const merged = new Map<string, PreviewVariableDto>();
  for (const list of lists) {
    for (const variable of list ?? []) {
      if (!merged.has(variable.key)) merged.set(variable.key, variable);
    }
  }
  return [...merged.values()];
}
