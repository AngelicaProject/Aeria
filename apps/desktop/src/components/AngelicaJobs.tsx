import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { angelicaJobControl, angelicaJobEvents, angelicaJobRemove, angelicaJobRetry, angelicaJobUnits, angelicaJobWorkers, angelicaJobs, localizationChooseName, localizationDecisions, localizationOverview, localizationProject, normalizeCommandError } from "../ipc";
import { formatElapsed, formatTokens, jobProblems, jobProgress, jobWritten, sortJobs, totalTokens, workerHealth, scopeText } from "../angelica";
import type { CommandError, JobAction, JobEvent, JobFilter, JobStatus, JobSummary, JobUnit, JobUnitStatus, KnowledgeDomain, LocalizationDecision, LocalizationOverview, LocalizationProjectArea, SourceBinding, UnitLocationDto, WorkerActivity, WorkerPhase, WorkerStep } from "../types";
import type { MessageKey } from "../i18n/translate";
import { askAngelica } from "../angelicaAsk";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";

type AngelicaJobsProps = {
  onError: (error: CommandError) => void;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
  /** Asks Angelica something in the open conversation. */
  onAsk?: ((text: string) => void) | undefined;
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
  revise: "angelica.job.filter.revise",
};

const unitLabels: Readonly<Record<JobUnitStatus, MessageKey>> = {
  pending: "angelica.job.unit.pending",
  running: "angelica.job.unit.running",
  drafted: "angelica.job.unit.drafted",
  finished: "angelica.job.unit.finished",
  flagged: "angelica.job.unit.flagged",
  rejected: "angelica.job.unit.rejected",
  failed: "angelica.job.unit.failed",
  conflict: "angelica.job.unit.conflict",
};

const phaseLabels: Readonly<Record<WorkerPhase, MessageKey>> = {
  idle: "angelica.worker.idle",
  preparing: "angelica.worker.preparing",
  waiting: "angelica.worker.waiting",
  reasoning: "angelica.worker.reasoning",
  writing: "angelica.worker.writing",
  recording: "angelica.worker.recording",
  backoff: "angelica.worker.backoff",
  stopped: "angelica.worker.stopped",
};

const stepLabels: Readonly<Record<WorkerStep, MessageKey>> = {
  study: "angelica.worker.step.study",
  learning: "angelica.worker.step.learning",
  terms: "angelica.worker.step.terms",
  contract: "angelica.worker.step.contract",
  writing: "angelica.worker.step.writing",
  voicing: "angelica.worker.step.voicing",
  reviewing: "angelica.worker.step.reviewing",
  fixing: "angelica.worker.step.fixing",
  rechecking: "angelica.worker.step.rechecking",
};

/** What each kind of text is called in the panel. */
export const domainLabels: Readonly<Record<KnowledgeDomain, MessageKey>> = {
  general: "localization.domain.general",
  journal: "localization.domain.journal",
  objective: "localization.domain.objective",
  system: "localization.domain.system",
  dialogue: "localization.domain.dialogue",
  names: "localization.domain.names",
  items: "localization.domain.items",
  actions: "localization.domain.actions",
  interface: "localization.domain.interface",
  lore: "localization.domain.lore",
};

/** How often the diagnostics poll a running localization's lanes. */
const WORKER_POLL_MS = 1000;
/** How often a running localization's areas and speed are read again. */
const OVERVIEW_POLL_MS = 10_000;

type ProblemStatus = "rejected" | "failed" | "conflict" | "flagged";
/** Outcomes that can be retried. */
const PROBLEMS: ProblemStatus[] = ["rejected", "failed", "conflict"];
/** Outcomes listed on the Problems tab: retryable ones and strings left for review. */
const LISTED: ProblemStatus[] = [...PROBLEMS, "flagged"];

type DetailTab = "problems" | "events" | "info" | "diagnostics";

const isLive = (job: JobSummary) => job.status === "running" || job.status === "paused";

function unitBinding(location: UnitLocationDto): SourceBinding {
  return { sheetName: location.sheet, rowId: location.row, subrowId: location.subrow, columnIndex: location.column ?? 0 };
}

function unitAddress(location: UnitLocationDto): string {
  return `${location.sheet}:${location.row}:${location.subrow}:${location.column ?? 0}`;
}

/** The share of the job's prompt tokens its provider served from cache. */
const cachedShare = (job: JobSummary): string => {
  const prompt = job.usage.promptTokens;
  const cached = job.usage.cachedPromptTokens ?? 0;
  return prompt > 0 ? `${Math.round((cached / prompt) * 100)} %` : "—";
};

