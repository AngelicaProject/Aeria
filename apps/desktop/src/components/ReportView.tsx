import { useCallback, useEffect, useRef, useState } from "react";
import { normalizeCommandError, runReport, runReports, translationContinue } from "../ipc";
import type { MessageKey } from "../i18n/translate";
import { formatRunTime, reportLabel, runKindLabel } from "../reportLabel";
import { describeIssue } from "../issueText";
import type { ReportInfo, ReportItem, ReportPage, ReportRun, ReportString, SourceBinding } from "../types";
import { useI18n } from "../ui/i18n";
import { Select } from "../ui/primitives/Select";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { describeStop } from "./TranslateDialog";

/** Items read at a time. */
const PAGE = 100;
/** How often a report whose run goes is read again. */
const REFRESH_MS = 3000;

type Facet = { view: string; label: MessageKey; icon: UiIconName; tone?: "danger" | "warning" };

const STRINGS: readonly Facet[] = [
  { view: "string:refused", label: "report.view.stringRefused", icon: "circleAlert", tone: "danger" },
  { view: "string:translated", label: "report.view.stringTranslated", icon: "languages" },
  { view: "string:changed", label: "report.view.stringChanged", icon: "pencil" },
  { view: "string:kept", label: "report.view.stringKept", icon: "check" },
  { view: "string:unchanged", label: "report.view.stringUnchanged", icon: "minus" },
  { view: "string:stale", label: "report.view.stringStale", icon: "clock" },
  { view: "left", label: "report.view.left", icon: "cloudOff", tone: "danger" },
];

function termsText(terms: readonly string[]): string {
  return terms.join(", ");
}

function StringItem({ item, onReveal }: { item: ReportString; onReveal: (binding: SourceBinding) => void }) {
  const { t } = useI18n();
  // A field another build does not write reads as empty.
  const attempts = item.attempts ?? [];
  const failed = attempts.some((attempt) => (attempt.problems ?? []).length > 0);
  const binding = item.binding;
  return (
    <li className={`report-item${item.outcome === "refused" ? " refused" : ""}`}>
      <div className="report-item-head">
        {binding
          ? <button type="button" className="link-button report-context" title={t("report.string.open")} onClick={() => onReveal(binding)}>{item.context}</button>
          : <span className="report-context muted">{item.context}</span>}
      </div>
      <dl className="report-texts">
        <dt>{t("report.string.source")}</dt><dd>{item.source}</dd>
        {item.before ? <><dt>{t("report.string.before")}</dt><dd>{item.before}</dd></> : null}
        {item.after !== undefined && item.after !== item.before ? <><dt>{t("report.string.after")}</dt><dd>{item.after}</dd></> : null}
        {item.reason ? <><dt>{t("report.string.reason")}</dt><dd>{item.reason}</dd></> : null}
      </dl>
      {failed ? (
        <ol className="report-attempts">
          {attempts.map((attempt, index) => (
            <li key={index}>
              <span className="report-attempt-label">{t("report.attempt", { number: String(index + 1) })}</span>
              <span>{attempt.translation || t("report.string.noTranslation")}</span>
              {(attempt.problems ?? []).map((problem, at) => <span key={at} className="report-problem"><UiIcon icon="circleAlert" size="xs" />{describeIssue(problem, t)}</span>)}
            </li>
          ))}
        </ol>
      ) : null}
    </li>
  );
}

function Item({ item, onReveal }: { item: ReportItem; onReveal: (binding: SourceBinding) => void }) {
  const { t } = useI18n();
  if (item.record === "string") return <StringItem item={item} onReveal={onReveal} />;
  return (
    <li className="report-item refused">
      <div className="report-item-head"><strong>{t("report.left.title", { count: item.strings })}</strong></div>
      <p className="report-problem"><UiIcon icon="circleAlert" size="xs" />{item.error}</p>
      {item.paths.length > 0 ? <p className="muted">{item.paths.join(", ")}</p> : null}
    </li>
  );
}

type ReportViewProps = {
  reportId: string;
  onRevealBinding: (binding: SourceBinding) => void;
  onOpenReport: (reportId: string, label: string) => void;
};

