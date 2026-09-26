import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { aiSettings, angelicaApplyProposal, angelicaProposals, angelicaRejectProposal, angelicaCancel, angelicaConversation, angelicaConversations, angelicaDeleteConversation, angelicaSend, normalizeCommandError } from "../ipc";
import { ANGELICA, activitySummary, applyAgentEvent, contextFill, formatElapsed, formatTokens, groupTranscript, isQuietWait, parseReply, reasoningTitle, resolveModel, toolSubject, totalTokens, transcriptFromMessages, workingPhase, type ActivityStep, type ReplySpan, type TranscriptBlock, type TranscriptItem, type WorkingPhase } from "../angelica";
import type { AgentMode, ProposalRecord, SourceBinding } from "../types";
import { ModeMenu, ModelMenu, SelectionToggle } from "./AngelicaComposerControls";
import { AngelicaJobs } from "./AngelicaJobs";
import { AngelicaProposals } from "./AngelicaProposals";
import type { AgentEvent, AiModelSelection, AiSettingsDto, AiUsage, AngelicaEventDto, CommandError, ConversationDto, ConversationSummaryDto, EditorContextDto, ReasoningEffort } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ErrorBanner } from "./ErrorBanner";

type AngelicaPanelProps = {
  editorContext: EditorContextDto | null;
  onOpenSettings?: (() => void) | undefined;
  onOpenGuide?: ((tab: "glossary" | "guidance") => void) | undefined;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
};

const toolLabels: Readonly<Record<string, MessageKey>> = {
  project_overview: "angelica.tool.projectOverview",
  list_sheets: "angelica.tool.listSheets",
  read_rows: "angelica.tool.readRows",
  get_unit: "angelica.tool.getUnit",
  propose_translation: "angelica.tool.proposeTranslation",
  get_guidance: "angelica.tool.getGuidance",
  propose_glossary_change: "angelica.tool.proposeGlossaryChange",
  propose_guidance_change: "angelica.tool.proposeGuidanceChange",
  validate_target: "angelica.tool.validateTarget",
  pending_changes: "angelica.tool.pendingChanges",
  unit_history: "angelica.tool.unitHistory",
  navigate_to: "angelica.tool.navigateTo",
  fetch_url: "angelica.tool.fetchUrl",
  propose_review: "angelica.tool.proposeReview",
  search_source: "angelica.tool.searchSource",
  search_translations: "angelica.tool.searchTranslations",
  similar_translations: "angelica.tool.similarTranslations",
  estimate_job: "angelica.tool.estimateJob",
  start_job: "angelica.tool.startJob",
  job_status: "angelica.tool.jobStatus",
  job_events: "angelica.tool.jobEvents",
  amend_job: "angelica.tool.amendJob",
  retry_units: "angelica.tool.retryUnits",
  pause_job: "angelica.tool.pauseJob",
  resume_job: "angelica.tool.resumeJob",
  raise_job_limit: "angelica.tool.raiseJobLimit",
  set_job_workers: "angelica.tool.setJobWorkers",
  cancel_job: "angelica.tool.cancelJob",
};

const suggestionKeys: readonly MessageKey[] = ["angelica.suggestion.overview", "angelica.suggestion.selection", "angelica.suggestion.macros"];
const EMPTY_USAGE: AiUsage = { promptTokens: 0, completionTokens: 0 };
/** Longest tool text shown in a card; the model received the full result. */
const TOOL_PREVIEW_CHARS = 4000;

function Spans({ spans }: { spans: ReplySpan[] }) {
  return <>{spans.map((span, index) => span.kind === "code" ? <code key={index}>{span.text}</code> : span.kind === "strong" ? <strong key={index}>{span.text}</strong> : <span key={index}>{span.text}</span>)}</>;
}