/** What a lane is doing right now, as one line. */
function workerActivity(worker: WorkerActivity, now: number, t: ReturnType<typeof useI18n>["t"], studying: boolean): string {
  switch (worker.phase) {
    case "idle":
      // Every chunk waits for the study of the job's scope.
      return t(studying ? "angelica.worker.waitingStudy" : "angelica.worker.idle");
    case "waiting":
      return worker.requests > 0 ? t("angelica.worker.waitingRequests", { count: worker.requests }) : t("angelica.worker.waiting");
    case "backoff":
      return worker.retryAtUnixMs !== null ? t("angelica.worker.backoffUntil", { time: formatElapsed(worker.retryAtUnixMs - now) }) : t("angelica.worker.backoff");
    default:
      return t(phaseLabels[worker.phase]);
  }
}

function WorkerRow({ worker, now, studying }: { worker: WorkerActivity; now: number; studying: boolean }) {
  const { t } = useI18n();
  const health = workerHealth(worker, now);
  const timed = worker.phase !== "stopped" && worker.phase !== "backoff";
  const working = worker.phase !== "idle" && worker.phase !== "stopped" && worker.phase !== "backoff";
  const inChunk = worker.chunk !== null && working;
  const rows = worker.firstRow === null || worker.lastRow === null ? null
    : worker.firstRow === worker.lastRow ? String(worker.firstRow) : `${worker.firstRow}–${worker.lastRow}`;
  return (
    <li className={`angelica-worker ${health} ${worker.phase}`}>
      <div className="angelica-worker-line">
        <span className="angelica-worker-dot" aria-hidden="true" />
        <span className="angelica-worker-lane">{t("angelica.worker.lane", { lane: String(worker.lane) })}</span>
        <span className="angelica-worker-phase">{workerActivity(worker, now, t, studying)}</span>
        {timed ? <span className="angelica-worker-time">{formatElapsed(now - worker.phaseStartedUnixMs)}</span> : null}
      </div>
      {health === "active" ? null : <span className="angelica-worker-silence" title={t("angelica.worker.silentHint")}>{t("angelica.worker.silence", { time: formatElapsed(now - worker.lastActivityUnixMs) })}</span>}
      <span className="angelica-worker-meta">
        {inChunk ? (
          <span title={t("angelica.worker.chunkHint", { chunk: String((worker.chunk ?? 0) + 1) })}>
            {rows ? t("angelica.worker.rows", { sheet: worker.sheet ?? "", rows }) : worker.sheet}
          </span>
        ) : null}
        {working && worker.step ? <span title={t("angelica.worker.stepHint")}>{worker.round > 0 ? t("angelica.worker.step", { round: worker.round, max: worker.maxRounds, step: t(stepLabels[worker.step]) }) : t(stepLabels[worker.step])}</span> : null}
        {inChunk ? <span title={t("angelica.worker.unitsHint")}>{t("angelica.worker.units", { finished: worker.finishedUnits, total: worker.units })}</span> : null}
        {working && worker.chunkTokens > 0 ? <span>{t("angelica.worker.tokens", { tokens: formatTokens(worker.chunkTokens) })}</span> : null}
        {worker.lastError ? <span className="angelica-job-problems" title={worker.lastError}>{t("angelica.worker.lastError")}</span> : null}
      </span>
    </li>
  );
}

/** The lanes of a running localization, for diagnosis; polled only while shown. */
function Diagnostics({ job }: { job: JobSummary }) {
  const { t } = useI18n();
  const [workers, setWorkers] = useState<WorkerActivity[] | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    let current = true;
    // Polling also ticks the elapsed times; a failed poll keeps the last view.
    const poll = () => {
      void angelicaJobWorkers(job.id)
        .then((next) => { if (current) { setWorkers(next); setNow(Date.now()); } })
        .catch(() => undefined);
    };
    poll();
    const timer = window.setInterval(poll, WORKER_POLL_MS);
    return () => { current = false; window.clearInterval(timer); };
  }, [job.id]);

  if (workers === null) return null;
  const working = workers.filter((worker) => worker.phase !== "stopped" && worker.phase !== "idle");
  if (working.length === 0) return <p className="angelica-job-empty">{t("angelica.job.noWorkers")}</p>;
  const studying = workers.some((worker) => worker.step === "study" && worker.phase !== "idle" && worker.phase !== "stopped");
  return (
    <>
      <p className="angelica-job-empty" title={t("localization.diagnosticsHint")}>{t("localization.scenesAtOnce", { count: working.length })}</p>
      <ul className="angelica-workers">
        {working.map((worker) => <WorkerRow key={worker.lane} worker={worker} now={now} studying={studying} />)}
      </ul>
    </>
  );
}

