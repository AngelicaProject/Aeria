import { useEffect, useId, useState } from "react";
import {
  aiClearApiKey,
  aiListRemoteModels,
  aiRemoveProvider,
  aiSaveProvider,
  aiSetAgentModel,
  aiSetApiKey,
  aiSettings,
  aiTestConnection,
} from "../ipc";
import {
  addModels,
  parseContextWindow,
  parseSelectionKey,
  providerFromPreset,
  providerInput,
  reasoningEfforts,
  removeModel,
  selectableEfforts,
  selectionKey,
  toggleEffort,
} from "../aiSettings";
import type { AiModelConfig, AiProviderDto, AiProviderPresetDto, AiSettingsDto, CommandError, ReasoningEffort } from "../types";
import { ErrorBanner } from "./ErrorBanner";
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
          <AgentModelPicker settings={settings} disabled={busy} apply={apply} />
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

function AgentModelPicker({ settings, disabled, apply }: { settings: AiSettingsDto; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean> }) {
  const { t } = useI18n();
  const modelId = useId();
  const effortId = useId();
  const selection = settings.agentModel;
  const efforts = selectableEfforts(settings.providers, selection);
  const hasModels = settings.providers.some((provider) => provider.models.length > 0);

  return (
    <div className="ai-agent-model">
      <div className="field">
        <label className="field-label" htmlFor={modelId}>{t("ai.settings.agentModel")}</label>
        <select
          id={modelId}
          className="input"
          value={selection ? selectionKey(selection) : ""}
          disabled={disabled || !hasModels}
          onChange={(event) => {
            const next = parseSelectionKey(event.target.value);
            void apply(() => aiSetAgentModel(next ? { ...next, effort: null } : null));
          }}
        >
          <option value="">{t("ai.settings.noAgentModel")}</option>
          {settings.providers.filter((provider) => provider.models.length > 0).map((provider) => (
            <optgroup key={provider.id} label={provider.name}>
              {provider.models.map((model) => <option key={model.id} value={selectionKey({ providerId: provider.id, modelId: model.id })}>{model.id}</option>)}
            </optgroup>
          ))}
        </select>
      </div>
      {efforts.length > 0 && selection ? (
        <div className="field">
          <label className="field-label" htmlFor={effortId}>{t("ai.settings.effort")}</label>
          <select id={effortId} className="input" value={selection.effort ?? ""} disabled={disabled} onChange={(event) => {
            const effort = (event.target.value || null) as ReasoningEffort | null;
            void apply(() => aiSetAgentModel({ ...selection, effort }));
          }}>
            <option value="">{t("ai.effort.default")}</option>
            {efforts.map((effort) => <option key={effort} value={effort}>{t(effortLabels[effort])}</option>)}
          </select>
        </div>
      ) : null}
      <p className="field-hint">{t("ai.settings.agentModelHint")}</p>
    </div>
  );
}

