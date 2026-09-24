import type { MessageKey } from "./i18n/translate";
import type { DetachReason, SourceUpdateReportDto } from "./types";

/** The current Workspace Format version written by the backend. */
export const CURRENT_WORKSPACE_FORMAT_VERSION = 2;

export type SourceUpdateFact = {
  key: MessageKey;
  count: number;
  tone: "neutral" | "attention";
};

/**
 * Lists the non-empty outcome counts of a source update in reading order.
 * Units that need review or are detached are highlighted; nothing is hidden.
 * `applied` selects past-tense wording for an update that already ran.
 */
export function sourceUpdateFacts(report: SourceUpdateReportDto, applied: boolean): SourceUpdateFact[] {
  const facts: SourceUpdateFact[] = [
    { key: "sourceUpdate.fact.unchanged", count: report.unchanged, tone: "neutral" },
    { key: "sourceUpdate.fact.encodingChanged", count: report.encodingChanged, tone: "neutral" },
    { key: "sourceUpdate.fact.sourceChanged", count: report.sourceChanged, tone: "attention" },
    { key: "sourceUpdate.fact.rowMoved", count: report.rowMoved, tone: "neutral" },
    { key: "sourceUpdate.fact.columnMapped", count: report.columnMapped, tone: "neutral" },
    { key: "sourceUpdate.fact.reattached", count: report.reattached, tone: "neutral" },
    {
      key: applied ? "sourceUpdate.fact.wereDetached" : "sourceUpdate.fact.newlyDetached",
      count: report.newlyDetached,
      tone: "attention",
    },
    { key: "sourceUpdate.fact.stillDetached", count: report.detached - report.newlyDetached, tone: "neutral" },
  ];
  return facts.filter((fact) => fact.count > 0);
}

/** Whether applying the update also migrates the workspace format. */
export function migratesWorkspaceFormat(report: SourceUpdateReportDto): boolean {
  return report.previousFormatVersion < CURRENT_WORKSPACE_FORMAT_VERSION;
}

export const detachReasonLabels: Readonly<Record<DetachReason, MessageKey>> = {
  sheetRemoved: "sourceUpdate.reason.sheetRemoved",
  sheetUnavailable: "sourceUpdate.reason.sheetUnavailable",
  rowRemoved: "sourceUpdate.reason.rowRemoved",
  cellRemoved: "sourceUpdate.reason.cellRemoved",
  columnUnresolved: "sourceUpdate.reason.columnUnresolved",
  notTranslatable: "sourceUpdate.reason.notTranslatable",
  bindingConflict: "sourceUpdate.reason.bindingConflict",
};
