import { useEffect, useId, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  aiChatGptLoginCancel,
  aiChatGptLoginStart,
  aiClearApiKey,
  aiListRemoteModels,
  aiRemoveProvider,
  aiSaveProvider,
  aiSetAgentModel,
  aiSetWebDomains,
  aiSetWorkerModel,
  aiSetApiKey,
  aiSettings,
  aiTestConnection,
} from "../ipc";
import {
  addModels,
  normalizeHeaders,
  parseContextWindow,
  parseSelectionKey,
  providerFromPreset,
  providerInput,
  reasoningEfforts,
  removeModel,
  selectableEfforts,
  selectionKey,
  syncModels,
  toggleEffort,
  toggleVision,
} from "../aiSettings";
import type { ChatGptLoginDto, ChatGptLoginEventDto } from "../types";
import type { AiHeaderConfig, AiModelConfig, AiModelSelection, AiProviderDto, AiProviderPresetDto, AiSettingsDto, CommandError, ReasoningEffort } from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

type Run = <T>(operation: () => Promise<T>) => Promise<T | undefined>;

type TestResult = { state: "running" } | { state: "ok"; text: string } | { state: "failed"; text: string };

const effortLabels: Readonly<Record<ReasoningEffort, MessageKey>> = {
  minimal: "ai.effort.minimal",
  low: "ai.effort.low",
  medium: "ai.effort.medium",
  high: "ai.effort.high",
  xhigh: "ai.effort.xhigh",
};

/** Local AI provider settings: presets, keys, models, and connection checks. */
export function AiProvidersSettings() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<AiSettingsDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    aiSettings().then((next) => { if (active) setSettings(next); }).catch((reason: CommandError) => { if (active) setError(reason); });
    return () => { active = false; };
  }, []);

  const run: Run = async (operation) => {
    setBusy(true);
    setError(null);
    try {
      return await operation();
    } catch (reason) {
      setError(reason as CommandError);
      return undefined;
    } finally {
      setBusy(false);
    }
  };

  const apply = async (operation: () => Promise<AiSettingsDto>) => {
    const next = await run(operation);
    if (next) setSettings(next);
    return next !== undefined;
  };

  return (
    <div className="ai-settings">
      {error ? <ErrorBanner title={t("ai.settings.error")} error={error} onDismiss={() => setError(null)} /> : null}
      {settings === null ? (
        error ? null : <p className="muted">{t("common.loading")}</p>
      ) : (
        <>
          <ModelPicker settings={settings} selection={settings.agentModel} disabled={busy} apply={apply} set={aiSetAgentModel} label="ai.settings.agentModel" empty="ai.settings.noAgentModel" hint="ai.settings.agentModelHint" />
          <WebDomains settings={settings} disabled={busy} apply={apply} />
          <ModelPicker settings={settings} selection={settings.workerModel} inherited={settings.agentModel} disabled={busy} apply={apply} set={aiSetWorkerModel} label="ai.settings.workerModel" empty="ai.settings.noWorkerModel" hint="ai.settings.workerModelHint" />
          {settings.providers.map((provider) => (
            <ProviderCard key={provider.id} provider={provider} disabled={busy} apply={apply} run={run} />
          ))}
          {settings.providers.length === 0 ? <p className="field-hint">{t("ai.settings.noProviders")}</p> : null}
          <AddProvider presets={settings.presets} disabled={busy} apply={apply} />
        </>
      )}
    </div>
  );
}

/** Domains whose pages Angelica reads without asking, one per line. */
function WebDomains({ settings, disabled, apply }: { settings: AiSettingsDto; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean> }) {
  const { t } = useI18n();
  const id = useId();
  const saved = settings.webDomains.join("\n");
  const [text, setText] = useState(saved);
  useEffect(() => setText(saved), [saved]);
  const domains = text.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  const changed = domains.join("\n") !== saved;
  return (
    <div className="ai-web-domains field">
      <label className="field-label" htmlFor={id}>{t("ai.settings.webDomains")}</label>
      <textarea id={id} className="input" rows={3} value={text} disabled={disabled} placeholder="ffxiv.consolegameswiki.com" spellCheck={false} onChange={(event) => setText(event.target.value)} />
      <div className="ai-web-domains-actions">
        <p className="field-hint">{t("ai.settings.webDomainsHint")}</p>
        <button className="button button-secondary" type="button" disabled={disabled || !changed} onClick={() => void apply(() => aiSetWebDomains(domains))}>{t("ai.settings.saveDomains")}</button>
      </div>
    </div>
  );
}

