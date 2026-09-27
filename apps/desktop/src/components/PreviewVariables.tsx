import type { MessageKey } from "../i18n/translate";
import { variableCode, variableHint, variableLabel } from "../previewVariables";
import type { PreviewVariableDto } from "../types";
import { useI18n } from "../ui/i18n";
import { resetPreviewValues, setPreviewValue, usePreviewValues } from "../ui/previewValues";
import { Select } from "../ui/primitives/Select";

function controlId(key: string): string {
  return `preview-var-${key}`;
}

/** Moves focus to a variable's control, from a value in the preview. */
export function focusPreviewVariable(key: string) {
  const control = document.getElementById(controlId(key));
  if (!control) return;
  control.scrollIntoView({ block: "nearest" });
  control.focus();
}

/** Two-way choices: the label of value 0, then of value 1. */
const binaryOptions: Readonly<Record<string, readonly [MessageKey, MessageKey]>> = {
  "player-female": ["preview.choice.male", "preview.choice.female"],
  gender: ["preview.choice.male", "preview.choice.female"],
  self: ["preview.choice.other", "preview.choice.self"],
  name: ["preview.choice.nameDiffers", "preview.choice.nameMatches"],
  josa: ["preview.choice.consonant", "preview.choice.vowel"],
};

/** Whether the variable is chosen from a list rather than typed. */
function isChoice(variable: PreviewVariableDto): boolean {
  if (typeof variable.value !== "number") return false;
  return binaryOptions[variable.global ?? variable.key] !== undefined || variable.options.length > 0;
}

function Control({ variable }: { variable: PreviewVariableDto }) {
  const { t } = useI18n();
  const id = controlId(variable.key);
  const label = variableLabel(variable, t);
  const binary = binaryOptions[variable.global ?? variable.key];
  if (binary && typeof variable.value === "number") {
    return (
      <Select<"0" | "1">
        id={id}
        variant="quiet"
        label={label}
        value={variable.value === 0 ? "0" : "1"}
        onChange={(chosen) => setPreviewValue(variable.key, chosen === "1" ? 1 : 0)}
        options={[{ value: "0", label: t(binary[0]) }, { value: "1", label: t(binary[1]) }]}
      />
    );
  }
  if (variable.options.length > 0 && typeof variable.value === "number") {
    const current = String(variable.value);
    const options = variable.options.map((option) => ({ value: String(option.value), label: option.label }));
    if (!options.some((option) => option.value === current)) options.unshift({ value: current, label: variable.valueName ?? current });
    return (
      <Select<string>
        id={id}
        variant="quiet"
        label={label}
        value={current}
        onChange={(chosen) => setPreviewValue(variable.key, Number.parseInt(chosen, 10))}
        options={options}
      />
    );
  }
  if (variable.kind === "text" || typeof variable.value === "string") {
    return (
      <input
        id={id}
        className="preview-var-text"
        type="text"
        aria-label={label}
        placeholder={t("preview.var.empty")}
        value={typeof variable.value === "string" ? variable.value : ""}
        onChange={(event) => setPreviewValue(variable.key, event.target.value === "" ? null : event.target.value)}
      />
    );
  }
  return (
    <>
      <input
        id={id}
        type="number"
        min={0}
        aria-label={label}
        value={variable.value}
        onChange={(event) => {
          const next = Number.parseInt(event.target.value, 10);
          setPreviewValue(variable.key, Number.isFinite(next) && next >= 0 ? next : null);
        }}
      />
      {variable.valueName ? <span className="preview-var-name">{variable.valueName}</span> : null}
    </>
  );
}

/**
 * The variables the shown strings read, each once: the class, the level,
 * the player's name, string parameters, and so on. Their values apply to
 * every string, and Rust evaluates the previews with them.
 */
export function PreviewVariables({ variables }: { variables: readonly PreviewVariableDto[] }) {
  const { t } = useI18n();
  const chosen = usePreviewValues();
  if (variables.length === 0) return null;
  const changed = variables.some((variable) => chosen[variable.key] !== undefined);
  return (
    <div className="preview-variables" role="group" aria-label={t("preview.variables")}>
      <span className="preview-variables-label" title={t("preview.variables.hint")}>{t("preview.variables")}</span>
      {variables.map((variable) => {
        const code = variableCode(variable);
        const label = variableLabel(variable, t);
        const content = <>
          <span className={label === code ? "preview-var-code" : "preview-var-label"}>{label}</span>
          <Control variable={variable} />
        </>;
        // A list opens on its own trigger; a label around it would close it again.
        return isChoice(variable)
          ? <span key={variable.key} className="preview-var" title={variableHint(variable, t)}>{content}</span>
          : <label key={variable.key} className="preview-var" htmlFor={controlId(variable.key)} title={variableHint(variable, t)}>{content}</label>;
      })}
      {changed ? (
        <button className="preview-variables-reset" type="button" title={t("preview.variables.resetHint")} onClick={() => resetPreviewValues(variables.map((variable) => variable.key))}>
          {t("preview.variables.reset")}
        </button>
      ) : null}
    </div>
  );
}
