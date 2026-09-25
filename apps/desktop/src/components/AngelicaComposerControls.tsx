import { useState, type KeyboardEvent } from "react";
import { DropdownMenu, Popover } from "radix-ui";
import { parseSelectionKey, selectableEfforts, selectionKey } from "../aiSettings";
import { resolveModel } from "../angelica";
import type { AgentMode, AiModelSelection, AiProviderDto, ReasoningEffort, UnitLocationDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

export const modeLabels: Readonly<Record<AgentMode, MessageKey>> = {
  chat: "angelica.mode.chat",
  ask: "angelica.mode.ask",
  autoDraft: "angelica.mode.autoDraft",
};

const modeHints: Readonly<Record<AgentMode, MessageKey>> = {
  chat: "angelica.mode.chatHint",
  ask: "angelica.mode.askHint",
  autoDraft: "angelica.mode.autoDraftHint",
};

export const effortLabels: Readonly<Record<ReasoningEffort, MessageKey>> = {
  minimal: "ai.effort.minimal",
  low: "ai.effort.low",
  medium: "ai.effort.medium",
  high: "ai.effort.high",
  xhigh: "ai.effort.xhigh",
};

/** The conversation mode as a quiet text menu with a hint per mode. */
export function ModeMenu({ mode, onChange }: { mode: AgentMode; onChange: (mode: AgentMode) => void }) {
  const { t } = useI18n();
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <button className="angelica-control angelica-mode" type="button" title={`${t("angelica.mode.label")}: ${t(modeHints[mode])}`}>
          <span className="angelica-control-text">{t(modeLabels[mode])}</span>
          <UiIcon icon="chevronDown" size="xs" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content className="menu-content angelica-menu" side="top" align="start" sideOffset={6} collisionPadding={8}>
          <DropdownMenu.Label className="menu-label angelica-menu-label">{t("angelica.mode.label")}</DropdownMenu.Label>
          <DropdownMenu.RadioGroup value={mode} onValueChange={(value) => onChange(value as AgentMode)}>
            {(Object.keys(modeLabels) as AgentMode[]).map((value) => (
              <DropdownMenu.RadioItem key={value} className="menu-item angelica-menu-item" value={value}>
                <span className="menu-item-check"><DropdownMenu.ItemIndicator><UiIcon icon="check" size="xs" /></DropdownMenu.ItemIndicator></span>
                <span className="angelica-menu-text">
                  <span className="menu-item-label">{t(modeLabels[value])}</span>
                  <span className="angelica-menu-hint">{t(modeHints[value])}</span>
                </span>
              </DropdownMenu.RadioItem>
            ))}
          </DropdownMenu.RadioGroup>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

/** The selected occurrence Angelica receives with the message; click to stop sending it. */
export function SelectionToggle({ selection, attached, onToggle }: { selection: UnitLocationDto; attached: boolean; onToggle: () => void }) {
  const { t } = useI18n();
  const full = `${selection.sheet}:${selection.row}:${selection.subrow}${selection.column === null ? "" : `:${selection.column}`}`;
  const sheet = selection.sheet.split("/").at(-1) ?? selection.sheet;
  const short = `${sheet}:${selection.row}:${selection.subrow}${selection.column === null ? "" : `:${selection.column}`}`;
  return (
    <button type="button" className={attached ? "angelica-control angelica-selection" : "angelica-control angelica-selection off"} aria-pressed={attached} title={`${full}\n${t(attached ? "angelica.contextOn" : "angelica.contextOff")}`} onClick={onToggle}>
      <UiIcon icon={attached ? "locateFixed" : "eyeOff"} size="xs" />
      <span className="angelica-selection-text">{short}</span>
    </button>
  );
}

/** Reasoning effort as a stepped slider from faster to smarter. */
function EffortSlider({ efforts, value, onChange }: { efforts: readonly ReasoningEffort[]; value: ReasoningEffort | null; onChange: (effort: ReasoningEffort | null) => void }) {
  const { t } = useI18n();
  const stops: (ReasoningEffort | null)[] = [null, ...efforts];
  const index = Math.max(0, stops.indexOf(value));
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowUp" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowDown" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = stops[Math.min(stops.length - 1, Math.max(0, index + step))];
    if (next !== undefined) onChange(next);
  };
  return (
    <div className="angelica-effort">
      <div className="angelica-effort-head">
        <span>{t("angelica.effort")}</span>
        <strong>{value ? t(effortLabels[value]) : t("ai.effort.default")}</strong>
      </div>
      <div className="angelica-effort-scale"><span>{t("angelica.effortFaster")}</span><span>{t("angelica.effortSmarter")}</span></div>
      <div className="angelica-effort-track" role="radiogroup" aria-label={t("angelica.effort")} tabIndex={0} onKeyDown={move}>
        <span className="angelica-effort-fill" style={{ width: `${stops.length > 1 ? (index / (stops.length - 1)) * 100 : 0}%` }} />
        {stops.map((stop, position) => (
          <button
            key={stop ?? "default"}
            type="button"
            role="radio"
            tabIndex={-1}
            aria-checked={position === index}
            className={position === index ? "angelica-effort-stop current" : "angelica-effort-stop"}
            style={{ left: `${stops.length > 1 ? (position / (stops.length - 1)) * 100 : 0}%` }}
            title={stop ? t(effortLabels[stop]) : t("ai.effort.default")}
            onClick={() => onChange(stop)}
          />
        ))}
      </div>
    </div>
  );
}

/** Model and effort for the next message, in one popover. */
export function ModelMenu({ providers, model, onChange, onOpen }: { providers: readonly AiProviderDto[]; model: AiModelSelection | null; onChange: (model: AiModelSelection | null) => void; onOpen: () => void }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const efforts = selectableEfforts([...providers], model);
  const withModels = providers.filter((provider) => provider.models.length > 0);
  const current = model ? selectionKey(model) : "";
  return (
    <Popover.Root open={open} onOpenChange={(next) => { setOpen(next); if (next) onOpen(); }}>
      <Popover.Trigger asChild>
        <button className="angelica-control angelica-model" type="button" title={t("angelica.model")}>
          <span className="angelica-model-name">{model?.modelId ?? t("angelica.noModel")}</span>
          {model?.effort ? <span className="angelica-model-effort">{t(effortLabels[model.effort])}</span> : null}
          <UiIcon icon="chevronDown" size="xs" />
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="menu-content angelica-menu angelica-model-menu" side="top" align="end" sideOffset={6} collisionPadding={8}>
          <div className="angelica-model-list" role="listbox" aria-label={t("angelica.model")}>
            {withModels.map((provider) => (
              <div key={provider.id} role="group" aria-label={provider.name}>
                {withModels.length > 1 ? <div className="menu-label angelica-menu-label">{provider.name}</div> : <div className="menu-label angelica-menu-label">{t("angelica.model")}</div>}
                {provider.models.map((entry) => {
                  const key = selectionKey({ providerId: provider.id, modelId: entry.id });
                  return (
                    <button
                      key={entry.id}
                      type="button"
                      role="option"
                      aria-selected={key === current}
                      className="menu-item angelica-menu-item"
                      onClick={() => {
                        const next = parseSelectionKey(key);
                        onChange(next ? resolveModel([...providers], [{ ...next, effort: model?.effort ?? null }]) : null);
                      }}
                    >
                      <span className="menu-item-check">{key === current ? <UiIcon icon="check" size="xs" /> : null}</span>
                      <span className="menu-item-label">{entry.id}</span>
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
          {efforts.length > 0 && model ? (
            <>
              <div className="menu-separator" />
              <EffortSlider efforts={efforts} value={model.effort} onChange={(effort) => onChange({ ...model, effort })} />
            </>
          ) : null}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