type ModelPickerProps = {
  settings: AiSettingsDto;
  selection: AiModelSelection | null;
  /** The selection used while `selection` is empty; its effort can be overridden. */
  inherited?: AiModelSelection | null;
  disabled: boolean;
  apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean>;
  set: (selection: AiModelSelection | null) => Promise<AiSettingsDto>;
  label: MessageKey;
  empty: MessageKey;
  hint: MessageKey;
};

/** A default model and effort: Angelica's, or the job workers'. */
function ModelPicker({ settings, selection, inherited = null, disabled, apply, set, label, empty, hint }: ModelPickerProps) {
  const { t } = useI18n();
  const modelId = useId();
  const effortId = useId();
  const efforts = selectableEfforts(settings.providers, selection);
  // Without an own model, an effort choice pins the inherited model with it.
  const inheritedEfforts = selection === null ? selectableEfforts(settings.providers, inherited) : [];
  const hasModels = settings.providers.some((provider) => provider.models.length > 0);

  return (
    <div className="ai-agent-model">
      <div className="field">
        <label className="field-label" htmlFor={modelId}>{t(label)}</label>
        <Select
          id={modelId}
          value={selection ? selectionKey(selection) : ""}
          disabled={disabled || !hasModels}
          onChange={(value) => {
            const next = parseSelectionKey(value);
            void apply(() => set(next ? { ...next, effort: null } : null));
          }}
          groups={[
            { options: [{ value: "", label: t(empty) }] },
            ...settings.providers.filter((provider) => provider.models.length > 0).map((provider) => ({
              label: provider.name,
              options: provider.models.map((model) => ({ value: selectionKey({ providerId: provider.id, modelId: model.id }), label: model.id })),
            })),
          ]}
        />
      </div>
      {efforts.length > 0 && selection ? (
        <div className="field">
          <label className="field-label" htmlFor={effortId}>{t("ai.settings.effort")}</label>
          <Select
            id={effortId}
            value={selection.effort ?? ""}
            disabled={disabled}
            onChange={(value) => {
              const effort = (value || null) as ReasoningEffort | null;
              void apply(() => set({ ...selection, effort }));
            }}
            options={[{ value: "", label: t("ai.effort.default") }, ...efforts.map((effort) => ({ value: effort, label: t(effortLabels[effort]) }))]}
          />
        </div>
      ) : null}
      {inherited && inheritedEfforts.length > 0 ? (
        <div className="field">
          <label className="field-label" htmlFor={effortId}>{t("ai.settings.effort")}</label>
          <Select
            id={effortId}
            value=""
            disabled={disabled}
            onChange={(value) => {
              if (value) void apply(() => set({ ...inherited, effort: value as ReasoningEffort }));
            }}
            options={[
              { value: "", label: inherited.effort ? t("ai.settings.inheritedEffort", { effort: t(effortLabels[inherited.effort]) }) : t("ai.settings.inheritedDefaultEffort") },
              ...inheritedEfforts.map((effort) => ({ value: effort, label: t(effortLabels[effort]) })),
            ]}
          />
        </div>
      ) : null}
      <p className="field-hint">{t(hint)}</p>
    </div>
  );
}

