import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { aiSettings, angelicaApplyProposal, angelicaProposals, angelicaRejectProposal, angelicaCancel, angelicaConversation, angelicaConversations, angelicaDeleteConversation, angelicaSend, normalizeCommandError } from "../ipc";
import { ANGELICA, applyAgentEvent, contextFill, parseReply, resolveModel, toolSubject, totalTokens, transcriptFromMessages, type ReplySpan, type TranscriptItem } from "../angelica";
import { parseSelectionKey, selectableEfforts, selectionKey } from "../aiSettings";
import type { AgentMode, ProposalRecord, SourceBinding } from "../types";
import { AngelicaJobs } from "./AngelicaJobs";
import { AngelicaProposals } from "./AngelicaProposals";
import type { AgentEvent, AiModelSelection, AiSettingsDto, AiUsage, AngelicaEventDto, CommandError, ConversationDto, ConversationSummaryDto, EditorContextDto, ReasoningEffort } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ErrorBanner } from "./ErrorBanner";

type AngelicaPanelProps = {
  editorContext: EditorContextDto | null;
  onOpenSettings?: (() => void) | undefined;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
};

const modeLabels: Readonly<Record<AgentMode, MessageKey>> = {
  chat: "angelica.mode.chat",
  ask: "angelica.mode.ask",
  autoDraft: "angelica.mode.autoDraft",
};

const modeHints: Readonly<Record<AgentMode, MessageKey>> = {
  chat: "angelica.mode.chatHint",
  ask: "angelica.mode.askHint",
  autoDraft: "angelica.mode.autoDraftHint",
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
  cancel_job: "angelica.tool.cancelJob",
};

