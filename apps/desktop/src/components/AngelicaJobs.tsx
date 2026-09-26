import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { angelicaJobControl, angelicaJobEvents, angelicaJobRemove, angelicaJobRetry, angelicaJobSetConcurrency, angelicaJobSetLimit, angelicaJobUnits, angelicaJobWorkers, angelicaJobs, normalizeCommandError } from "../ipc";
import { atTokenLimit, formatElapsed, formatTokens, jobProblems, jobProgress, reasoningTitle, sortJobs, suggestedTokenLimit, totalTokens, workerHealth } from "../angelica";
import type { CommandError, JobAction, JobEvent, JobFilter, JobStatus, JobSummary, JobUnit, JobUnitStatus, SourceBinding, UnitLocationDto, WorkerActivity, WorkerPhase } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { Select } from "../ui/primitives/Select";
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

const phaseLabels: Readonly<Record<Exclude<WorkerPhase, "tool">, MessageKey>> = {
  idle: "angelica.worker.idle",
  preparing: "angelica.worker.preparing",
  waiting: "angelica.worker.waiting",
  reasoning: "angelica.worker.reasoning",
  writing: "angelica.worker.writing",
  recording: "angelica.worker.recording",
  backoff: "angelica.worker.backoff",
  stopped: "angelica.worker.stopped",
};

const toolLabels: Readonly<Partial<Record<string, MessageKey>>> = {
  submit_translations: "angelica.worker.tool.submit",
  validate_target: "angelica.worker.tool.validate",
  get_unit: "angelica.worker.tool.context",
  read_rows: "angelica.worker.tool.context",
  get_guidance: "angelica.worker.tool.guidance",
  report_issue: "angelica.worker.tool.report",
};

/** How often a running job's workers are polled while they are shown. */
const WORKER_POLL_MS = 1000;

type ProblemStatus = "rejected" | "failed" | "conflict";
const PROBLEMS: ProblemStatus[] = ["rejected", "failed", "conflict"];
/** Finished jobs listed below the ones that still run or wait. */
const FINISHED_SHOWN = 3;

type JobTab = "workers" | "problems" | "events" | "info";

const isLive = (job: JobSummary) => job.status === "running" || job.status === "paused";

function unitBinding(location: UnitLocationDto): SourceBinding {
  return { sheetName: location.sheet, rowId: location.row, subrowId: location.subrow, columnIndex: location.column ?? 0 };
}

function unitAddress(location: UnitLocationDto): string {
  return `${location.sheet}:${location.row}:${location.subrow}:${location.column ?? 0}`;
}

/** Reasoning summaries mark their headings as **bold** paragraphs. */
function thoughtParts(text: string): { heading: string | null; body: string } {
  const paragraphs = text.split(/\n{2,}/).map((part) => part.trim()).filter(Boolean);
  let heading: string | null = null;
  const body: string[] = [];
  for (const paragraph of paragraphs) {
    const match = /^\*\*(.+)\*\*$/s.exec(paragraph);
    if (match) {
      heading = match[1] ?? null;
      body.length = 0;
    } else {
      body.push(paragraph.replaceAll("**", ""));
    }
  }
  return { heading, body: body.join("\n\n") };
}

/** A worker's latest reasoning: its heading and the newest lines, or all of it. */
function WorkerThought({ text, expanded, headingShown }: { text: string; expanded: boolean; headingShown: boolean }) {
  if (expanded) return <span className="angelica-worker-thought-full">{text.replaceAll("**", "")}</span>;
  const { heading, body } = thoughtParts(text);
  return (
    <>
      {heading && !headingShown ? <strong className="angelica-worker-thought-heading">{heading}</strong> : null}
      {body ? <span className="angelica-worker-thought-tail"><span>{body}</span></span> : null}
    </>
  );
}

