import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { angelicaJobControl, angelicaJobEvents, angelicaJobRetry, angelicaJobUnits, angelicaJobs, normalizeCommandError } from "../ipc";
import { jobProblems, jobProgress, sortJobs, totalTokens } from "../angelica";
import type { CommandError, JobAction, JobEvent, JobFilter, JobStatus, JobSummary, JobUnit, JobUnitStatus, SourceBinding } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type AngelicaJobsProps = {
  onError: (error: CommandError) => void;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
};

const statusLabels: Readonly<Record<JobStatus, MessageKey>> = {
  running: "angelica.job.running",
  paused: "angelica.job.paused",
  completed: "angelica.job.completed",
  cancelled: "angelica.job.cancelled",
};

export const jobFilterLabels: Readonly<Record<JobFilter, MessageKey>> = {
  untranslated: "angelica.job.filter.untranslated",
  needsReview: "angelica.job.filter.needsReview",
  untranslatedAndDrafts: "angelica.job.filter.untranslatedAndDrafts",
};

const unitLabels: Readonly<Record<JobUnitStatus, MessageKey>> = {
  pending: "angelica.job.unit.pending",
  running: "angelica.job.unit.running",
  drafted: "angelica.job.unit.drafted",
  rejected: "angelica.job.unit.rejected",
  failed: "angelica.job.unit.failed",
  conflict: "angelica.job.unit.conflict",
};

const PROBLEMS: JobUnitStatus[] = ["rejected", "failed", "conflict"];
/** Finished jobs listed below the ones that still run or wait. */
const FINISHED_SHOWN = 3;

function JobDetails({ job, onError, onReveal }: { job: JobSummary } & AngelicaJobsProps) {
  const { t } = useI18n();
  const [units, setUnits] = useState<JobUnit[] | null>(null);
  const [events, setEvents] = useState<JobEvent[]>([]);
  const processed = job.counts.drafted + job.counts.rejected + job.counts.failed + job.counts.conflict;

  useEffect(() => {
    let current = true;
    void Promise.all([angelicaJobUnits(job.id, PROBLEMS), angelicaJobEvents(job.id)])
      .then(([nextUnits, nextEvents]) => { if (current) { setUnits(nextUnits); setEvents(nextEvents.slice(-20)); } })
      .catch((reason: unknown) => onError(normalizeCommandError(reason)));
    return () => { current = false; };
    // Reload as the job makes progress.
  }, [job.id, processed, job.status, onError]);

  return (
    <div className="angelica-job-details">
      {job.spec.instructions ? <p className="angelica-job-instructions">{job.spec.instructions}</p> : null}
      {units && units.length > 0 ? (
        <ul className="angelica-job-units">
          {units.map((unit) => {
            const location = unit.location;
            const binding: SourceBinding = { sheetName: location.sheet, rowId: location.row, subrowId: location.subrow, columnIndex: location.column ?? 0 };
            return (
              <li key={unit.seq}>
                <button className="link-button mono" type="button" onClick={() => onReveal?.(binding)}>{`${location.sheet}:${location.row}:${location.subrow}:${location.column ?? 0}`}</button>
                <span className="angelica-chip">{t(unitLabels[unit.status])}</span>
                {unit.message ? <span className="angelica-job-message">{unit.message}</span> : null}
              </li>
            );
          })}
        </ul>
      ) : null}
      {events.length > 0 ? (
        <ul className="angelica-job-events">
          {events.map((event) => <li key={event.seq}><span className="angelica-chip">{event.kind}</span>{event.message}</li>)}
        </ul>
      ) : null}
    </div>
  );
}