const effortLabels: Readonly<Record<ReasoningEffort, MessageKey>> = {
  minimal: "ai.effort.minimal",
  low: "ai.effort.low",
  medium: "ai.effort.medium",
  high: "ai.effort.high",
  xhigh: "ai.effort.xhigh",
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

function ToolCard({ item }: { item: Extract<TranscriptItem, { kind: "tool" }> }) {
  const { t } = useI18n();
  const label = toolLabels[item.name];
  const subject = toolSubject(item.name, item.arguments);
  const state = item.result === null ? "running" : item.isError ? "failed" : "done";
  return (
    <details className={`angelica-tool angelica-tool-${state}`}>
      <summary>
        <UiIcon icon={state === "running" ? "circleDot" : state === "failed" ? "circleAlert" : "check"} size="xs" />
        <span className="angelica-tool-name">{label ? t(label) : item.name}</span>
        {subject ? <code className="angelica-tool-subject">{subject}</code> : null}
      </summary>
      <div className="angelica-tool-body">
        <span className="field-label">{t("angelica.toolArguments")}</span>
        <pre>{preview(item.arguments)}</pre>
        {item.result !== null ? <><span className="field-label">{t("angelica.toolResult")}</span><pre>{preview(item.result)}</pre></> : null}
      </div>
    </details>
  );
}

function TranscriptEntry({ item }: { item: TranscriptItem }) {
  const { t } = useI18n();
  if (item.kind === "user") return <div className="angelica-user">{item.text}</div>;
  if (item.kind === "notice") return <div className="angelica-notice"><UiIcon icon="info" size="xs" />{item.text.replace(/^\[Aeria\]\s*/, "")}</div>;
  if (item.kind === "tool") return <ToolCard item={item} />;
  return (
    <div className="angelica-assistant">
      {item.reasoning ? (
        <details className="angelica-reasoning">
          <summary>{item.streaming && !item.text ? t("angelica.thinking") : t("angelica.reasoning")}</summary>
          <p>{item.reasoning}</p>
        </details>
      ) : null}
      {item.text ? <Reply text={item.text} /> : null}
    </div>
  );
}

/** Angelica's chat: conversations, live turns, and model and effort choice. */
export function AngelicaPanel({ editorContext, onOpenSettings, onReveal }: AngelicaPanelProps) {
  const { t } = useI18n();
  const [settings, setSettings] = useState<AiSettingsDto | null>(null);
  const [conversations, setConversations] = useState<ConversationSummaryDto[]>([]);
  const [conversation, setConversation] = useState<ConversationDto | null>(null);
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [running, setRunning] = useState(false);
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
    switch (event.type) {
      case "usage":
        setLastPromptTokens(event.promptTokens);
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
  const efforts = selectableEfforts(providers, model);
  const modelConfig = model ? providers.find((provider) => provider.id === model.providerId)?.models.find((entry) => entry.id === model.modelId) ?? null : null;
  const fill = contextFill(lastPromptTokens, modelConfig?.contextWindow ?? null);
  const selection = editorContext?.selection ?? null;

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
        <select className="input angelica-history" value={conversation?.id ?? ""} disabled={running} aria-label={t("angelica.history")} onChange={(event) => openConversation(event.target.value)}>
          <option value="">{t("angelica.newConversation")}</option>
          {conversations.map((entry) => <option key={entry.id} value={entry.id}>{entry.title || t("angelica.untitled")}</option>)}
        </select>
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
        ) : items.map((item) => <TranscriptEntry key={item.key} item={item} />)}
        {running && !(items[items.length - 1]?.kind === "assistant") ? <div className="angelica-working">{t("angelica.working")}</div> : null}
        {notice ? <p className="field-hint">{t(notice)}</p> : null}
      </div>

      <AngelicaJobs onError={setError} onReveal={onReveal} />

      <AngelicaProposals proposals={proposals} busy={settling} onApply={(ids) => void settle(ids, true)} onReject={(ids) => void settle(ids, false)} onReveal={onReveal} />

      {error ? <div className="angelica-error"><ErrorBanner title={t("angelica.error")} error={error} onDismiss={() => setError(null)} /></div> : null}

      <div className="angelica-composer">
        <div className="angelica-chips">
          <select className="input angelica-mode-select" value={mode} aria-label={t("angelica.mode.label")} title={t(modeHints[mode])} onChange={(event) => setMode(event.target.value as AgentMode)}>
            {(Object.keys(modeLabels) as AgentMode[]).map((value) => <option key={value} value={value}>{t(modeLabels[value])}</option>)}
          </select>
          {selection ? (
            <button type="button" className={attachContext ? "angelica-chip" : "angelica-chip off"} aria-pressed={attachContext} title={t(attachContext ? "angelica.contextOn" : "angelica.contextOff")} onClick={() => setAttachContext((value) => !value)}>
              <UiIcon icon={attachContext ? "locateFixed" : "eyeOff"} size="xs" />
              <code>{`${selection.sheet}:${selection.row}:${selection.subrow}${selection.column === null ? "" : `:${selection.column}`}`}</code>
            </button>
          ) : null}
          {queue.length > 0 ? <span className="angelica-chip">{t("angelica.queued", { count: queue.length })}</span> : null}
        </div>
        <textarea className="input angelica-input" rows={2} value={draft} placeholder={t("angelica.placeholder")} aria-label={t("angelica.placeholder")} onChange={(event) => setDraft(event.target.value)} onKeyDown={onComposerKey} />
        <div className="angelica-toolbar">
          <select className="input angelica-model" value={model ? selectionKey(model) : ""} aria-label={t("angelica.model")} title={t("angelica.model")} onFocus={loadSettings} onChange={(event) => {
            const next = parseSelectionKey(event.target.value);
            setModel(next ? resolveModel(providers, [{ ...next, effort: model?.effort ?? null }]) : null);
          }}>
            {providers.filter((provider) => provider.models.length > 0).map((provider) => (
              <optgroup key={provider.id} label={provider.name}>
                {provider.models.map((entry) => <option key={entry.id} value={selectionKey({ providerId: provider.id, modelId: entry.id })}>{entry.id}</option>)}
              </optgroup>
            ))}
          </select>
          {efforts.length > 0 && model ? (
            <select className="input angelica-effort" value={model.effort ?? ""} aria-label={t("angelica.effort")} title={t("angelica.effort")} onChange={(event) => setModel({ ...model, effort: (event.target.value || null) as ReasoningEffort | null })}>
              <option value="">{t("ai.effort.default")}</option>
              {efforts.map((effort) => <option key={effort} value={effort}>{t(effortLabels[effort])}</option>)}
            </select>
          ) : null}
          <span className="angelica-usage" title={t("angelica.usageHint", { prompt: usage.promptTokens, completion: usage.completionTokens })}>
            {fill !== null ? t("angelica.contextFill", { percent: Math.round(fill * 100) }) : null}
            {totalTokens(usage) > 0 ? <span>{t("angelica.tokens", { count: totalTokens(usage) })}</span> : null}
          </span>
          {running ? (
            <button className="button button-secondary" type="button" onClick={stop}><UiIcon icon="square" size="sm" />{t("angelica.stop")}</button>
          ) : null}
          <button className="button button-primary" type="button" disabled={!draft.trim() || !model} onClick={submit}>
            <UiIcon icon="arrowUp" size="sm" />{running ? t("angelica.queue") : t("angelica.send")}
          </button>
        </div>
      </div>
    </section>
  );
}