/** What a lane is doing right now, as one line. */
function workerActivity(worker: WorkerActivity, now: number, t: ReturnType<typeof useI18n>["t"]): string {
  switch (worker.phase) {
    case "reasoning": {
      const title = worker.thought ? reasoningTitle(worker.thought) : null;
      return title ? t("angelica.worker.thinkingAbout", { title }) : t("angelica.worker.reasoning");
    }
    case "writing":
      return worker.target?.unit != null
        ? t("angelica.worker.writingUnit", { current: worker.streamedUnits, total: worker.units })
        : t("angelica.worker.writingReply");
    case "tool": {
      if (worker.tool === "submit_translations" && worker.toolUnits > 0) return t("angelica.worker.tool.submitCount", { count: worker.toolUnits });
      const label = worker.tool ? toolLabels[worker.tool] : undefined;
      return label ? t(label) : t("angelica.worker.tool", { tool: worker.tool ?? "" });
    }
    case "backoff":
      return worker.retryAtUnixMs !== null ? t("angelica.worker.backoffUntil", { time: formatElapsed(worker.retryAtUnixMs - now) }) : t("angelica.worker.backoff");
    default:
      return t(phaseLabels[worker.phase]);
  }
}

function WorkerRow({ worker, now }: { worker: WorkerActivity; now: number }) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const health = workerHealth(worker, now);
  const timed = worker.phase !== "stopped" && worker.phase !== "backoff";
  const inChunk = worker.chunk !== null && worker.phase !== "idle" && worker.phase !== "stopped" && worker.phase !== "backoff";
  const thought = inChunk ? worker.thought?.trim() : undefined;
  const target = inChunk ? worker.target : null;
  const rows = worker.firstRow === null || worker.lastRow === null ? null
    : worker.firstRow === worker.lastRow ? String(worker.firstRow) : `${worker.firstRow}–${worker.lastRow}`;
  return (
    <li className={`angelica-worker ${health} ${worker.phase}`}>
      <div className="angelica-worker-line">
        <span className="angelica-worker-dot" aria-hidden="true" />
        <span className="angelica-worker-lane">{t("angelica.worker.lane", { lane: String(worker.lane) })}</span>
        <span className="angelica-worker-phase">{workerActivity(worker, now, t)}</span>
        {timed ? <span className="angelica-worker-time">{formatElapsed(now - worker.phaseStartedUnixMs)}</span> : null}
      </div>
      {target ? (
        <div className="angelica-worker-target" title={target.source ? `${target.address}\n${target.source}` : target.address}>
          {target.unit !== null ? <span className="angelica-worker-unit">{t("angelica.worker.unitNumber", { unit: String(target.unit) })}</span> : null}
          <span className="mono">{target.address}</span>
          {target.source ? <span className="angelica-worker-source">{`«${target.source}»`}</span> : null}
        </div>
      ) : null}
      {health === "active" ? null : <span className="angelica-worker-silence" title={t("angelica.worker.silentHint")}>{t("angelica.worker.silence", { time: formatElapsed(now - worker.lastActivityUnixMs) })}</span>}
      {thought ? (
        <button
          className={`angelica-worker-thought${expanded ? " expanded" : ""}${worker.phase === "reasoning" ? " live" : ""}`}
          type="button"
          aria-expanded={expanded}
          title={expanded ? undefined : t("angelica.worker.thoughtHint")}
          onClick={() => setExpanded((value) => !value)}
        >
          <WorkerThought text={thought} expanded={expanded} headingShown={worker.phase === "reasoning"} />
        </button>
      ) : null}
      <span className="angelica-worker-meta">
        {inChunk ? (
          <>
            <span title={t("angelica.worker.chunkHint", { chunk: String((worker.chunk ?? 0) + 1) })}>
              {rows ? t("angelica.worker.rows", { sheet: worker.sheet ?? "", rows }) : worker.sheet}
            </span>
            {worker.round > 0 ? <span title={t("angelica.worker.roundHint", { max: worker.maxRounds })}>{t("angelica.worker.round", { round: worker.round, max: worker.maxRounds })}</span> : null}
            <span title={t("angelica.worker.unitsHint")}>{t("angelica.worker.units", { finished: worker.finishedUnits, total: worker.units })}</span>
            {worker.chunkTokens > 0 ? <span>{t("angelica.worker.tokens", { tokens: formatTokens(worker.chunkTokens) })}</span> : null}
          </>
        ) : null}
        {worker.chunksDone > 0 ? <span title={t("angelica.worker.chunksDoneHint")}>{t("angelica.worker.chunksDone", { count: worker.chunksDone })}</span> : null}
        {worker.lastError ? <span className="angelica-job-problems" title={worker.lastError}>{t("angelica.worker.lastError")}</span> : null}
      </span>
    </li>
  );
}