function JobCard({ job, busy, act, retry, onError, onReveal }: { job: JobSummary; busy: boolean; act: (action: JobAction) => void; retry: () => void } & AngelicaJobsProps) {
  const { t } = useI18n();
  const progress = jobProgress(job.counts);
  const problems = jobProblems(job.counts);
  const tokens = totalTokens(job.usage);
  const sheets = job.spec.scope.sheets.length > 0 ? job.spec.scope.sheets.join(", ") : t("angelica.job.allSheets");
  return (
    <li className={`angelica-job ${job.status}`}>
      <div className="angelica-job-head">
        <span className="angelica-job-scope" title={sheets}>{sheets}</span>
        <span className="angelica-chip">{t(statusLabels[job.status])}</span>
      </div>
      <div className="angelica-job-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
        <span style={{ width: `${progress * 100}%` }} />
      </div>
      <div className="angelica-job-stats">
        <span>{t("angelica.job.drafted", { drafted: job.counts.drafted, total: job.counts.total })}</span>
        {job.status === "running" ? <span title={t("angelica.job.workersHint")}>{t("angelica.job.workers", { active: job.activeWorkers, total: job.spec.concurrency })}</span> : null}
        {problems > 0 ? <span className="angelica-job-problems">{t("angelica.job.problems", { count: problems })}</span> : null}
        <span title={t("angelica.job.tokenLimit", { limit: job.spec.tokenLimit })}>{t("angelica.tokens", { count: tokens })}</span>
        <span>{t(jobFilterLabels[job.spec.scope.filter])}</span>
      </div>
      {job.status === "paused" && job.reason ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{job.reason}</p> : null}
      <div className="angelica-job-actions">
        {job.status === "running" ? <button className="button button-ghost" type="button" disabled={busy} onClick={() => act("pause")}><UiIcon icon="pause" size="sm" />{t("angelica.job.pause")}</button> : null}
        {job.status === "paused" ? <button className="button button-secondary" type="button" disabled={busy} onClick={() => act("resume")}><UiIcon icon="play" size="sm" />{t("angelica.job.resume")}</button> : null}
        {problems > 0 && job.status !== "running" && job.status !== "cancelled" ? <button className="button button-ghost" type="button" disabled={busy} onClick={retry}><UiIcon icon="refreshCw" size="sm" />{t("angelica.job.retry")}</button> : null}
        {job.status === "running" || job.status === "paused" ? <button className="button button-ghost" type="button" disabled={busy} onClick={() => act("cancel")}><UiIcon icon="x" size="sm" />{t("angelica.job.cancel")}</button> : null}
      </div>
      <details className="angelica-job-more">
        <summary>{t("angelica.job.details")}</summary>
        <JobDetails job={job} onError={onError} onReveal={onReveal} />
      </details>
    </li>
  );
}

/** The project's translation jobs, with their progress and controls. */
export function AngelicaJobs({ onError, onReveal }: AngelicaJobsProps) {
  const { t } = useI18n();
  const [jobs, setJobs] = useState<JobSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const reloadTimer = useRef<number | null>(null);

  const load = useCallback(() => {
    void angelicaJobs().then(setJobs).catch((reason: unknown) => onError(normalizeCommandError(reason)));
  }, [onError]);

  useEffect(() => {
    load();
    const subscription = listen<{ jobId: string }>("angelica://job", () => {
      // Lanes report often; one reload per short burst is enough.
      if (reloadTimer.current !== null) return;
      reloadTimer.current = window.setTimeout(() => { reloadTimer.current = null; load(); }, 500);
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
      if (reloadTimer.current !== null) window.clearTimeout(reloadTimer.current);
    };
  }, [load]);

  const run = async (operation: () => Promise<JobSummary>) => {
    setBusy(true);
    try {
      const next = await operation();
      setJobs((current) => current.map((job) => job.id === next.id ? next : job));
    } catch (reason) {
      onError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  };

  const sorted = sortJobs(jobs);
  const active = sorted.filter((job) => job.status === "running" || job.status === "paused");
  const shown = [...active, ...sorted.filter((job) => job.status !== "running" && job.status !== "paused").slice(0, FINISHED_SHOWN)];
  if (shown.length === 0) return null;

  return (
    <details className="angelica-jobs" open={active.length > 0}>
      <summary>{t("angelica.jobs", { count: active.length })}</summary>
      <ul>
        {shown.map((job) => (
          <JobCard
            key={job.id}
            job={job}
            busy={busy}
            act={(action) => void run(() => angelicaJobControl(job.id, action))}
            retry={() => void run(() => angelicaJobRetry(job.id, PROBLEMS))}
            onError={onError}
            onReveal={onReveal}
          />
        ))}
      </ul>
    </details>
  );
}