function JobProblems({ job, busy, retry, only, onError, onReveal }: { job: JobSummary; busy: boolean; retry: (statuses: JobUnitStatus[]) => void; only?: ProblemStatus } & AngelicaJobsProps) {
  const { t, formatNumber } = useI18n();
  const [filter, setFilter] = useState<ProblemStatus | "all">(only ?? "all");
  const [units, setUnits] = useState<JobUnit[] | null>(null);
  const problems = jobProblems(job.counts) + job.counts.flagged;
  const statuses = useMemo(() => filter === "all" ? LISTED : [filter], [filter]);
  const total = filter === "all" ? problems : job.counts[filter];
  const retryable = filter === "all" ? jobProblems(job.counts) : filter === "flagged" ? 0 : job.counts[filter];

  useEffect(() => {
    let current = true;
    void angelicaJobUnits(job.id, statuses)
      .then((next) => { if (current) setUnits(next); })
      .catch((reason: unknown) => onError(normalizeCommandError(reason)));
    return () => { current = false; };
    // Reload as the localization finds more problems.
  }, [job.id, statuses, problems, onError]);

  if (problems === 0) return <p className="angelica-job-empty">{t("angelica.job.noProblems")}</p>;
  const options = [
    { value: "all" as const, label: `${t("angelica.job.problemFilter.all")} ${formatNumber(problems)}` },
    ...LISTED.filter((status) => job.counts[status] > 0).map((status) => ({ value: status, label: `${t(unitLabels[status])} ${formatNumber(job.counts[status])}` })),
  ];
  return (
    <>
      <div className="angelica-job-toolbar">
        <Segmented value={filter} options={options} onChange={setFilter} label={t("angelica.job.tab.problems")} />
        {job.status !== "running" && job.status !== "cancelled" ? (
          <button className="button button-ghost" type="button" disabled={busy || retryable === 0} onClick={() => retry(statuses.filter((status) => status !== "flagged"))}>
            <UiIcon icon="refreshCw" size="sm" />{t("angelica.job.retryThese")}
          </button>
        ) : null}
      </div>
      {units ? (
        <ul className="angelica-job-rows">
          {units.map((unit) => (
            <li key={unit.seq}>
              <button className="link-button mono" type="button" title={t("angelica.job.reveal")} onClick={() => onReveal?.(unitBinding(unit.location))}>{unitAddress(unit.location)}</button>
              {filter === "all" ? <span className={`angelica-chip ${unit.status}`}>{t(unitLabels[unit.status])}</span> : null}
              {unit.message ? <span className="angelica-job-message" title={unit.message}>{unit.message}</span> : null}
            </li>
          ))}
        </ul>
      ) : null}
      {units && units.length < total ? <p className="angelica-job-empty">{t("angelica.job.shownOf", { shown: units.length, total })}</p> : null}
    </>
  );
}