/** Live activity of a job's workers; polled only while shown. */
function JobWorkers({ job, busy, setConcurrency }: { job: JobSummary; busy: boolean; setConcurrency: SetConcurrency }) {
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
  if (workers.length === 0) return <p className="angelica-job-empty">{t("angelica.job.noWorkers")}</p>;
  const working = workers.filter((worker) => worker.phase !== "stopped");
  const stopped = workers.length - working.length;
  return (
    <>
      <div className="angelica-job-toolbar">
        <p className="angelica-job-empty" title={t("angelica.job.workersHint")}>{t("angelica.worker.title", { active: working.length, total: workers.length })}</p>
        <label className="angelica-job-concurrency">{t("angelica.job.concurrency")}<ConcurrencyControl job={job} busy={busy} setConcurrency={setConcurrency} /></label>
      </div>
      <ul className="angelica-workers">
        {working.map((worker) => <WorkerRow key={worker.lane} worker={worker} now={now} />)}
      </ul>
      {stopped > 0 && working.length > 0 ? <p className="angelica-job-empty">{t("angelica.worker.stoppedCount", { count: stopped })}</p> : null}
    </>
  );
}

function JobProblems({ job, busy, retry, onError, onReveal }: { job: JobSummary; busy: boolean; retry: (statuses: JobUnitStatus[]) => void } & AngelicaJobsProps) {
  const { t, formatNumber } = useI18n();
  const [filter, setFilter] = useState<ProblemStatus | "all">("all");
  const [units, setUnits] = useState<JobUnit[] | null>(null);
  const problems = jobProblems(job.counts);
  const statuses = useMemo(() => filter === "all" ? PROBLEMS : [filter], [filter]);
  const total = filter === "all" ? problems : job.counts[filter];

  useEffect(() => {
    let current = true;
    void angelicaJobUnits(job.id, statuses)
      .then((next) => { if (current) setUnits(next); })
      .catch((reason: unknown) => onError(normalizeCommandError(reason)));
    return () => { current = false; };
    // Reload as the job finds more problems.
  }, [job.id, statuses, problems, onError]);

  if (problems === 0) return <p className="angelica-job-empty">{t("angelica.job.noProblems")}</p>;
  const options = [
    { value: "all" as const, label: `${t("angelica.job.problemFilter.all")} ${formatNumber(problems)}` },
    ...PROBLEMS.filter((status) => job.counts[status] > 0).map((status) => ({ value: status, label: `${t(unitLabels[status])} ${formatNumber(job.counts[status])}` })),
  ];
  return (
    <>
      <div className="angelica-job-toolbar">
        <Segmented value={filter} options={options} onChange={setFilter} label={t("angelica.job.tab.problems")} />
        {job.status !== "running" && job.status !== "cancelled" ? (
          <button className="button button-ghost" type="button" disabled={busy || total === 0} onClick={() => retry(statuses)}>
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
    // Reload as the job makes progress.
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

type SetLimit = (tokenLimit: number, resume: boolean) => void;
type SetConcurrency = (concurrency: number) => void;

/** Most workers a job runs at once; matches the Rust limit. */
const MAX_CONCURRENCY = 16;
const concurrencyOptions = Array.from({ length: MAX_CONCURRENCY }, (_, index) => ({ value: String(index + 1), label: String(index + 1) }));

/** How many workers a job runs; a running job follows within seconds. */
function ConcurrencyControl({ job, busy, setConcurrency }: { job: JobSummary; busy: boolean; setConcurrency: SetConcurrency }) {
  const { t } = useI18n();
  return (
    <Select
      value={String(job.spec.concurrency)}
      options={concurrencyOptions}
      onChange={(value) => setConcurrency(Number(value))}
      label={t("angelica.job.concurrency")}
      title={t("angelica.job.concurrencyHint")}
      disabled={busy || job.status === "cancelled"}
      variant="quiet"
    />
  );
}

/** Whether the job will likely use more than its limit. */
const overLimit = (job: JobSummary) => job.projectedTokens !== null && job.projectedTokens > job.spec.tokenLimit;

/** A new token limit for a job, prefilled with a suggestion. */
function LimitEditor({ job, busy, resume, setLimit, onDone }: { job: JobSummary; busy: boolean; resume: boolean; setLimit: SetLimit; onDone?: () => void }) {
  const { t, formatNumber } = useI18n();
  const [text, setText] = useState(() => formatNumber(suggestedTokenLimit(job)));
  const value = Number(text.replace(/\D/g, ""));
  const used = totalTokens(job.usage);
  const valid = /\d/.test(text) && Number.isSafeInteger(value) && value > used;
  const submit = () => {
    if (!valid) return;
    setLimit(value, resume);
    onDone?.();
  };
  return (
    <div className="angelica-job-limit-editor">
      <input
        className={valid ? "input" : "input invalid"}
        inputMode="numeric"
        value={text}
        disabled={busy}
        aria-label={t("angelica.job.limit")}
        title={valid ? undefined : t("angelica.job.limitTooLow", { used })}
        onChange={(event) => setText(event.target.value)}
        onBlur={() => { if (valid) setText(formatNumber(value)); }}
        onKeyDown={(event) => { if (event.key === "Enter") submit(); }}
      />
      <button className={resume ? "button button-secondary" : "button button-ghost"} type="button" disabled={busy || !valid} onClick={submit}>
        {resume ? <><UiIcon icon="play" size="sm" />{t("angelica.job.resumeWithLimit")}</> : t("angelica.job.saveLimit")}
      </button>
    </div>
  );
}

function JobInfo({ job, busy, setLimit, setConcurrency }: { job: JobSummary; busy: boolean; setLimit: SetLimit; setConcurrency: SetConcurrency }) {
  const { t, formatNumber } = useI18n();
  const sheets = job.spec.scope.sheets.length > 0 ? job.spec.scope.sheets.join(", ") : t("angelica.job.allSheets");
  const model = job.spec.model.effort ? `${job.spec.model.modelId} · ${job.spec.model.effort}` : job.spec.model.modelId;
  return (
    <dl className="angelica-job-facts">
      <dt>{t("angelica.job.sheets")}</dt><dd>{sheets}</dd>
      <dt>{t("angelica.job.strings")}</dt><dd>{t(jobFilterLabels[job.spec.scope.filter])}</dd>
      <dt>{t("angelica.job.model")}</dt><dd>{model}</dd>
      <dt>{t("angelica.job.concurrency")}</dt><dd><ConcurrencyControl job={job} busy={busy} setConcurrency={setConcurrency} /></dd>
      <dt>{t("angelica.job.tokenUse")}</dt><dd>{`${formatNumber(totalTokens(job.usage))} / ${formatNumber(job.spec.tokenLimit)}`}</dd>
      {job.projectedTokens !== null ? <><dt>{t("angelica.job.projectionLabel")}</dt><dd title={t("angelica.job.projectionHint", { finished: job.finishedChunks, total: job.chunks })}>{formatNumber(job.projectedTokens)}</dd></> : null}
      {job.status !== "cancelled" ? <><dt>{t("angelica.job.limit")}</dt><dd><LimitEditor key={job.spec.tokenLimit} job={job} busy={busy} resume={false} setLimit={setLimit} /></dd></> : null}
      {job.spec.instructions ? <><dt>{t("angelica.job.instructions")}</dt><dd>{job.spec.instructions}</dd></> : null}
    </dl>
  );
}

function JobDetails({ job, busy, retry, setLimit, setConcurrency, onError, onReveal }: { job: JobSummary; busy: boolean; retry: (statuses: JobUnitStatus[]) => void; setLimit: SetLimit; setConcurrency: SetConcurrency } & AngelicaJobsProps) {
  const { t, formatNumber } = useI18n();
  const problems = jobProblems(job.counts);
  const [tab, setTab] = useState<JobTab>(() => job.status === "running" ? "workers" : problems > 0 ? "problems" : "info");
  const running = job.status === "running";
  const shown: JobTab = tab === "workers" && !running ? (problems > 0 ? "problems" : "info") : tab;
  const options = [
    ...(running ? [{ value: "workers" as const, label: t("angelica.job.tab.workers") }] : []),
    { value: "problems" as const, label: problems > 0 ? `${t("angelica.job.tab.problems")} ${formatNumber(problems)}` : t("angelica.job.tab.problems") },
    { value: "events" as const, label: t("angelica.job.tab.events") },
    { value: "info" as const, label: t("angelica.job.tab.info") },
  ];
  return (
    <div className="angelica-job-details">
      <Segmented value={shown} options={options} onChange={setTab} label={t("angelica.job.details")} />
      <div className="angelica-job-pane">
        {shown === "workers" ? <JobWorkers job={job} busy={busy} setConcurrency={setConcurrency} /> : null}
        {shown === "problems" ? <JobProblems job={job} busy={busy} retry={retry} onError={onError} onReveal={onReveal} /> : null}
        {shown === "events" ? <JobEvents job={job} onError={onError} onReveal={onReveal} /> : null}
        {shown === "info" ? <JobInfo job={job} busy={busy} setLimit={setLimit} setConcurrency={setConcurrency} /> : null}
      </div>
    </div>
  );
}

type JobCardProps = {
  job: JobSummary;
  busy: boolean;
  expanded: boolean;
  onToggle: () => void;
  act: (action: JobAction) => void;
  retry: (statuses: JobUnitStatus[]) => void;
  setLimit: SetLimit;
  setConcurrency: SetConcurrency;
  remove: () => void;
} & AngelicaJobsProps;

function JobCard({ job, busy, expanded, onToggle, act, retry, setLimit, setConcurrency, remove, onError, onReveal }: JobCardProps) {
  const { t, locale } = useI18n();
  const [raising, setRaising] = useState(false);
  const limitPause = job.status === "paused" && atTokenLimit(job);
  const problems = jobProblems(job.counts);
  const tokens = totalTokens(job.usage);
  const sheets = job.spec.scope.sheets.length > 0 ? job.spec.scope.sheets.join(", ") : t("angelica.job.allSheets");
  const percent = new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 }).format(jobProgress(job.counts));
  const share = (count: number) => `${job.counts.total === 0 ? 0 : (count / job.counts.total) * 100}%`;
  return (
    <li className={`angelica-job ${job.status}${expanded ? " expanded" : ""}`}>
      <div className="angelica-job-head">
        <button className="angelica-job-toggle" type="button" aria-expanded={expanded} title={t(expanded ? "angelica.job.collapse" : "angelica.job.expand")} onClick={onToggle}>
          <UiIcon icon={expanded ? "chevronDown" : "chevronRight"} size="xs" />
          <span className="angelica-job-scope">{sheets}</span>
          <span className={`angelica-job-status ${job.status}`}>{t(statusLabels[job.status])}</span>
          <span className="angelica-job-percent">{percent}</span>
        </button>
        <div className="angelica-job-actions">
          {job.status === "running" ? <IconButton icon="pause" label={t("angelica.job.pause")} disabled={busy} onClick={() => act("pause")} /> : null}
          {job.status === "paused" ? <IconButton icon="play" label={t("angelica.job.resume")} disabled={busy} onClick={() => act("resume")} /> : null}
          {problems > 0 && job.status !== "running" && job.status !== "cancelled" ? <IconButton icon="refreshCw" label={t("angelica.job.retry")} disabled={busy} onClick={() => retry(PROBLEMS)} /> : null}
          {isLive(job) ? <IconButton icon="square" label={t("angelica.job.cancel")} disabled={busy} onClick={() => act("cancel")} /> : null}
          {isLive(job) ? null : <IconButton icon="x" label={t("angelica.job.remove")} disabled={busy} onClick={remove} />}
        </div>
      </div>
      <div className="angelica-job-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(jobProgress(job.counts) * 100)}>
        <span className="drafted" style={{ width: share(job.counts.drafted) }} />
        <span className="problems" style={{ width: share(problems) }} />
        <span className="running" style={{ width: share(job.counts.running) }} />
      </div>
      <div className="angelica-job-stats">
        <span>{t("angelica.job.drafted", { drafted: job.counts.drafted, total: job.counts.total })}</span>
        {problems > 0 ? <span className="angelica-job-problems">{t("angelica.job.problems", { count: problems })}</span> : null}
        {job.status === "running" ? <span title={t("angelica.job.workersHint")}>{t("angelica.job.workers", { active: job.activeWorkers, total: job.spec.concurrency })}</span> : null}
        <span title={t("angelica.job.tokenLimit", { limit: job.spec.tokenLimit })}>
          {t("angelica.job.limitFacts", { used: formatTokens(tokens), limit: formatTokens(job.spec.tokenLimit) })}
        </span>
        {job.projectedTokens !== null && job.status !== "completed" ? (
          <span className={overLimit(job) ? "angelica-job-over" : undefined} title={t("angelica.job.projectionHint", { finished: job.finishedChunks, total: job.chunks })}>
            {t("angelica.job.projection", { tokens: formatTokens(job.projectedTokens) })}
          </span>
        ) : null}
      </div>
      {job.status === "paused" && job.reason && !limitPause ? (
        <p className="angelica-job-reason" title={job.reason}><UiIcon icon="circleAlert" size="xs" /><span>{job.reason}</span></p>
      ) : null}
      {limitPause ? (
        <div className="angelica-job-limit">
          <p className="angelica-job-reason"><UiIcon icon="circleAlert" size="xs" /><span>{t("angelica.job.limitReached")}</span></p>
          <LimitEditor key={job.spec.tokenLimit} job={job} busy={busy} resume setLimit={setLimit} />
        </div>
      ) : job.status === "running" && overLimit(job) ? (
        <div className="angelica-job-limit">
          <p className="angelica-job-reason">
            <UiIcon icon="triangleAlert" size="xs" /><span>{t("angelica.job.overLimit")}</span>
            {raising ? null : <button className="link-button" type="button" onClick={() => setRaising(true)}>{t("angelica.job.raiseLimit")}</button>}
          </p>
          {raising ? <LimitEditor job={job} busy={busy} resume={false} setLimit={setLimit} onDone={() => setRaising(false)} /> : null}
        </div>
      ) : null}
      {expanded ? <JobDetails job={job} busy={busy} retry={retry} setLimit={setLimit} setConcurrency={setConcurrency} onError={onError} onReveal={onReveal} /> : null}
    </li>
  );
}

/** The project's translation jobs, with their progress and controls. */
export function AngelicaJobs({ onError, onReveal }: AngelicaJobsProps) {
  const { t } = useI18n();
  const [jobs, setJobs] = useState<JobSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(true);
  // Cards the user opened or closed; others follow their job's status.
  const [toggled, setToggled] = useState<Readonly<Record<string, boolean>>>({});
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

  const sorted = sortJobs(jobs);
  const active = sorted.filter(isLive);
  const finished = sorted.filter((job) => !isLive(job));
  const shown = [...active, ...finished.slice(0, FINISHED_SHOWN)];
  if (shown.length === 0) return null;
  // Only a running job opens by itself, and only the first one.
  const firstRunning = active.find((job) => job.status === "running")?.id;
  const isExpanded = (job: JobSummary) => toggled[job.id] ?? job.id === firstRunning;

  return (
    <section className={`angelica-jobs${open ? " open" : ""}`}>
      <div className="angelica-jobs-head">
        <button className="angelica-jobs-toggle" type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
          <UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" />
          <span>{t("angelica.jobs", { count: active.length })}</span>
        </button>
        {finished.length > 0 ? (
          <button className="button button-ghost" type="button" disabled={busy} onClick={() => void removeJobs(finished.map((job) => job.id))}>
            {t("angelica.jobs.clearFinished", { count: finished.length })}
          </button>
        ) : null}
      </div>
      {open ? (
        <ul className="angelica-job-list">
          {shown.map((job) => (
            <JobCard
              key={job.id}
              job={job}
              busy={busy}
              expanded={isExpanded(job)}
              onToggle={() => setToggled((current) => ({ ...current, [job.id]: !isExpanded(job) }))}
              act={(action) => void run(() => angelicaJobControl(job.id, action))}
              retry={(statuses) => void run(() => angelicaJobRetry(job.id, statuses))}
              setLimit={(tokenLimit, resume) => void run(() => angelicaJobSetLimit(job.id, tokenLimit, resume))}
              setConcurrency={(concurrency) => void run(() => angelicaJobSetConcurrency(job.id, concurrency))}
              remove={() => void removeJobs([job.id])}
              onError={onError}
              onReveal={onReveal}
            />
          ))}
        </ul>
      ) : null}
    </section>
  );
}
