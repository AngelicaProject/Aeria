import type { MessageKey } from "./i18n/translate";
import type { SourceUpdateReportDto } from "./types";

export type SourceUpdateFact = {
  key: MessageKey;
  count: number;
  tone: "neutral" | "attention";
};

/**
 * Lists the non-empty outcome counts of an update to a new game version in
 * reading order. Translations to check again are highlighted.
 */
export function sourceUpdateFacts(report: SourceUpdateReportDto): SourceUpdateFact[] {
  const facts: SourceUpdateFact[] = [
    { key: "sourceUpdate.fact.fuzzy", count: report.fuzzy, tone: "attention" },
    { key: "sourceUpdate.fact.obsolete", count: report.obsolete, tone: "attention" },
    { key: "sourceUpdate.fact.files", count: report.files, tone: "neutral" },
  ];
  return facts.filter((fact) => fact.count > 0);
}