function ProviderCard({ provider, disabled, apply, run }: { provider: AiProviderDto; disabled: boolean; apply: (operation: () => Promise<AiSettingsDto>) => Promise<boolean>; run: Run }) {
  const { t } = useI18n();
  const [name, setName] = useState(provider.name);
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl);
  const [apiKey, setApiKey] = useState("");
  const [newModel, setNewModel] = useState("");
  const [remoteModels, setRemoteModels] = useState<string[] | null>(null);
  const [testEffort, setTestEffort] = useState<ReasoningEffort | "">("");
  const [tests, setTests] = useState<Record<string, TestResult>>({});
  const [confirmRemove, setConfirmRemove] = useState(false);
  const listId = useId();

  useEffect(() => { setName(provider.name); setBaseUrl(provider.baseUrl); }, [provider.name, provider.baseUrl]);

  const saveModels = (models: AiModelConfig[]) => apply(() => aiSaveProvider(providerInput(provider, { models })));
  const saveField = (changes: { name?: string; baseUrl?: string }) => {
    if ((changes.name ?? provider.name) === provider.name && (changes.baseUrl ?? provider.baseUrl) === provider.baseUrl) return;
    void apply(() => aiSaveProvider(providerInput(provider, changes))).then((saved) => {
      if (!saved) { setName(provider.name); setBaseUrl(provider.baseUrl); }
    });
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

  const keyLabel: Record<AiProviderDto["apiKey"], MessageKey> = { stored: "ai.settings.keyStored", missing: "ai.settings.keyMissing", unavailable: "ai.settings.keyUnavailable" };

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

      <label className="field">
        <span className="field-label">{t("ai.settings.baseUrl")}</span>
        <input className="input" value={baseUrl} disabled={disabled} spellCheck={false} onChange={(event) => setBaseUrl(event.target.value)} onBlur={() => saveField({ baseUrl })} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }} />
      </label>

      <form className="ai-key-row" onSubmit={(event) => {
        event.preventDefault();
        void apply(() => aiSetApiKey(provider.id, apiKey)).then((saved) => { if (saved) setApiKey(""); });
      }}>
        <label className="field">
          <span className="field-label">{t("ai.settings.apiKey")}</span>
          <input className="input" type="password" autoComplete="off" spellCheck={false} value={apiKey} disabled={disabled} placeholder={provider.apiKey === "stored" ? t("ai.settings.replaceKeyPlaceholder") : t("ai.settings.keyPlaceholder")} onChange={(event) => setApiKey(event.target.value)} />
        </label>
        <button className="button button-secondary" type="submit" disabled={disabled || !apiKey.trim()}>{t("ai.settings.saveKey")}</button>
        {provider.apiKey === "stored" ? <button className="button button-ghost" type="button" disabled={disabled} onClick={() => void apply(() => aiClearApiKey(provider.id))}>{t("ai.settings.clearKey")}</button> : null}
      </form>
      <p className="field-hint">{t("ai.settings.keyHint")}</p>

      <div className="ai-models-head">
        <strong>{t("ai.settings.models")}</strong>
        <label className="ai-test-effort">
          <span>{t("ai.settings.testEffort")}</span>
          <select className="input" value={testEffort} onChange={(event) => setTestEffort(event.target.value as ReasoningEffort | "")}>
            <option value="">{t("ai.effort.none")}</option>
            {reasoningEfforts.map((effort) => <option key={effort} value={effort}>{t(effortLabels[effort])}</option>)}
          </select>
        </label>
      </div>
      {provider.models.length === 0 ? <p className="field-hint">{t("ai.settings.noModels")}</p> : (
        <ul className="ai-models">
          {provider.models.map((model) => (
            <ModelRow key={model.id} model={model} disabled={disabled} test={tests[model.id]} canTest={provider.apiKey === "stored"}
              onToggleEffort={(effort) => void saveModels(toggleEffort(provider.models, model.id, effort))}
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
        <input className="input" value={newModel} list={listId} disabled={disabled} spellCheck={false} placeholder={t("ai.settings.modelPlaceholder")} aria-label={t("ai.settings.addModel")} onChange={(event) => setNewModel(event.target.value)} />
        <datalist id={listId}>{(remoteModels ?? []).map((id) => <option key={id} value={id} />)}</datalist>
        <button className="button button-secondary" type="submit" disabled={disabled || !newModel.trim()}>{t("ai.settings.addModel")}</button>
        <button className="button button-ghost" type="button" disabled={disabled || provider.apiKey !== "stored"} title={t("ai.settings.fetchModelsHint")} onClick={() => void run(() => aiListRemoteModels(provider.id)).then((ids) => { if (ids) setRemoteModels(ids); })}>
          <UiIcon icon="refreshCw" size="sm" />{t("ai.settings.fetchModels")}
        </button>
      </form>
      {remoteModels ? <p className="field-hint">{t("ai.settings.remoteModels", { count: remoteModels.length })}</p> : null}
    </section>
  );
}

function ModelRow({ model, disabled, test, canTest, onToggleEffort, onContextWindow, onRemove, onTest }: {
  model: AiModelConfig;
  disabled: boolean;
  test: TestResult | undefined;
  canTest: boolean;
  onToggleEffort: (effort: ReasoningEffort) => void;
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
      <code className="ai-model-id">{model.id}</code>
      <div className="ai-efforts" role="group" aria-label={t("ai.settings.efforts")}>
        {reasoningEfforts.map((effort) => {
          const enabled = model.reasoningEfforts.includes(effort);
          return <button key={effort} type="button" className={enabled ? "ai-effort-chip enabled" : "ai-effort-chip"} aria-pressed={enabled} disabled={disabled} onClick={() => onToggleEffort(effort)}>{t(effortLabels[effort])}</button>;
        })}
      </div>
      <input className={parsed === undefined ? "input ai-context invalid" : "input ai-context"} inputMode="numeric" value={contextWindow} disabled={disabled} placeholder={t("ai.settings.contextPlaceholder")} aria-label={t("ai.settings.contextWindow")} title={t("ai.settings.contextWindow")}
        onChange={(event) => setContextWindow(event.target.value)}
        onBlur={() => { if (parsed !== undefined && parsed !== model.contextWindow) onContextWindow(parsed); }} />
      <button className="button button-ghost" type="button" disabled={!canTest || test?.state === "running"} title={canTest ? undefined : t("ai.settings.testNeedsKey")} onClick={onTest}>
        {test?.state === "running" ? t("ai.settings.testing") : t("ai.settings.test")}
      </button>
      <button className="icon-button icon-button-ghost" type="button" disabled={disabled} aria-label={t("ai.settings.removeModel")} title={t("ai.settings.removeModel")} onClick={onRemove}><UiIcon icon="x" size="sm" /></button>
      {test && test.state !== "running" ? (
        <p className={test.state === "ok" ? "ai-test-result ok" : "ai-test-result failed"}>
          <UiIcon icon={test.state === "ok" ? "circleCheck" : "circleAlert"} size="xs" />{test.text}
        </p>
      ) : null}
    </li>
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
            <UiIcon icon="plus" size="sm" />{preset.name}
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