function ProviderCard({ provider, disabled, apply, run }: { provider: AiProviderDto; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean>; run: Run }) {
  const { t } = useI18n();
  const [name, setName] = useState(provider.name);
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl);
  const [apiKey, setApiKey] = useState("");
  const [newModel, setNewModel] = useState("");
  const [syncResult, setSyncResult] = useState<{ added: number; removed: number } | null>(null);
  const [testEffort, setTestEffort] = useState<ReasoningEffort | "">("");
  const [tests, setTests] = useState<Record<string, TestResult>>({});
  const [confirmRemove, setConfirmRemove] = useState(false);

  useEffect(() => { setName(provider.name); setBaseUrl(provider.baseUrl); }, [provider.name, provider.baseUrl]);

  const saveModels = (models: AiModelConfig[]) => apply(() => aiSaveProvider(providerInput(provider, { models })));
  const saveField = (changes: { name?: string; baseUrl?: string }) => {
    if ((changes.name ?? provider.name) === provider.name && (changes.baseUrl ?? provider.baseUrl) === provider.baseUrl) return;
    void apply(() => aiSaveProvider(providerInput(provider, changes))).then((saved) => {
      if (!saved) { setName(provider.name); setBaseUrl(provider.baseUrl); }
    });
  };

  const syncFromProvider = async () => {
    const remote = await run(() => aiListRemoteModels(provider.id));
    if (!remote) return;
    const result = syncModels(provider.models, remote);
    if (await saveModels(result.models)) setSyncResult({ added: result.added, removed: result.removed });
  };

  const testModel = async (modelId: string) => {
    setTests((current) => ({ ...current, [modelId]: { state: "running" } }));
    try {
      const check = await aiTestConnection(provider.id, modelId, testEffort || null);
      const text = check.model && check.model !== modelId
        ? t("ai.settings.testOkModel", { ms: check.latencyMs, model: check.model })
        : t("ai.settings.testOk", { ms: check.latencyMs });
      setTests((current) => ({ ...current, [modelId]: { state: "ok", text } }));
    } catch (reason) {
      const failure = reason as CommandError;
      setTests((current) => ({ ...current, [modelId]: { state: "failed", text: failure.message } }));
    }
  };

  const chatGpt = provider.kind === "chatGpt";
  const keyLabel: Record<AiProviderDto["apiKey"], MessageKey> = chatGpt
    ? { stored: "ai.chatGpt.signedIn", missing: "ai.chatGpt.signedOut", unavailable: "ai.settings.keyUnavailable" }
    : { stored: "ai.settings.keyStored", missing: "ai.settings.keyMissing", unavailable: "ai.settings.keyUnavailable" };

  return (
    <section className="ai-provider" aria-label={provider.name}>
      <header className="ai-provider-head">
        <input className="input ai-provider-name" value={name} disabled={disabled} aria-label={t("ai.settings.providerName")} onChange={(event) => setName(event.target.value)} onBlur={() => saveField({ name })} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }} />
        <span className={`ai-key-state ai-key-${provider.apiKey}`}>{t(keyLabel[provider.apiKey])}</span>
        {confirmRemove ? (
          <>
            <button className="button button-danger" type="button" disabled={disabled} onClick={() => void apply(() => aiRemoveProvider(provider.id))}>{t("ai.settings.confirmRemove")}</button>
            <button className="button button-ghost" type="button" onClick={() => setConfirmRemove(false)}>{t("common.cancel")}</button>
          </>
        ) : (
          <button className="icon-button icon-button-ghost" type="button" disabled={disabled} aria-label={t("ai.settings.removeProvider")} title={t("ai.settings.removeProvider")} onClick={() => setConfirmRemove(true)}><UiIcon icon="trash" size="sm" /></button>
        )}
      </header>

      {chatGpt ? <ChatGptSignIn provider={provider} disabled={disabled} apply={apply} onSignedIn={() => { if (provider.models.length === 0) void syncFromProvider(); }} /> : <>
      <label className="field">
        <span className="field-label">{t("ai.settings.baseUrl")}</span>
        <input className="input" value={baseUrl} disabled={disabled} spellCheck={false} onChange={(event) => setBaseUrl(event.target.value)} onBlur={() => saveField({ baseUrl })} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }} />
      </label>

      <form className="ai-key-row" onSubmit={(event) => {
        event.preventDefault();
        void apply(() => aiSetApiKey(provider.id, apiKey)).then((saved) => {
          if (!saved) return;
          setApiKey("");
          // A new provider has no models yet: ask the provider right away.
          if (provider.models.length === 0) void syncFromProvider();
        });
      }}>
        <label className="field">
          <span className="field-label">{t("ai.settings.apiKey")}</span>
          <input className="input" type="password" autoComplete="off" spellCheck={false} value={apiKey} disabled={disabled} placeholder={provider.apiKey === "stored" ? t("ai.settings.replaceKeyPlaceholder") : t("ai.settings.keyPlaceholder")} onChange={(event) => setApiKey(event.target.value)} />
        </label>
        <button className="button button-secondary" type="submit" disabled={disabled || !apiKey.trim()}>{t("ai.settings.saveKey")}</button>
        {provider.apiKey === "stored" ? <button className="button button-ghost" type="button" disabled={disabled} onClick={() => void apply(() => aiClearApiKey(provider.id))}>{t("ai.settings.clearKey")}</button> : null}
      </form>
      <p className="field-hint">{t("ai.settings.keyHint")}</p>

      <HeaderSettings provider={provider} disabled={disabled} apply={apply} />
      </>}

      <div className="ai-models-head">
        <strong>{t("ai.settings.models")}</strong>
        <div className="ai-test-effort">
          <span>{t("ai.settings.testEffort")}</span>
          <Select
            variant="quiet"
            label={t("ai.settings.testEffort")}
            value={testEffort}
            onChange={(value) => setTestEffort(value as ReasoningEffort | "")}
            options={[{ value: "", label: t("ai.effort.none") }, ...reasoningEfforts.map((effort) => ({ value: effort, label: t(effortLabels[effort]) }))]}
          />
        </div>
      </div>
      {provider.models.length === 0 ? <p className="field-hint">{t("ai.settings.noModels")}</p> : (
        <ul className="ai-models">
          {provider.models.map((model) => (
            <ModelRow key={model.id} model={model} disabled={disabled} test={tests[model.id]} canTest={provider.apiKey === "stored"}
              onToggleEffort={(effort) => void saveModels(toggleEffort(provider.models, model.id, effort))}
              onToggleVision={() => void saveModels(toggleVision(provider.models, model.id))}
              onContextWindow={(contextWindow) => void saveModels(provider.models.map((entry) => entry.id === model.id ? { ...entry, contextWindow } : entry))}
              onRemove={() => void saveModels(removeModel(provider.models, model.id))}
              onTest={() => void testModel(model.id)} />
          ))}
        </ul>
      )}
      <form className="ai-add-model" onSubmit={(event) => {
        event.preventDefault();
        void saveModels(addModels(provider.models, [newModel])).then((saved) => { if (saved) setNewModel(""); });
      }}>
        <button className="button button-secondary" type="button" disabled={disabled || provider.apiKey !== "stored"} title={t("ai.settings.syncModelsHint")} onClick={() => void syncFromProvider()}>
          <UiIcon icon="refreshCw" size="sm" />{t("ai.settings.syncModels")}
        </button>
        <input className="input" value={newModel} disabled={disabled} spellCheck={false} placeholder={t("ai.settings.modelPlaceholder")} aria-label={t("ai.settings.addModel")} onChange={(event) => setNewModel(event.target.value)} />
        <button className="button button-ghost" type="submit" disabled={disabled || !newModel.trim()}>{t("ai.settings.addModel")}</button>
      </form>
      {syncResult ? <p className="field-hint">{t("ai.settings.syncResult", { added: syncResult.added, removed: syncResult.removed, total: provider.models.length })}</p> : null}
    </section>
  );
}

