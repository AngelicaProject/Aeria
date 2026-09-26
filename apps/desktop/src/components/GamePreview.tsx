import { useMemo, useState, type CSSProperties, type ReactNode } from "react";
import type { MessageKey } from "../i18n/translate";
import { choiceLabels, valueHint, valueLabel } from "../macroLabels";
import { chosenBranch, conditionVariables, effectiveValues, type ConditionValues, type ConditionVariable } from "../previewConditions";
import type { PreviewPieceDto, PreviewStyleDto } from "../types";
import { resetConditionValues, setConditionValue, useConditionValues } from "../ui/conditionValues";
import { useI18n } from "../ui/i18n";
import { Select } from "../ui/primitives/Select";

/** Converts `#rrggbbaa` to a CSS color. */
function cssColor(rgba: string): string {
  return rgba.length === 9 && rgba.endsWith("ff") ? rgba.slice(0, 7) : rgba;
}

function textStyle(style: PreviewStyleDto): CSSProperties | undefined {
  const css: CSSProperties = {};
  if (style.color) css.color = cssColor(style.color);
  if (style.edge) {
    const edge = cssColor(style.edge);
    css.textShadow = `0 0 2px ${edge}, 0 0 2px ${edge}, 0 0 1px ${edge}`;
  }
  if (style.italic) css.fontStyle = "italic";
  if (style.bold) css.fontWeight = 700;
  return Object.keys(css).length > 0 ? css : undefined;
}

/**
 * How choices are resolved: from the condition values when the preview can
 * evaluate them, and otherwise by clicking through the branches.
 */
type Choices = {
  values: ConditionValues;
  selected: ReadonlyMap<string, number>;
  select: (path: string, branch: number) => void;
};

function Pieces({ pieces, path, choices }: { pieces: readonly PreviewPieceDto[]; path: string; choices: Choices }) {
  return <>{pieces.map((piece, index) => <Piece key={index} piece={piece} path={`${path}.${index}`} choices={choices} />)}</>;
}

function Piece({ piece, path, choices }: { piece: PreviewPieceDto; path: string; choices: Choices }): ReactNode {
  const { t } = useI18n();
  switch (piece.kind) {
    case "text":
      return <span style={textStyle(piece.style)}>{piece.text}</span>;
    case "break":
      return <br />;
    case "value":
      return <span className={`preview-value preview-value-${piece.valueKind}${piece.parameter && piece.valueKind !== "playerName" ? " is-code" : ""}`} style={textStyle(piece.style)} title={valueHint(piece, t)}>{valueLabel(piece, t)}</span>;
    case "icon":
      return <span className="preview-icon" title={t("preview.icon", { icon: piece.icon })}>{piece.icon}</span>;
    case "opaque":
      return <span className="preview-opaque">{piece.spelling}</span>;
    case "ruby":
      return (
        <ruby>
          <Pieces pieces={piece.base} path={`${path}.base`} choices={choices} />
          <rt><Pieces pieces={piece.reading} path={`${path}.reading`} choices={choices} /></rt>
        </ruby>
      );
    case "choice": {
      const count = piece.branches.length;
      if (count === 0) return null;
      const labels = choiceLabels(piece.choice, count, t);
      const evaluated = chosenBranch(piece.choice, count, choices.values);
      if (evaluated !== null) {
        // As in the game: the branch the values select, or nothing.
        const branch = evaluated >= 0 ? piece.branches[evaluated] ?? [] : [];
        if (branch.length === 0) return null;
        return (
          <span className="preview-cond" title={t("preview.choice.shown", { label: labels[evaluated] ?? "" })}>
            <Pieces pieces={branch} path={`${path}.${evaluated}`} choices={choices} />
          </span>
        );
      }
      const selected = Math.min(choices.selected.get(path) ?? 0, count - 1);
      const branch = piece.branches[selected] ?? [];
      const next = () => choices.select(path, (selected + 1) % count);
      return (
        // An inline span, not a button: a branch may hold line breaks, and a
        // button would turn it into a block.
        <span
          role="button"
          tabIndex={0}
          className={`preview-choice${count > 1 ? " is-switchable" : ""}`}
          title={t("preview.choice.cycle", { label: labels[selected] ?? "", index: selected + 1, count })}
          onClick={(event) => {
            event.stopPropagation();
            next();
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter" && event.key !== " ") return;
            event.preventDefault();
            next();
          }}
        >
          {branch.length > 0 ? <Pieces pieces={branch} path={`${path}.${selected}`} choices={choices} /> : <span className="preview-empty-branch">∅</span>}
          {count > 1 ? <sup className="preview-choice-index">{selected + 1}/{count}</sup> : null}
        </span>
      );
    }
  }
}

