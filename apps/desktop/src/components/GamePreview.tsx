import { useState, type CSSProperties, type ReactNode } from "react";
import { choiceLabels, valueLabel } from "../macroLabels";
import type { PreviewPieceDto, PreviewStyleDto } from "../types";
import { useI18n } from "../ui/i18n";

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

type Choices = { selected: ReadonlyMap<string, number>; select: (path: string, branch: number) => void };

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
      return <span className={`preview-value preview-value-${piece.valueKind}`} style={textStyle(piece.style)} title={piece.label}>{valueLabel(piece, t)}</span>;
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
      const selected = Math.min(choices.selected.get(path) ?? 0, count - 1);
      const labels = choiceLabels(piece.choice, count, t);
      const branch = piece.branches[selected] ?? [];
      return (
        <button
          type="button"
          className={`preview-choice${count > 1 ? " is-switchable" : ""}`}
          title={t("preview.choice.cycle", { label: labels[selected] ?? "", index: selected + 1, count })}
          onClick={(event) => {
            event.stopPropagation();
            choices.select(path, (selected + 1) % count);
          }}
        >
          {branch.length > 0 ? <Pieces pieces={branch} path={`${path}.${selected}`} choices={choices} /> : <span className="preview-empty-branch">∅</span>}
          {count > 1 ? <sup className="preview-choice-index">{selected + 1}/{count}</sup> : null}
        </button>
      );
    }
  }
}

/**
 * A string as the game shows it: colors, outlines, italics, line breaks,
 * icons, and runtime values. Conditions show one branch at a time; clicking
 * one shows the next.
 */
export function GamePreview({ pieces, className }: { pieces: readonly PreviewPieceDto[]; className?: string }) {
  const { t } = useI18n();
  const [selected, setSelected] = useState<ReadonlyMap<string, number>>(new Map());
  const choices: Choices = {
    selected,
    select: (path, branch) => setSelected((current) => new Map(current).set(path, branch)),
  };
  return (
    <div className={`game-preview${className ? ` ${className}` : ""}`} aria-label={t("preview.title")}>
      {pieces.length > 0 ? <Pieces pieces={pieces} path="root" choices={choices} /> : <span className="preview-nothing">{t("preview.empty")}</span>}
    </div>
  );
}