function JobEvents({ job, onError, onReveal }: { job: JobSummary } & AngelicaJobsProps) {
  const { t, locale } = useI18n();
  const [events, setEvents] = useState<JobEvent[] | null>(null);
  const processed = jobProgress(job.counts);
  const time = useMemo(() => new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit" }), [locale]);

  useEffect(() => {
    let current = true;
    void angelicaJobEvents(job.id)
      .then((next) => { if (current) setEvents([...next].reverse()); })
      .catch((reason: unknown) => onError(normalizeCommandError(reason)));
    return () => { current = false; };
    // Reload as the localization makes progress.
  }, [job.id, processed, job.status, onError]);

  if (events === null) return null;
  if (events.length === 0) return <p className="angelica-job-empty">{t("angelica.job.noEvents")}</p>;
  return (
    <ul className="angelica-job-rows">
      {events.map((event) => (
        <li key={event.seq}>
          <span className="angelica-job-time">{time.format(event.createdAtUnixMs)}</span>
          <span className="angelica-chip">{event.kind}</span>
          {event.location ? <button className="link-button mono" type="button" title={t("angelica.job.reveal")} onClick={() => event.location && onReveal?.(unitBinding(event.location))}>{unitAddress(event.location)}</button> : null}
          <span className="angelica-job-message" title={event.message}>{event.message}</span>
        </li>
      ))}
    </ul>
  );
}

function JobInfo({ job }: { job: JobSummary }) {
  const { t, formatNumber } = useI18n();
  const sheets = scopeText(job.spec.scope) ?? t("localization.wholeProject");
  const model = job.spec.model.effort ? `${job.spec.model.modelId} · ${job.spec.model.effort}` : job.spec.model.modelId;
  const careful = job.spec.scope.careful ?? [];
  return (
    <dl className="angelica-job-facts">
      <dt>{t("angelica.job.sheets")}</dt><dd>{sheets}</dd>
      <dt>{t("angelica.job.strings")}</dt><dd>{t(jobFilterLabels[job.spec.scope.filter])}</dd>
      <dt>{t("angelica.job.model")}</dt><dd>{model}</dd>
      <dt>{t("angelica.job.quality")}</dt><dd title={t("angelica.job.qualityHint")}>{t(`angelica.job.quality.${job.spec.quality ?? "fast"}` as const)}</dd>
      {careful.length > 0 ? <><dt>{t("localization.carefulAreas")}</dt><dd>{careful.join(", ")}</dd></> : null}
      <dt>{t("angelica.job.tokenUse")}</dt><dd>{formatNumber(totalTokens(job.usage))}</dd>
      <dt title={t("angelica.job.cachedHint")}>{t("angelica.job.cached")}</dt><dd>{cachedShare(job)}</dd>
      {job.spec.instructions ? <><dt>{t("angelica.job.instructions")}</dt><dd>{job.spec.instructions}</dd></> : null}
    </dl>
  );
}

function JobDetails({ job, busy, retry, tab, setTab, onError, onReveal }: { job: JobSummary; busy: boolean; retry: (statuses: JobUnitStatus[]) => void; tab: DetailTab; setTab: (tab: DetailTab) => void } & AngelicaJobsProps) {
  const { t, formatNumber } = useI18n();
  const problems = jobProblems(job.counts) + job.counts.flagged;
  const running = job.status === "running";
  const shown: DetailTab = tab === "diagnostics" && !running ? "problems" : tab;
  const options = [
    { value: "problems" as const, label: problems > 0 ? `${t("angelica.job.tab.problems")} ${formatNumber(problems)}` : t("angelica.job.tab.problems") },
    { value: "events" as const, label: t("angelica.job.tab.events") },
    { value: "info" as const, label: t("angelica.job.tab.info") },
    ...(running ? [{ value: "diagnostics" as const, label: t("localization.diagnostics") }] : []),
  ];
  return (
    <div className="angelica-job-details">
      <Segmented value={shown} options={options} onChange={setTab} label={t("angelica.job.details")} />
      <div className="angelica-job-pane">
        {shown === "problems" ? <JobProblems job={job} busy={busy} retry={retry} onError={onError} onReveal={onReveal} /> : null}
        {shown === "events" ? <JobEvents job={job} onError={onError} onReveal={onReveal} /> : null}
        {shown === "info" ? <JobInfo job={job} /> : null}
        {shown === "diagnostics" ? <Diagnostics job={job} /> : null}
      </div>
    </div>
  );
}

/** A duration in minutes as hours and minutes. */
function formatMinutes(minutes: number, t: ReturnType<typeof useI18n>["t"]): string {
  const rounded = Math.max(1, Math.round(minutes));
  const hours = Math.floor(rounded / 60);
  return hours > 0 ? t("localization.hoursMinutes", { hours, minutes: rounded % 60 }) : t("localization.minutes", { minutes: rounded });
}

/** How far each kind of text of the whole project is localized. */
function ProjectState({ areas }: { areas: LocalizationProjectArea[] }) {
  const { t, formatNumber } = useI18n();
  if (areas.length === 0) return null;
  return (
    <ul className="localization-areas localization-project" aria-label={t("localization.project")}>
      {areas.map((area) => {
        const share = (count: number) => `${area.total === 0 ? 0 : (count / area.total) * 100}%`;
        const plain = Math.max(0, area.translated - area.reviewed - area.needsReview);
        return (
          <li key={area.domain ?? "other"} className="localization-area" title={t("localization.projectHint", { reviewed: formatNumber(area.reviewed), review: formatNumber(area.needsReview), drafts: formatNumber(plain) })}>
            <span className="localization-area-name">{area.domain ? t(domainLabels[area.domain]) : "—"}</span>
            <span className="angelica-job-bar" aria-hidden="true">
              <span className="drafted" style={{ width: share(area.reviewed) }} />
              <span className="running" style={{ width: share(plain) }} />
              <span className="flagged" style={{ width: share(area.needsReview) }} />
            </span>
            <span className="localization-area-count">{`${formatNumber(area.translated)} / ${formatNumber(area.total)}`}</span>
          </li>
        );
      })}
    </ul>
  );
}

/** Cost and pace: tokens per string, cache share, strings per minute, time left. */
function Economy({ job, overview }: { job: JobSummary; overview: LocalizationOverview | null }) {
  const { t, formatNumber } = useI18n();
  const written = jobWritten(job.counts);
  const remaining = job.counts.pending + job.counts.running;
  const perMinute = overview?.perMinute ?? 0;
  return (
    <div className="localization-economy">
      {written > 0 ? <span title={t("localization.perStringHint")}>{t("localization.perString", { tokens: formatNumber(Math.round(totalTokens(job.usage) / written)) })}</span> : null}
      {job.usage.promptTokens > 0 ? <span title={t("angelica.job.cachedHint")}>{t("angelica.job.cachedFacts", { share: cachedShare(job) })}</span> : null}
      {job.status === "running" && perMinute > 0 ? <span>{t("localization.speed", { count: formatNumber(Math.round(perMinute)) })}</span> : null}
      {job.status === "running" && perMinute > 0 && remaining > 0 ? <span>{t("localization.left", { time: formatMinutes(remaining / perMinute, t) })}</span> : null}
    </div>
  );
}

/** The localization's kinds of text, in the order it takes them. */
function Areas({ overview }: { overview: LocalizationOverview | null }) {
  const { t, formatNumber } = useI18n();
  if (!overview || overview.areas.length < 2) return null;
  return (
    <ul className="localization-areas">
      {overview.areas.map((area) => {
        const share = (count: number) => `${area.total === 0 ? 0 : (count / area.total) * 100}%`;
        return (
          <li key={area.domain ?? "other"} className="localization-area">
            <span className="localization-area-name">{area.domain ? t(domainLabels[area.domain]) : "—"}</span>
            <span className="angelica-job-bar" aria-hidden="true">
              <span className="drafted" style={{ width: share(area.done) }} />
              <span className="flagged" style={{ width: share(area.flagged) }} />
              <span className="problems" style={{ width: share(area.problems) }} />
            </span>
            <span className="localization-area-count">{`${formatNumber(area.done + area.flagged)} / ${formatNumber(area.total)}`}</span>
          </li>
        );
      })}
    </ul>
  );
}

type CardProps = {
  job: JobSummary;
  busy: boolean;
  act: (action: JobAction) => void;
  retry: (statuses: JobUnitStatus[]) => void;
  remove: () => void;
  focus: DetailTab | null;
  setFocus: (tab: DetailTab | null) => void;
} & AngelicaJobsProps;

function LocalizationCard({ job, busy, act, retry, remove, focus, setFocus, onError, onReveal }: CardProps) {
  const { t, locale } = useI18n();
  const [overview, setOverview] = useState<LocalizationOverview | null>(null);
  const problems = jobProblems(job.counts);
  const scope = scopeText(job.spec.scope) ?? t("localization.wholeProject");
  const percent = new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 }).format(jobProgress(job.counts));
  const share = (count: number) => `${job.counts.total === 0 ? 0 : (count / job.counts.total) * 100}%`;
  const processed = jobProgress(job.counts);

  useEffect(() => {
    let current = true;
    const load = () => {
      void localizationOverview(job.id).then((next) => { if (current) setOverview(next); }).catch(() => undefined);
    };
    load();
    const timer = job.status === "running" ? window.setInterval(load, OVERVIEW_POLL_MS) : null;
    return () => { current = false; if (timer !== null) window.clearInterval(timer); };
    // Progress changes the areas; the timer keeps the speed current.
  }, [job.id, job.status, processed]);

  const expanded = focus !== null;
  return (
    <li className={`angelica-job localization ${job.status}${expanded ? " expanded" : ""}`}>
      <div className="angelica-job-head">
        <button className="angelica-job-toggle" type="button" aria-expanded={expanded} title={t(expanded ? "angelica.job.collapse" : "angelica.job.expand")} onClick={() => setFocus(expanded ? null : "problems")}>
          <UiIcon icon={expanded ? "chevronDown" : "chevronRight"} size="xs" />
          <span className="angelica-job-scope" title={scope}>{scope}</span>
          <span className="angelica-job-percent">{percent}</span>
          {job.counts.flagged > 0 ? <span className="angelica-job-flagged" title={t("angelica.job.flaggedHint")}>{t("localization.flaggedShort", { count: job.counts.flagged })}</span> : null}
          {job.status !== "running" ? <span className={`angelica-job-status ${job.status}`}>{t(statusLabels[job.status])}</span> : null}
        </button>
        <div className="angelica-job-actions">
          {job.status === "running" ? <IconButton icon="pause" label={t("angelica.job.pause")} disabled={busy} onClick={() => act("pause")} /> : null}
          {job.status === "paused" ? <IconButton icon="play" label={t("angelica.job.resume")} disabled={busy} onClick={() => act("resume")} /> : null}
          {problems > 0 && job.status !== "running" && job.status !== "cancelled" ? <IconButton icon="refreshCw" label={t("angelica.job.retry")} disabled={busy} onClick={() => retry(PROBLEMS)} /> : null}
          {isLive(job) ? <IconButton icon="square" label={t("angelica.job.cancel")} disabled={busy} onClick={() => act("cancel")} /> : null}
          {isLive(job) ? null : <IconButton icon="x" label={t("angelica.job.remove")} disabled={busy} onClick={remove} />}
        </div>
      </div>
      <div className="angelica-job-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(processed * 100)}>
        <span className="drafted" style={{ width: share(job.counts.finished + job.counts.drafted) }} />
        <span className="flagged" style={{ width: share(job.counts.flagged) }} />
        <span className="problems" style={{ width: share(problems) }} />
        <span className="running" style={{ width: share(job.counts.running) }} />
      </div>
      {job.status === "paused" && job.reason ? (
        <p className="angelica-job-reason" title={job.reason}><UiIcon icon="circleAlert" size="xs" /><span>{job.reason}</span></p>
      ) : null}
      {expanded ? (
        <>
          <div className="angelica-job-stats">
            <span>{t("angelica.job.written", { written: jobWritten(job.counts), total: job.counts.total })}</span>
            {problems > 0 ? <span className="angelica-job-problems">{t("angelica.job.problems", { count: problems })}</span> : null}
          </div>
          <Economy job={job} overview={overview} />
          <Areas overview={overview} />
          <JobDetails job={job} busy={busy} retry={retry} tab={focus} setTab={setFocus} onError={onError} onReveal={onReveal} />
        </>
      ) : null}
    </li>
  );
}