const characterOptions: Readonly<Record<string, readonly [MessageKey, MessageKey, MessageKey]>> = {
  gender: ["preview.var.gender", "preview.choice.male", "preview.choice.female"],
  self: ["preview.var.self", "preview.choice.self", "preview.choice.other"],
  name: ["preview.var.name", "preview.choice.nameMatches", "preview.choice.nameDiffers"],
  josa: ["preview.var.josa", "preview.choice.consonant", "preview.choice.vowel"],
};

function ConditionControl({ variable, value }: { variable: ConditionVariable; value: number }) {
  const { t } = useI18n();
  const character = characterOptions[variable.kind];
  if (character) {
    const [label, first, second] = character;
    // The first option shows the first branch; for "self" and "name" that is the value 1.
    const inverted = variable.kind === "self" || variable.kind === "name";
    const option = inverted ? (value === 1 ? 0 : 1) : (value === 0 ? 0 : 1);
    return (
      <span className="preview-var">
        {t(label)}
        <Select<"0" | "1">
          variant="quiet"
          label={t(label)}
          value={option === 0 ? "0" : "1"}
          onChange={(chosen) => {
            const picked = chosen === "1" ? 1 : 0;
            setConditionValue(variable.key, inverted ? 1 - picked : picked);
          }}
          options={[{ value: "0", label: t(first) }, { value: "1", label: t(second) }]}
        />
      </span>
    );
  }
  const prefix = variable.key.match(/^(gn|gs|n|s)\d+$/)?.[1];
  const hint = prefix
    ? t(`preview.paramHint.${prefix}` as MessageKey, { code: `$${variable.key}` })
    : t("preview.var.time", { name: t(`preview.time.${variable.key}` as MessageKey) });
  return (
    <label className="preview-var" title={hint}>
      <code>${variable.key}</code>
      <input
        type="number"
        min={0}
        value={value}
        onChange={(event) => {
          const next = Number.parseInt(event.target.value, 10);
          setConditionValue(variable.key, Number.isFinite(next) && next >= 0 ? next : null);
        }}
      />
    </label>
  );
}

/**
 * A string as the game shows it: colors, outlines, italics, line breaks,
 * icons, and runtime values. Conditions follow the values in the bar above
 * the text, shared by every string, so nested conditions resolve as they do
 * in the game.
 */
export function GamePreview({ pieces, className }: { pieces: readonly PreviewPieceDto[]; className?: string }) {
  const { t } = useI18n();
  const chosenValues = useConditionValues();
  const variables = useMemo(() => conditionVariables(pieces), [pieces]);
  const values = effectiveValues(variables, chosenValues);
  // Branches clicked through belong to one string: new pieces start at the first.
  const [clicked, setClicked] = useState<{ pieces: readonly PreviewPieceDto[]; selected: ReadonlyMap<string, number> }>({ pieces, selected: new Map() });
  const choices: Choices = {
    values,
    selected: clicked.pieces === pieces ? clicked.selected : new Map(),
    select: (path, branch) => setClicked((current) => ({
      pieces,
      selected: new Map(current.pieces === pieces ? current.selected : []).set(path, branch),
    })),
  };
  const changed = variables.some((variable) => chosenValues[variable.key] !== undefined);
  return (
    <div className={`game-preview-frame${className ? ` ${className}` : ""}`}>
      {variables.length > 0 ? (
        <div className="preview-conditions">
          <span className="preview-conditions-label">{t("preview.conditions")}</span>
          {variables.map((variable) => <ConditionControl key={variable.key} variable={variable} value={values[variable.key] ?? variable.fallback} />)}
          {changed ? (
            <button className="preview-conditions-reset" type="button" title={t("preview.conditions.resetHint")} onClick={() => resetConditionValues(variables.map((variable) => variable.key))}>
              {t("preview.conditions.reset")}
            </button>
          ) : null}
        </div>
      ) : null}
      <div className="game-preview" aria-label={t("preview.title")}>
        {pieces.length > 0 ? <Pieces pieces={pieces} path="root" choices={choices} /> : <span className="preview-nothing">{t("preview.empty")}</span>}
      </div>
    </div>
  );
}