function ModelRow({ model, disabled, test, canTest, onToggleEffort, onToggleVision, onContextWindow, onRemove, onTest }: {
  model: AiModelConfig;
  disabled: boolean;
  test: TestResult | undefined;
  canTest: boolean;
  onToggleEffort: (effort: ReasoningEffort) => void;
  onToggleVision: () => void;
  onContextWindow: (value: number | null) => void;
  onRemove: () => void;
  onTest: () => void;
}) {
  const { t } = useI18n();
  const [contextWindow, setContextWindow] = useState(model.contextWindow === null ? "" : String(model.contextWindow));
  useEffect(() => setContextWindow(model.contextWindow === null ? "" : String(model.contextWindow)), [model.contextWindow]);
  const parsed = parseContextWindow(contextWindow);

  return (
    <li className="ai-model">
      <div className="ai-model-main">
        <code className="ai-model-id" title={model.id}>{model.id}</code>
        <label className="ai-context-field" title={t("ai.settings.contextWindow")}>
          <span>{t("ai.settings.contextShort")}</span>
          <input className={parsed === undefined ? "input ai-context invalid" : "input ai-context"} inputMode="numeric" value={contextWindow} disabled={disabled} placeholder={t("ai.settings.contextPlaceholder")} aria-label={t("ai.settings.contextWindow")}
            onChange={(event) => setContextWindow(event.target.value)}
            onBlur={() => { if (parsed !== undefined && parsed !== model.contextWindow) onContextWindow(parsed); }} />
        </label>
        <button className="button button-ghost" type="button" disabled={!canTest || test?.state === "running"} title={canTest ? undefined : t("ai.settings.testNeedsKey")} onClick={onTest}>
          {test?.state === "running" ? t("ai.settings.testing") : t("ai.settings.test")}
        </button>
        <button className="icon-button icon-button-ghost" type="button" disabled={disabled} aria-label={t("ai.settings.removeModel")} title={t("ai.settings.removeModel")} onClick={onRemove}><UiIcon icon="x" size="sm" /></button>
      </div>
      <div className="ai-model-efforts">
        <span className="ai-model-caption" title={t("ai.settings.efforts")}>{t("ai.settings.effortsShort")}</span>
        <div className="ai-efforts" role="group" aria-label={t("ai.settings.efforts")}>
          {reasoningEfforts.map((effort) => {
            const enabled = model.reasoningEfforts.includes(effort);
            return <button key={effort} type="button" className={enabled ? "ai-effort-chip enabled" : "ai-effort-chip"} aria-pressed={enabled} disabled={disabled} onClick={() => onToggleEffort(effort)}>{t(effortLabels[effort])}</button>;
          })}
        </div>
      </div>
      <div className="ai-model-efforts">
        <span className="ai-model-caption" title={t("ai.settings.visionHint")}>{t("ai.settings.inputShort")}</span>
        <div className="ai-efforts">
          <button type="button" className={model.vision ? "ai-effort-chip enabled" : "ai-effort-chip"} aria-pressed={model.vision === true} disabled={disabled} title={t("ai.settings.visionHint")} onClick={onToggleVision}>
            <UiIcon icon="image" size="xs" />{t("ai.settings.vision")}
          </button>
        </div>
      </div>
      {test && test.state !== "running" ? (
        <p className={test.state === "ok" ? "ai-test-result ok" : "ai-test-result failed"}>
          <UiIcon icon={test.state === "ok" ? "circleCheck" : "circleAlert"} size="xs" />{test.text}
        </p>
      ) : null}
    </li>
  );
}