/** A made-up name as a table row: the chosen rendering, the options, and a field for another one. */
function NameRow({ decision, busy, choose }: { decision: Extract<LocalizationDecision, { kind: "name" }>; busy: boolean; choose: (rendering: string) => void }) {
  const { t } = useI18n();
  const [other, setOther] = useState("");
  return (
    <li className="localization-name">
      <span className="localization-name-term" title={decision.term}>{decision.term}</span>
      <span className="localization-decision-options">
        {decision.options.map((option) => (
          <button key={option} className={option === decision.rendering ? "button button-secondary" : "button button-ghost"} type="button" disabled={busy} title={t("localization.decision.choose")} onClick={() => choose(option)}>{option}</button>
        ))}
        <input className="input" value={other} disabled={busy} placeholder={t("localization.decision.nameOther")} aria-label={t("localization.decision.nameOther")} onChange={(event) => setOther(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && other.trim()) choose(other.trim()); }} />
      </span>
    </li>
  );
}

type DecisionGroup = "style" | "names" | "review" | "knowledge";

const groupOf = (decision: LocalizationDecision): DecisionGroup =>
  decision.kind === "calibrate" ? "style" : decision.kind === "name" ? "names" : decision.kind;

const GROUPS: readonly DecisionGroup[] = ["style", "names", "review", "knowledge"];

