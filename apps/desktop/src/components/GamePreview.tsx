import { useState, type CSSProperties, type ReactNode } from "react";
import { choiceLabels, parameterCode, valueHint, valueLabel } from "../macroLabels";
import type { PreviewPieceDto, PreviewStyleDto } from "../types";
import { useGameIcon } from "../ui/gameGlyphs";
import { useI18n } from "../ui/i18n";
import { focusPreviewVariable } from "./PreviewVariables";

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

/** Branches clicked through, for conditions the variables cannot decide. */
type Clicked = {
  selected: ReadonlyMap<string, number>;
  select: (path: string, branch: number) => void;
};

function Pieces({ pieces, path, clicked }: { pieces: readonly PreviewPieceDto[]; path: string; clicked: Clicked }) {
  return <>{pieces.map((piece, index) => <Piece key={index} piece={piece} path={`${path}.${index}`} clicked={clicked} />)}</>;
}

function Piece({ piece, path, clicked }: { piece: PreviewPieceDto; path: string; clicked: Clicked }): ReactNode {
  const { t } = useI18n();
  switch (piece.kind) {
    case "text":
      return <span style={textStyle(piece.style)}>{piece.text}</span>;
    case "break":
      return <br />;
    case "value": {
      const parameter = piece.parameter;
      if (piece.shown.length > 0) {
        // A value filled in from the variables: shown as the game shows it,
        // marked so it reads as a variable, and a click goes to its control.
        return (
          <span
            className={`preview-filled${parameter ? " is-linked" : ""}`}
            title={`${valueHint(piece, t)}${parameter ? ` · ${t("preview.var.pick")}` : ""}`}
            onClick={parameter ? () => focusPreviewVariable(parameterCode(parameter).slice(1)) : undefined}
          >
            <Pieces pieces={piece.shown} path={`${path}.shown`} clicked={clicked} />
          </span>
        );
      }
      return (
        <span
          className={`preview-value preview-value-${piece.valueKind}${parameter && piece.valueKind !== "playerName" ? " is-code" : ""}${parameter ? " is-linked" : ""}`}
          style={textStyle(piece.style)}
          title={valueHint(piece, t)}
          onClick={parameter ? () => focusPreviewVariable(parameterCode(parameter).slice(1)) : undefined}
        >
          {valueLabel(piece, t)}
        </span>
      );
    }
    case "icon":
      return <GameIcon icon={piece.icon} />;
    case "opaque":
      return <span className="preview-opaque">{piece.spelling}</span>;
    case "ruby":
      return (
        <ruby>
          <Pieces pieces={piece.base} path={`${path}.base`} clicked={clicked} />
          <rt><Pieces pieces={piece.reading} path={`${path}.reading`} clicked={clicked} /></rt>
        </ruby>
      );
    case "choice": {
      const count = piece.branches.length;
      if (count === 0) return null;
      const labels = choiceLabels(piece.choice, count, t);
      if (piece.selected !== null) {
        // As in the game: the branch the variables select.
        const branch = piece.branches[piece.selected] ?? [];
        if (branch.length === 0) return null;
        return (
          <span className="preview-cond" title={t("preview.choice.shown", { label: labels[piece.selected] ?? "" })}>
            <Pieces pieces={branch} path={`${path}.${piece.selected}`} clicked={clicked} />
          </span>
        );
      }
      const selected = Math.min(clicked.selected.get(path) ?? 0, count - 1);
      const branch = piece.branches[selected] ?? [];
      const next = () => clicked.select(path, (selected + 1) % count);
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
          {branch.length > 0 ? <Pieces pieces={branch} path={`${path}.${selected}`} clicked={clicked} /> : <span className="preview-empty-branch">∅</span>}
          {count > 1 ? <sup className="preview-choice-index">{selected + 1}/{count}</sup> : null}
        </span>
      );
    }
  }
}

/** An inline game icon as the game draws it; its number until the image is read. */
function GameIcon({ icon }: { icon: number }) {
  const { t } = useI18n();
  const url = useGameIcon(icon);
  const title = t("preview.icon", { icon });
  return url
    ? <img className="preview-icon-image" src={url} alt={title} title={title} draggable={false} />
    : <span className="preview-icon" title={title}>{icon}</span>;
}

/**
 * A string as the game shows it: colors, outlines, italics, line breaks,
 * icons, and values. Rust evaluates it with the preview variables, so
 * conditions show the branch the variables select and values are filled
 * in; the variables are edited in {@link PreviewVariables}.
 */
export function GamePreview({ pieces, className }: { pieces: readonly PreviewPieceDto[]; className?: string }) {
  const { t } = useI18n();
  // Branches clicked through belong to one string: new pieces start at the first.
  const [clicked, setClicked] = useState<{ pieces: readonly PreviewPieceDto[]; selected: ReadonlyMap<string, number> }>({ pieces, selected: new Map() });
  const state: Clicked = {
    selected: clicked.pieces === pieces ? clicked.selected : new Map(),
    select: (path, branch) => setClicked((current) => ({
      pieces,
      selected: new Map(current.pieces === pieces ? current.selected : []).set(path, branch),
    })),
  };
  return (
    <div className={`game-preview${className ? ` ${className}` : ""}`} aria-label={t("preview.title")}>
      {pieces.length > 0 ? <Pieces pieces={pieces} path="root" clicked={state} /> : <span className="preview-nothing">{t("preview.empty")}</span>}
    </div>
  );
}