function ChatGptSignIn({ provider, disabled, apply, onSignedIn }: { provider: AiProviderDto; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean>; onSignedIn: () => void }) {
  const { t } = useI18n();
  const [login, setLogin] = useState<ChatGptLoginDto | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    if (!login) return;
    const subscription = listen<ChatGptLoginEventDto>("ai://chatgpt-login", ({ payload }) => {
      if (payload.loginId !== login.loginId) return;
      setLogin(null);
      if (payload.succeeded) {
        void apply(() => aiSettings()).then(onSignedIn);
      } else {
        setFailure(payload.message ?? payload.code ?? "");
      }
    });
    return () => { void subscription.then((unlisten) => unlisten()); };
  }, [apply, login, onSignedIn]);

  const start = async () => {
    setFailure(null);
    setStarting(true);
    try {
      setLogin(await aiChatGptLoginStart(provider.id));
    } catch (reason) {
      setFailure((reason as CommandError).message);
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="ai-chatgpt">
      <p className="ai-chatgpt-warning"><UiIcon icon="triangleAlert" size="sm" />{t("ai.chatGpt.warning")}</p>
      {login ? (
        <div className="ai-chatgpt-code">
          <p>{t(login.browserOpened ? "ai.chatGpt.enterCode" : "ai.chatGpt.openAndEnterCode", { url: login.verificationUrl })}</p>
          <code className="ai-chatgpt-user-code">{login.userCode}</code>
          <div className="ai-chatgpt-actions">
            <button className="button button-ghost" type="button" onClick={() => void navigator.clipboard?.writeText(login.userCode)}><UiIcon icon="copy" size="sm" />{t("ai.chatGpt.copyCode")}</button>
            <button className="button button-ghost" type="button" onClick={() => { void aiChatGptLoginCancel(login.loginId); setLogin(null); }}>{t("common.cancel")}</button>
          </div>
          <p className="field-hint">{t("ai.chatGpt.waiting")}</p>
        </div>
      ) : (
        <div className="ai-chatgpt-actions">
          {provider.apiKey === "stored" ? (
            <button className="button button-ghost" type="button" disabled={disabled} onClick={() => void apply(() => aiClearApiKey(provider.id))}>{t("ai.chatGpt.signOut")}</button>
          ) : (
            <button className="button button-primary" type="button" disabled={disabled || starting} onClick={() => void start()}>{t("ai.chatGpt.signIn")}</button>
          )}
        </div>
      )}
      {failure ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{failure}</p> : null}
    </div>
  );
}