const groupLabels: Readonly<Record<DecisionGroup, MessageKey>> = {
  style: "localization.group.style",
  names: "localization.group.names",
  review: "localization.group.review",
  knowledge: "localization.group.knowledge",
};

/** Decisions that wait for a person, by kind: one kind open at a time. */
function Decisions({ decisions, busy, onAsk, choose, acceptAll, review }: { decisions: LocalizationDecision[]; busy: boolean; onAsk?: ((text: string) => void) | undefined; choose: (term: string, rendering: string) => void; acceptAll: (names: Array<{ term: string; rendering: string }>) => void; review: (jobId: string) => void }) {
  const { t, formatNumber } = useI18n();
  const [group, setGroup] = useState<DecisionGroup | null>(null);
  if (decisions.length === 0) return null;
  const counts = new Map<DecisionGroup, number>();
  for (const decision of decisions) counts.set(groupOf(decision), (counts.get(groupOf(decision)) ?? 0) + (decision.kind === "review" ? decision.count : 1));
  const present = GROUPS.filter((kind) => counts.has(kind));
  const open = group !== null && counts.has(group) ? group : null;
  const shown = open ? decisions.filter((decision) => groupOf(decision) === open) : [];
  const names = shown.filter((decision): decision is Extract<LocalizationDecision, { kind: "name" }> => decision.kind === "name");
  return (
    <div className="localization-decisions">
      <div className="localization-decision-summary">
        <span className="localization-decision-title">{t("localization.waiting")}</span>
        {present.map((kind) => (
          <button key={kind} className={kind === open ? "button button-secondary" : "button button-ghost"} type="button" aria-pressed={kind === open} onClick={() => setGroup(kind === open ? null : kind)}>
            {`${t(groupLabels[kind])} ${formatNumber(counts.get(kind) ?? 0)}`}
          </button>
        ))}
        {onAsk ? <button className="link-button" type="button" onClick={() => onAsk(t("localization.ask.triage"))}>{t("localization.triage")}</button> : null}
      </div>
      {open === "style" ? (
        <ul className="localization-decision-list">
          {shown.map((decision) => decision.kind === "calibrate" ? (
            <li key={decision.domain} className="localization-decision">
              <span className="localization-decision-text">{t(domainLabels[decision.domain])}</span>
              {onAsk ? <button className="button button-ghost" type="button" onClick={() => onAsk(t("localization.ask.calibrate", { domain: t(domainLabels[decision.domain]) }))}>{t("localization.decision.calibrateAction")}</button> : null}
            </li>
          ) : null)}
        </ul>
      ) : null}
      {open === "names" ? (
        <>
          <div className="localization-decision">
            <span className="localization-decision-text field-hint">{t("localization.names.hint")}</span>
            <button className="button button-secondary" type="button" disabled={busy} onClick={() => acceptAll(names.map((name) => ({ term: name.term, rendering: name.rendering })))}>{t("localization.names.acceptAll")}</button>
          </div>
          <ul className="localization-decision-list localization-names">
            {names.map((decision) => <NameRow key={decision.term} decision={decision} busy={busy} choose={(rendering) => choose(decision.term, rendering)} />)}
          </ul>
        </>
      ) : null}
      {open === "review" ? (
        <ul className="localization-decision-list">
          {shown.map((decision) => decision.kind === "review" ? (
            <li key={decision.jobId} className="localization-decision">
              <span className="localization-decision-text">{t("localization.decision.review", { count: decision.count })}</span>
              <button className="button button-ghost" type="button" onClick={() => review(decision.jobId)}>{t("localization.decision.reviewAction")}</button>
            </li>
          ) : null)}
        </ul>
      ) : null}
      {open === "knowledge" ? (
        <ul className="localization-decision-list">
          {shown.map((decision) => decision.kind === "knowledge" ? (
            <li key={`${decision.jobId}-${decision.message}`} className="localization-decision">
              <span className="localization-decision-text" title={decision.message}>{decision.message}</span>
              {onAsk ? <button className="button button-ghost" type="button" onClick={() => onAsk(t("localization.ask.knowledge", { message: decision.message }))}>{t("localization.decision.discuss")}</button> : null}
            </li>
          ) : null)}
        </ul>
      ) : null}
    </div>
  );
}