function Reply({ text }: { text: string }) {
  return (
    <div className="angelica-reply">
      {parseReply(text).map((block, index) => block.kind === "code"
        ? <pre key={index}><code>{block.text}</code></pre>
        : block.kind === "list"
          ? <ul key={index}>{block.items.map((item, itemIndex) => <li key={itemIndex}><Spans spans={item} /></li>)}</ul>
          : <p key={index}><Spans spans={block.spans} /></p>)}
    </div>
  );
}

function preview(text: string): string {
  let pretty = text;
  try { pretty = JSON.stringify(JSON.parse(text), null, 2); } catch { /* keep the raw text */ }
  return pretty.length > TOOL_PREVIEW_CHARS ? `${pretty.slice(0, TOOL_PREVIEW_CHARS)}\n…` : pretty;
}

function ToolStep({ item }: { item: Extract<TranscriptItem, { kind: "tool" }> }) {
  const { t } = useI18n();
  const label = toolLabels[item.name];
  const subject = toolSubject(item.name, item.arguments);
  const state = item.result === null ? "running" : item.isError ? "failed" : "done";
  return (
    <details className={`angelica-step angelica-step-${state}`}>
      <summary>
        <UiIcon icon={state === "running" ? "circleDot" : state === "failed" ? "circleAlert" : "check"} size="xs" />
        <span className="angelica-step-name">{label ? t(label) : item.name}</span>
        {subject ? <code className="angelica-step-subject">{subject}</code> : null}
      </summary>
      <div className="angelica-step-body">
        <span className="field-label">{t("angelica.toolArguments")}</span>
        <pre>{preview(item.arguments)}</pre>
        {item.result !== null ? <><span className="field-label">{t("angelica.toolResult")}</span><pre>{preview(item.result)}</pre></> : null}
      </div>
    </details>
  );
}

/** What an activity is doing now, for its folded line and the status line. */
function currentStep(steps: readonly ActivityStep[], t: ReturnType<typeof useI18n>["t"]): string {
  const last = steps.at(-1);
  if (!last) return t("angelica.thinking");
  if (last.kind === "reasoning") return reasoningTitle(last.text) ?? t("angelica.thinking");
  const label = toolLabels[last.item.name];
  const subject = toolSubject(last.item.name, last.item.arguments);
  return `${label ? t(label) : last.item.name}${subject ? ` ${subject}` : ""}`;
}

/** Reasoning and tool calls between two replies, folded into one line. */
function ActivityBlock({ block }: { block: Extract<TranscriptBlock, { kind: "activity" }> }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const summary = activitySummary(block.steps);
  const parts: string[] = [];
  if (summary.tools > 0) parts.push(t("angelica.activity.tools", { count: summary.tools }));
  if (summary.failed > 0) parts.push(t("angelica.activity.failed", { count: summary.failed }));
  if (summary.reasoning) parts.push(t("angelica.activity.reasoning"));
  return (
    <div className={block.live ? "angelica-activity live" : "angelica-activity"}>
      <button className="angelica-activity-head" type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        <span className="angelica-activity-text">{block.live ? currentStep(block.steps, t) : parts.join(" · ")}</span>
        <UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" />
      </button>
      {open ? (
        <div className="angelica-activity-body">
          {block.steps.map((entry) => entry.kind === "reasoning"
            ? <div key={entry.key} className="angelica-thought"><Reply text={entry.text} /></div>
            : <ToolStep key={entry.item.key} item={entry.item} />)}
        </div>
      ) : null}
    </div>
  );
}

function TranscriptEntry({ block }: { block: TranscriptBlock }) {
  if (block.kind === "user") return <div className="angelica-user">{block.text}</div>;
  if (block.kind === "notice") return <div className="angelica-notice"><UiIcon icon="info" size="xs" />{block.text.replace(/^\[Aeria\]\s*/, "")}</div>;
  if (block.kind === "activity") return <ActivityBlock block={block} />;
  return <div className="angelica-assistant"><Reply text={block.text} /></div>;
}

const STATUS_COUNT = 16;

const phaseLabels: Readonly<Record<WorkingPhase, MessageKey>> = {
  thinking: "angelica.phase.thinking",
  tools: "angelica.phase.tools",
  writing: "angelica.phase.writing",
};