/** A run's report: what became of every item it asked, by what became of it, with a search. */
export function ReportView({ reportId, onRevealBinding, onOpenReport }: ReportViewProps) {
  const { t, locale, formatNumber } = useI18n();
  const [view, setView] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(PAGE);
  const [page, setPage] = useState<ReportPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reports, setReports] = useState<ReportInfo[]>([]);
  const chosen = useRef(false);

  useEffect(() => {
    void runReports().then(setReports).catch(() => setReports([]));
  }, [reportId]);

  const read = useCallback((current: { live: boolean }) => {
    void runReport(reportId, view ?? "", query, 0, limit)
      .then((found) => {
        if (!current.live) return;
        setPage(found);
        setError(null);
        // Failures come first when the run has any; else the first view with items.
        if (!chosen.current && view === null) {
          const first = STRINGS.find((facet) => facet.tone === "danger" && (found.counts[facet.view] ?? 0) > 0)
            ?? STRINGS.find((facet) => (found.counts[facet.view] ?? 0) > 0);
          if (first) setView(first.view);
        }
      })
      .catch((reason: unknown) => { if (current.live) setError(normalizeCommandError(reason).message); });
  }, [reportId, view, query, limit]);

  useEffect(() => {
    const current = { live: true };
    read(current);
    return () => { current.live = false; };
  }, [read]);

  // A report whose run goes is read again until it stops.
  const going = page?.running ?? false;
  const [continuing, setContinuing] = useState(false);
  const continueRun = () => {
    setContinuing(true);
    setError(null);
    void translationContinue(reportId)
      .then(() => runReports().then(setReports))
      .catch((reason: unknown) => setError(normalizeCommandError(reason).message))
      .finally(() => setContinuing(false));
  };
  useEffect(() => {
    if (!going) return;
    const current = { live: true };
    const timer = window.setInterval(() => read(current), REFRESH_MS);
    return () => { current.live = false; window.clearInterval(timer); };
  }, [going, read]);

  const choose = (next: string) => {
    chosen.current = true;
    setView(next);
    setLimit(PAGE);
  };
  const run: ReportRun | null = page?.run ?? null;
  const facet = (item: Facet) => {
    const count = page?.counts[item.view] ?? 0;
    if (count === 0 && view !== item.view) return null;
    return (
      <button key={item.view} type="button" className={`glossary-facet${view === item.view ? " active" : ""}${item.tone && count > 0 ? ` ${item.tone}` : ""}`} aria-pressed={view === item.view} onClick={() => choose(item.view)}>
        <UiIcon icon={item.icon} size="xs" />
        <span className="glossary-facet-label">{t(item.label)}</span>
        <span className="glossary-facet-count">{count.toLocaleString()}</span>
      </button>
    );
  };
  const hasStrings = STRINGS.some((item) => (page?.counts[item.view] ?? 0) > 0);

  return (
    <div className="glossary-view report-view">
      <div className="report-head">
        <UiIcon icon="clipboard" size="sm" />
        <strong>{run ? runKindLabel(run.kind, t) : t("report.title")}</strong>
        {run ? <span className="muted">{formatRunTime(run.startedAt, locale)} · {run.model}{run.effort ? ` · ${run.effort}` : ""}</span> : null}
        {run && run.terms.length > 0 ? <span className="muted report-head-terms" title={termsText(run.terms)}>{termsText(run.terms)}</span> : null}
        <span className="report-head-stop">{page?.running ? t("report.going") : page?.stop ? describeStop(page.stop.stop, t, locale) : page ? t("report.interrupted") : null}</span>
        {page?.continuable ? (
          <button className="button button-primary" type="button" disabled={continuing} title={t("report.continueTitle")} onClick={continueRun}>
            <UiIcon icon="sparkles" size="sm" />{t("report.continue")}
          </button>
        ) : null}
        {reports.length > 1 ? (
          <Select
            className="report-picker"
            variant="quiet"
            label={t("report.others")}
            value={reportId}
            options={reports.map((report) => ({ value: report.id, label: reportLabel(report.run.kind, report.run.startedAt, t, locale) }))}
            onChange={(id) => {
              const other = reports.find((report) => report.id === id);
              if (other) onOpenReport(other.id, reportLabel(other.run.kind, other.run.startedAt, t, locale));
            }}
          />
        ) : null}
      </div>
      {error ? <p className="glossary-file-problem"><UiIcon icon="circleAlert" size="xs" />{error}</p> : null}
      <div className="glossary-panes report-panes">
        <nav className="glossary-facets" aria-label={t("report.views")}>
          {hasStrings ? <><div className="glossary-facet-head">{t("report.strings")}</div><div className="glossary-facet-group">{STRINGS.map(facet)}</div></> : null}
          {page && !hasStrings ? <p className="muted report-empty">{t("report.empty")}</p> : null}
        </nav>
        <section className="glossary-list report-list" aria-label={t("report.title")}>
          <div className="glossary-toolbar">
            <span className="glossary-search">
              <UiIcon icon="search" size="xs" />
              <input className="input" type="search" value={query} placeholder={t("report.search")} aria-label={t("report.search")} spellCheck={false} onChange={(event) => { setQuery(event.target.value); setLimit(PAGE); }} />
            </span>
          </div>
          <div className="report-body">
            {page === null && !error ? <p className="muted report-empty">{t("common.loading")}</p> : null}
            {page && page.items.length === 0 ? <p className="muted report-empty">{t("report.noMatches")}</p> : null}
            {page && page.items.length > 0 ? (
              <ul className="report-items">
                {page.items.map((item, index) => <Item key={item.record === "string" ? item.context : `left:${index}`} item={item} onReveal={onRevealBinding} />)}
              </ul>
            ) : null}
            {page && page.total > page.items.length ? (
              <button className="button button-ghost report-more" type="button" onClick={() => setLimit((current) => current + PAGE)}>{t("report.more")}</button>
            ) : null}
          </div>
          <footer className="glossary-footer">
            <span className="muted">{page ? t("report.shown", { shown: formatNumber(page.items.length), total: formatNumber(page.total) }) : null}</span>
          </footer>
        </section>
      </div>
    </div>
  );
}