/** The project's localization as a workbench view: the project by area,
 * the decisions that wait, and the work in progress. */
export function LocalizationView({ onError, onReveal }: Omit<AngelicaJobsProps, "onAsk">) {
  return (
    <div className="localization-view">
      <AngelicaJobs onError={onError} onReveal={onReveal} onAsk={askAngelica} />
    </div>
  );
}

/** The project's localizations: how they go, and what waits for a person. */
export function AngelicaJobs({ onError, onReveal, onAsk }: AngelicaJobsProps) {
  const { t } = useI18n();
  const [jobs, setJobs] = useState<JobSummary[]>([]);
  const [decisions, setDecisions] = useState<LocalizationDecision[]>([]);
  const [project, setProject] = useState<LocalizationProjectArea[]>([]);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(true);
  const [showPast, setShowPast] = useState(false);
  // The details a card shows, by job; closed when absent.
  const [focus, setFocusState] = useState<Readonly<Record<string, DetailTab>>>({});
  const reloadTimer = useRef<number | null>(null);

  const load = useCallback(() => {
    void angelicaJobs().then(setJobs).catch((reason: unknown) => onError(normalizeCommandError(reason)));
    void localizationDecisions().then(setDecisions).catch(() => undefined);
    void localizationProject().then(setProject).catch(() => undefined);
  }, [onError]);

  useEffect(() => {
    load();
    const subscription = listen<{ jobId: string }>("angelica://job", () => {
      // Lanes report often; one reload per short burst is enough.
      if (reloadTimer.current !== null) return;
      reloadTimer.current = window.setTimeout(() => { reloadTimer.current = null; load(); }, 1000);
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

  const removeJobs = async (ids: string[]) => {
    setBusy(true);
    try {
      for (const id of ids) {
        await angelicaJobRemove(id);
        setJobs((current) => current.filter((job) => job.id !== id));
      }
    } catch (reason) {
      onError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  };

  // Accepting the study's renderings changes nothing already written.
  const acceptAll = async (names: Array<{ term: string; rendering: string }>) => {
    setBusy(true);
    try {
      for (const name of names) await localizationChooseName(name.term, name.rendering);
      const settled = new Set(names.map((name) => name.term));
      setDecisions((current) => current.filter((decision) => !(decision.kind === "name" && settled.has(decision.term))));
    } catch (reason) {
      onError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  };

  const choose = async (term: string, rendering: string) => {
    setBusy(true);
    try {
      const changed = await localizationChooseName(term, rendering);
      setDecisions((current) => current.filter((decision) => !(decision.kind === "name" && decision.term === term)));
      // Strings written with the old rendering are revised through Angelica.
      if (changed) onAsk?.(t("localization.ask.revise", { term, rendering }));
    } catch (reason) {
      onError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  };

  const setFocus = (jobId: string, tab: DetailTab | null) => setFocusState((current) => {
    const next = { ...current };
    if (tab === null) delete next[jobId];
    else next[jobId] = tab;
    return next;
  });

  const sorted = sortJobs(jobs);
  const active = sorted.filter(isLive);
  const past = sorted.filter((job) => !isLive(job));
  if (active.length === 0 && past.length === 0 && decisions.length === 0 && project.length === 0) return null;

  const card = (job: JobSummary) => (
    <LocalizationCard
      key={job.id}
      job={job}
      busy={busy}
      act={(action) => void run(() => angelicaJobControl(job.id, action))}
      retry={(statuses) => void run(() => angelicaJobRetry(job.id, statuses))}
      remove={() => void removeJobs([job.id])}
      focus={focus[job.id] ?? null}
      setFocus={(tab) => setFocus(job.id, tab)}
      onError={onError}
      onReveal={onReveal}
    />
  );

  return (
    <section className={`angelica-jobs${open ? " open" : ""}`}>
      <div className="angelica-jobs-head">
        <button className="angelica-jobs-toggle" type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
          <UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" />
          <span>{t("localization.title")}</span>
        </button>
      </div>
      {open ? (
        <div className="angelica-job-list">
          <ProjectState areas={project} />
          <Decisions decisions={decisions} busy={busy} onAsk={onAsk} choose={(term, rendering) => void choose(term, rendering)} acceptAll={(names) => void acceptAll(names)} review={(jobId) => setFocus(jobId, "problems")} />
          {active.length > 0 ? (
            <>
              <p className="localization-now">{t("localization.now", { count: active.length, left: active.reduce((sum, job) => sum + job.counts.pending + job.counts.running, 0) })}</p>
              <ul className="localization-cards">{active.map(card)}</ul>
            </>
          ) : null}
          {past.length > 0 ? (
            <div className="localization-past">
              <div className="angelica-jobs-head">
                <button className="angelica-jobs-toggle" type="button" aria-expanded={showPast} onClick={() => setShowPast((value) => !value)}>
                  <UiIcon icon={showPast ? "chevronDown" : "chevronRight"} size="xs" />
                  <span>{t("localization.past", { count: past.length })}</span>
                </button>
                <button className="button button-ghost" type="button" disabled={busy} onClick={() => void removeJobs(past.map((job) => job.id))}>
                  {t("angelica.jobs.clearFinished", { count: past.length })}
                </button>
              </div>
              {showPast ? <ul className="localization-cards">{past.map(card)}</ul> : null}
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