function HeaderSettings({ provider, disabled, apply }: { provider: AiProviderDto; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean> }) {
  const { t } = useI18n();
  const sessionId = useId();
  const [sessionHeader, setSessionHeader] = useState(provider.sessionHeader ?? "");
  const [headers, setHeaders] = useState<AiHeaderConfig[]>(provider.headers);
  useEffect(() => { setSessionHeader(provider.sessionHeader ?? ""); setHeaders(provider.headers); }, [provider.sessionHeader, provider.headers]);

  const normalizedSession = sessionHeader.trim().toLowerCase() || null;
  const normalized = normalizeHeaders(headers);
  const dirty = normalizedSession !== provider.sessionHeader || JSON.stringify(normalized) !== JSON.stringify(provider.headers) || normalized.length !== headers.length;
  const update = (index: number, change: Partial<AiHeaderConfig>) => setHeaders((current) => current.map((header, position) => position === index ? { ...header, ...change } : header));

  return (
    <details className="ai-headers" open={provider.headers.length > 0 || undefined}>
      <summary>{t("ai.settings.headers")}</summary>
      <form className="ai-headers-body" onSubmit={(event) => {
        event.preventDefault();
        void apply(() => aiSaveProvider(providerInput(provider, { sessionHeader: normalizedSession, headers: normalized })));
      }}>
        <div className="field">
          <label className="field-label" htmlFor={sessionId}>{t("ai.settings.sessionHeader")}</label>
          <input id={sessionId} className="input" value={sessionHeader} disabled={disabled} spellCheck={false} placeholder="x-provider-session" onChange={(event) => setSessionHeader(event.target.value)} />
          <p className="field-hint">{t("ai.settings.sessionHeaderHint")}</p>
        </div>
        <div className="field">
          <span className="field-label">{t("ai.settings.extraHeaders")}</span>
          {headers.map((header, index) => (
            <div className="ai-header-row" key={index}>
              <input className="input" value={header.name} disabled={disabled} spellCheck={false} placeholder={t("ai.settings.headerName")} aria-label={t("ai.settings.headerName")} onChange={(event) => update(index, { name: event.target.value })} />
              <input className="input" value={header.value} disabled={disabled} spellCheck={false} placeholder={t("ai.settings.headerValue")} aria-label={t("ai.settings.headerValue")} onChange={(event) => update(index, { value: event.target.value })} />
              <button className="icon-button icon-button-ghost" type="button" disabled={disabled} aria-label={t("ai.settings.removeHeader")} title={t("ai.settings.removeHeader")} onClick={() => setHeaders((current) => current.filter((_, position) => position !== index))}><UiIcon icon="x" size="sm" /></button>
            </div>
          ))}
          <p className="field-hint">{t("ai.settings.extraHeadersHint")}</p>
        </div>
        <div className="ai-headers-actions">
          <button className="button button-ghost" type="button" disabled={disabled} onClick={() => setHeaders((current) => [...current, { name: "", value: "" }])}><UiIcon icon="plus" size="sm" />{t("ai.settings.addHeader")}</button>
          <button className="button button-secondary" type="submit" disabled={disabled || !dirty}>{t("ai.settings.saveHeaders")}</button>
        </div>
      </form>
    </details>
  );
}

function AddProvider({ presets, disabled, apply }: { presets: AiProviderPresetDto[]; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean> }) {
  const { t } = useI18n();
  const [custom, setCustom] = useState<AiProviderPresetDto | null>(null);
  const [baseUrl, setBaseUrl] = useState("");

  return (
    <div className="ai-add-provider">
      <strong>{t("ai.settings.addProvider")}</strong>
      <div className="ai-preset-row">
        {presets.map((preset) => (
          <button key={preset.kind} className="button button-secondary" type="button" disabled={disabled} onClick={() => {
            if (preset.baseUrl === null) { setCustom(preset); return; }
            void apply(() => aiSaveProvider(providerFromPreset(preset)));
          }}>
            <UiIcon icon="plus" size="sm" />{preset.kind === "chatGpt" ? t("ai.chatGpt.preset") : preset.name}
          </button>
        ))}
      </div>
      {custom ? (
        <form className="ai-custom-provider" onSubmit={(event) => {
          event.preventDefault();
          void apply(() => aiSaveProvider(providerFromPreset(custom, baseUrl))).then((saved) => { if (saved) { setCustom(null); setBaseUrl(""); } });
        }}>
          <label className="field">
            <span className="field-label">{t("ai.settings.baseUrl")}</span>
            <input className="input" value={baseUrl} autoFocus spellCheck={false} placeholder="https://example.com/v1" onChange={(event) => setBaseUrl(event.target.value)} />
          </label>
          <button className="button button-primary" type="submit" disabled={disabled || !baseUrl.trim()}>{t("ai.settings.create")}</button>
          <button className="button button-ghost" type="button" onClick={() => setCustom(null)}>{t("common.cancel")}</button>
        </form>
      ) : null}
    </div>
  );
}