/**
 * What Angelica is doing, the elapsed time, and the tokens of the turn. The
 * status names the real phase; only when the model has been silent for a
 * while does a playful status fill the wait, until the next event arrives.
 */
function WorkingLine({ startedAt, tokens, phase, lastActivityAt }: { startedAt: number; tokens: number; phase: WorkingPhase; lastActivityAt: number }) {
  const { t } = useI18n();
  const [now, setNow] = useState(() => Date.now());
  const [playful, setPlayful] = useState(() => Math.floor(Math.random() * STATUS_COUNT));
  const quiet = isQuietWait(now, lastActivityAt);
  useEffect(() => {
    const clock = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(clock);
  }, []);
  // A new quiet stretch gets a new playful status.
  useEffect(() => {
    if (quiet) setPlayful((current) => (current + 1 + Math.floor(Math.random() * (STATUS_COUNT - 1))) % STATUS_COUNT);
  }, [quiet]);
  const parts = [formatElapsed(now - startedAt)];
  if (tokens > 0) parts.push(t("angelica.statusTokens", { count: formatTokens(tokens) }));
  return (
    <div className={quiet ? "angelica-working quiet" : "angelica-working"} role="status">
      <UiIcon icon="sparkles" size="xs" />
      <span className="angelica-working-status">{t(quiet ? `angelica.status.${playful + 1}` as MessageKey : phaseLabels[phase])}…</span>
      <span className="angelica-working-meta">{parts.join(" · ")}</span>
    </div>
  );
}

