import type { MessageKey, Translate } from "./i18n/translate";
import type { RunKind, TranslationStatus } from "./types";

/** What each kind of run is called. */
export const RUN_KINDS: Readonly<Record<RunKind, MessageKey>> = {
  translating: "report.kind.translating",
  fixing: "report.kind.fixing",
};

/** What a run's status says it does. */
export function runKind(status: TranslationStatus): RunKind {
  return status.fixing ? "fixing" : "translating";
}

/** The date and time a run started, short. */
export function formatRunTime(startedAt: number, locale: string): string {
  return new Date(startedAt).toLocaleString(locale, { dateStyle: "short", timeStyle: "short" });
}

/** What a run of `kind` is called; a kind of another build is named as a report. */
export function runKindLabel(kind: RunKind, t: Translate): string {
  return t(RUN_KINDS[kind] ?? "report.title");
}

/** The name of a report's tab: its run's kind and time. */
export function reportLabel(kind: RunKind, startedAt: number, t: Translate, locale: string): string {
  return t("report.tab", { kind: runKindLabel(kind, t), time: formatRunTime(startedAt, locale) });
}