/** Angelica's chat: conversations, live turns, and model and effort choice. */
export function AngelicaPanel({ editorContext, onOpenSettings, onOpenGuide, onReveal }: AngelicaPanelProps) {
  const { t } = useI18n();
  const [settings, setSettings] = useState<AiSettingsDto | null>(null);
  const [conversations, setConversations] = useState<ConversationSummaryDto[]>([]);
  const [conversation, setConversation] = useState<ConversationDto | null>(null);
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [running, setRunning] = useState(false);
  const [turnStartedAt, setTurnStartedAt] = useState(() => Date.now());
  const [turnTokens, setTurnTokens] = useState(0);
  const [lastActivityAt, setLastActivityAt] = useState(() => Date.now());
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [usage, setUsage] = useState<AiUsage>(EMPTY_USAGE);
  const [lastPromptTokens, setLastPromptTokens] = useState<number | null>(null);
  const [model, setModel] = useState<AiModelSelection | null>(null);
  const [draft, setDraft] = useState("");
  const [queue, setQueue] = useState<string[]>([]);
  const [attachContext, setAttachContext] = useState(true);
  const [mode, setMode] = useState<AgentMode>("ask");
  const [proposals, setProposals] = useState<ProposalRecord[]>([]);
  const [settling, setSettling] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<MessageKey | null>(null);
  const conversationIdRef = useRef<string | null>(null);
  const runningRef = useRef(false);
  runningRef.current = running;
  const awaitingIdRef = useRef(false);
  const bufferedRef = useRef(new Map<string, AgentEvent[]>());
  const transcriptRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  const loadSettings = useCallback(() => {
    void aiSettings().then(setSettings).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, []);
  const loadConversations = useCallback(() => {
    void angelicaConversations().then(setConversations).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, []);

  const loadProposals = useCallback((id: string | null) => {
    if (!id) { setProposals([]); return; }
    void angelicaProposals(id)
      .then((next) => { if (conversationIdRef.current === id) setProposals(next); })
      .catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, []);

  useEffect(() => { loadSettings(); loadConversations(); }, [loadConversations, loadSettings]);

  useEffect(() => {
    const subscription = listen<{ conversationId: string }>("angelica://proposals", ({ payload }) => {
      if (payload.conversationId === conversationIdRef.current) loadProposals(payload.conversationId);
    });
    return () => { void subscription.then((unlisten) => unlisten()); };
  }, [loadProposals]);

  const settle = async (ids: string[], apply: boolean) => {
    const id = conversationIdRef.current;
    if (!id) return;
    setSettling(true);
    try {
      let next = proposals;
      for (const proposalId of ids) {
        next = await (apply ? angelicaApplyProposal(id, proposalId) : angelicaRejectProposal(id, proposalId));
      }
      if (conversationIdRef.current === id) setProposals(next);
    } catch (reason) {
      setError(normalizeCommandError(reason));
      loadProposals(id);
    } finally {
      setSettling(false);
    }
  };

  useEffect(() => {
    if (settings) setModel((current) => resolveModel(settings.providers, [current, conversation?.model, settings.agentModel]));
  }, [conversation?.model, settings]);

  const showConversation = useCallback((next: ConversationDto | null) => {
    conversationIdRef.current = next?.id ?? null;
    setConversation(next);
    setItems(next ? transcriptFromMessages(next.messages) : []);
    setUsage(next?.usage ?? EMPTY_USAGE);
    setRunning(next?.running ?? false);
    setLastPromptTokens(null);
    setNotice(null);
    setModel(null);
    stickToBottom.current = true;
    loadProposals(next?.id ?? null);
  }, [loadProposals]);

  const reloadConversation = useCallback(async (id: string) => {
    try {
      const next = await angelicaConversation(id);
      if (conversationIdRef.current === id) {
        setConversation(next);
        setItems(transcriptFromMessages(next.messages));
        setUsage(next.usage);
        setRunning(next.running);
      }
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }, []);

  const handleEvent = useCallback((event: AgentEvent, id: string) => {
    setLastActivityAt(Date.now());
    switch (event.type) {
      case "usage":
        setLastPromptTokens(event.promptTokens);
        setTurnTokens((current) => current + event.completionTokens);
        return;
      case "turnFinished":
        setRunning(false);
        setUsage(event.usage);
        if (event.outcome === "roundLimit") setNotice("angelica.roundLimit");
        void reloadConversation(id);
        loadConversations();
        return;
      case "turnFailed":
        setRunning(false);
        setError({ code: event.code, message: event.message });
        void reloadConversation(id);
        loadConversations();
        return;
      case "turnCancelled":
        setRunning(false);
        setQueue([]);
        void reloadConversation(id);
        loadConversations();
        return;
      default:
        if (!runningRef.current) {
          // A turn Aeria started, such as a job report: show its message.
          runningRef.current = true;
          setRunning(true);
          void reloadConversation(id);
          loadConversations();
          return;
        }
        setItems((current) => applyAgentEvent(current, event));
    }
  }, [loadConversations, reloadConversation]);

  useEffect(() => {
    const subscription = listen<AngelicaEventDto>("angelica://event", ({ payload }) => {
      if (payload.conversationId === conversationIdRef.current) {
        handleEvent(payload.event, payload.conversationId);
      } else if (awaitingIdRef.current) {
        // A new conversation's events can arrive before its ID is known.
        const buffered = bufferedRef.current.get(payload.conversationId) ?? [];
        buffered.push(payload.event);
        bufferedRef.current.set(payload.conversationId, buffered);
      } else if (payload.event.type.startsWith("turn")) {
        loadConversations();
      }
    });
    return () => { void subscription.then((unlisten) => unlisten()); };
  }, [handleEvent, loadConversations]);

  useEffect(() => {
    if (!running) return;
    setTurnStartedAt(Date.now());
    setLastActivityAt(Date.now());
    setTurnTokens(0);
  }, [running]);

  // The message box grows with its text up to a limit.
  useLayoutEffect(() => {
    const element = inputRef.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 200)}px`;
  }, [draft]);

  useLayoutEffect(() => {
    const element = transcriptRef.current;
    if (element && stickToBottom.current) element.scrollTop = element.scrollHeight;
  }, [items]);

  const send = useCallback(async (text: string) => {
    if (!model) return;
    setError(null);
    setNotice(null);
    setRunning(true);
    runningRef.current = true;
    stickToBottom.current = true;
    const context = attachContext ? editorContext : null;
    const isNew = conversationIdRef.current === null;
    awaitingIdRef.current = isNew;
    setItems((current) => [...current, { kind: "user", key: `pending${current.length}`, text }]);
    try {
      const next = await angelicaSend(conversationIdRef.current, text, model, context, mode);
      conversationIdRef.current = next.id;
      setConversation(next);
      const buffered = bufferedRef.current.get(next.id) ?? [];
      bufferedRef.current.clear();
      awaitingIdRef.current = false;
      setItems(transcriptFromMessages(next.messages));
      for (const event of buffered) handleEvent(event, next.id);
      if (isNew) loadConversations();
    } catch (reason) {
      awaitingIdRef.current = false;
      setRunning(false);
      setItems((current) => current.filter((item) => !item.key.startsWith("pending")));
      setDraft((current) => current || text);
      setError(normalizeCommandError(reason));
    }
  }, [attachContext, editorContext, handleEvent, loadConversations, mode, model]);

  // Messages written while Angelica answers are sent after the turn ends.
  useEffect(() => {
    if (running || queue.length === 0) return;
    const [next, ...rest] = queue;
    setQueue(rest);
    void send(next!);
  }, [queue, running, send]);

  const submit = () => {
    const text = draft.trim();
    if (!text || !model) return;
    setDraft("");
    if (running) setQueue((current) => [...current, text]);
    else void send(text);
  };

  const onComposerKey = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      submit();
    }
  };

  const stop = () => {
    if (conversationIdRef.current) void angelicaCancel(conversationIdRef.current).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  };

  const newConversation = () => {
    if (running) return;
    showConversation(null);
    setQueue([]);
  };

  const openConversation = (id: string) => {
    if (!id) { newConversation(); return; }
    void angelicaConversation(id).then(showConversation).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  };

  const deleteConversation = () => {
    const id = conversationIdRef.current;
    if (!id) return;
    void angelicaDeleteConversation(id).then(() => { showConversation(null); loadConversations(); }).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  };

  const providers = settings?.providers ?? [];
  const hasModels = providers.some((provider) => provider.models.length > 0);
  const modelConfig = model ? providers.find((provider) => provider.id === model.providerId)?.models.find((entry) => entry.id === model.modelId) ?? null : null;
  const fill = contextFill(lastPromptTokens, modelConfig?.contextWindow ?? null);
  const selection = editorContext?.selection ?? null;
  const blocks = groupTranscript(items, running);
  // The live activity line already names the current step.
  const phase = workingPhase(items);
  const contextPercent = fill === null ? null : Math.round(fill * 100);
  const usageTitle = [
    contextPercent !== null ? t("angelica.contextFill", { percent: contextPercent }) : null,
    t("angelica.usageHint", { prompt: usage.promptTokens, completion: usage.completionTokens }),
  ].filter(Boolean).join("\n");

  if (settings && !hasModels) {
    return (
      <section className="tool-content angelica" aria-label={ANGELICA}>
        <div className="empty-state">
          <UiIcon icon="sparkles" size="xl" />
          <strong>{t("angelica.notConfigured")}</strong>
          <p>{t("angelica.notConfiguredHint")}</p>
          {onOpenSettings ? <button className="button button-secondary" type="button" onClick={onOpenSettings}>{t("angelica.openSettings")}</button> : null}
          <button className="button button-ghost" type="button" onClick={loadSettings}><UiIcon icon="refreshCw" size="sm" />{t("angelica.reloadSettings")}</button>
        </div>
      </section>
    );
  }

  return (
    <section className="tool-content angelica" aria-label={ANGELICA}>
      <header className="angelica-head">
        <Select
          className="angelica-history"
          variant="quiet"
          value={conversation?.id ?? ""}
          disabled={running}
          label={t("angelica.history")}
          onChange={openConversation}
          options={[{ value: "", label: t("angelica.newConversation") }, ...conversations.map((entry) => ({ value: entry.id, label: entry.title || t("angelica.untitled") }))]}
        />
        {onOpenGuide ? <button className="icon-button icon-button-ghost" type="button" aria-label={t("angelica.openGuide")} title={t("angelica.openGuide")} onClick={() => onOpenGuide("glossary")}><UiIcon icon="languages" size="sm" /></button> : null}
        <button className="icon-button icon-button-ghost" type="button" disabled={running || conversation === null} aria-label={t("angelica.newConversation")} title={t("angelica.newConversation")} onClick={newConversation}><UiIcon icon="plus" size="sm" /></button>
        <button className="icon-button icon-button-ghost" type="button" disabled={running || conversation === null} aria-label={t("angelica.deleteConversation")} title={t("angelica.deleteConversation")} onClick={deleteConversation}><UiIcon icon="trash" size="sm" /></button>
      </header>

      <div className="angelica-transcript" ref={transcriptRef} onScroll={(event) => {
        const element = event.currentTarget;
        stickToBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
      }}>
        {items.length === 0 ? (
          <div className="angelica-welcome">
            <UiIcon icon="sparkles" size="xl" />
            <strong>{ANGELICA}</strong>
            <p>{t("angelica.welcome")}</p>
            <div className="angelica-suggestions">
              {suggestionKeys.map((key) => <button key={key} className="button button-ghost" type="button" disabled={!model} onClick={() => void send(t(key))}>{t(key)}</button>)}
            </div>
          </div>
        ) : blocks.map((block) => <TranscriptEntry key={block.key} block={block} />)}
        {running ? <WorkingLine startedAt={turnStartedAt} tokens={turnTokens} phase={phase} lastActivityAt={lastActivityAt} /> : null}
        {notice ? <p className="field-hint">{t(notice)}</p> : null}
      </div>

      <AngelicaJobs onError={setError} onReveal={onReveal} />

      <AngelicaProposals proposals={proposals} busy={settling} onApply={(ids) => void settle(ids, true)} onReject={(ids) => void settle(ids, false)} onReveal={onReveal} />

      {error ? <div className="angelica-error"><ErrorBanner title={t("angelica.error")} error={error} onDismiss={() => setError(null)} /></div> : null}

      <div className="angelica-composer">
        {queue.length > 0 ? <span className="angelica-chip angelica-queue">{t("angelica.queued", { count: queue.length })}</span> : null}
        <div className="angelica-box" onClick={(event) => { if (event.target === event.currentTarget) inputRef.current?.focus(); }}>
          {selection ? <SelectionToggle selection={selection} attached={attachContext} onToggle={() => setAttachContext((value) => !value)} /> : null}
          <textarea ref={inputRef} className="angelica-input" rows={1} value={draft} placeholder={t("angelica.placeholder")} aria-label={t("angelica.placeholder")} onChange={(event) => setDraft(event.target.value)} onKeyDown={onComposerKey} />
          <div className="angelica-box-bar">
            <ModeMenu mode={mode} onChange={setMode} />
            <span className="angelica-box-end">
            <ModelMenu providers={providers} model={model} onChange={setModel} onOpen={loadSettings} />
            <span className="angelica-ring" role="img" aria-label={usageTitle} title={`${usageTitle}\n${t("angelica.tokens", { count: totalTokens(usage) })}`} style={{ "--fill": `${contextPercent ?? 0}%` } as CSSProperties} />
            {running ? (
              <button className="angelica-round angelica-stop" type="button" aria-label={t("angelica.stop")} title={t("angelica.stop")} onClick={stop}><UiIcon icon="square" size="xs" /></button>
            ) : null}
            <button className="angelica-round angelica-send" type="button" disabled={!draft.trim() || !model} aria-label={running ? t("angelica.queue") : t("angelica.send")} title={running ? t("angelica.queue") : t("angelica.send")} onClick={submit}>
              <UiIcon icon="arrowUp" size="sm" />
            </button>
            </span>
          </div>
        </div>
      </div>
    </section>
  );
}
