import { EditorSelection, EditorState, StateField, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, WidgetType, type DecorationSet } from "@codemirror/view";
import type { Translate } from "../i18n/translate";
import { chipSpecs, type ChipSpec, type ChipTone } from "../macroChips";
import { loadIcon } from "../ui/gameGlyphs";

/**
 * The text editor's chip view: tags drawn as compact atomic chips,
 * formatting pairs as styled text between thin markers, and `<br>` as a
 * line break. The document is the macro text itself, so copying, undo, and
 * saving see exactly what Rust validates.
 */

/** Colors of opening color tags by their source text, learned from Rust views. */
const tagColors = new Map<string, string>();

/** Remembers the colors of a Rust view's opening color tags. */
export function learnTagColors(text: string, tags: readonly { from: number; to: number; color: string | null }[]): boolean {
  let learned = false;
  for (const tag of tags) {
    if (!tag.color) continue;
    const source = text.slice(tag.from, tag.to);
    if (tagColors.get(source) !== tag.color) {
      tagColors.set(source, tag.color);
      learned = true;
    }
  }
  return learned;
}

/** Converts `#rrggbbaa` to a CSS color. */
function cssColor(rgba: string): string {
  return rgba.length === 9 && rgba.endsWith("ff") ? rgba.slice(0, 7) : rgba;
}

class ChipWidget extends WidgetType {
  constructor(readonly label: string, readonly tone: ChipTone, readonly icon: number | undefined, readonly error: boolean) {
    super();
  }

  override eq(other: ChipWidget): boolean {
    return other.label === this.label && other.tone === this.tone && other.icon === this.icon && other.error === this.error;
  }

  toDOM(): HTMLElement {
    const chip = document.createElement("span");
    chip.className = `cm-chip cm-chip-${this.tone}${this.error ? " is-error" : ""}`;
    chip.textContent = this.label;
    if (this.icon !== undefined) {
      const icon = this.icon;
      void loadIcon(icon).then((url) => {
        if (!url) return;
        const image = document.createElement("img");
        image.src = url;
        image.alt = String(icon);
        image.draggable = false;
        chip.replaceChildren(image);
        chip.classList.add("has-image");
      });
    }
    return chip;
  }

  override ignoreEvent(): boolean {
    return false;
  }
}

/** Ends the visual line after a `<br>` chip, so the cursor after it sits on the next line. */
class LineEndWidget extends WidgetType {
  override eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    return document.createElement("br");
  }
}

const lineEnd = Decoration.widget({ widget: new LineEndWidget(), side: -1 });

class MarkerWidget extends WidgetType {
  constructor(readonly side: "open" | "close", readonly color: string | null, readonly error: boolean) {
    super();
  }

  override eq(other: MarkerWidget): boolean {
    return other.side === this.side && other.color === this.color && other.error === this.error;
  }

  toDOM(): HTMLElement {
    const marker = document.createElement("span");
    marker.className = `cm-chip-marker is-${this.side}${this.error ? " is-error" : ""}`;
    if (this.color) marker.style.setProperty("--marker-color", cssColor(this.color));
    return marker;
  }

  override ignoreEvent(): boolean {
    return false;
  }
}

export type ChipContext = {
  t: Translate;
  /** The text the error ranges describe; errors apply only while it is the document. */
  text: string | null;
  errors: readonly (readonly [number, number])[];
  /** Bumps when new tag colors are learned, so the chips are redrawn. */
  version: number;
};

type Chips = { decorations: DecorationSet; atoms: DecorationSet };

function build(doc: string, context: ChipContext): Chips {
  const errors = context.text === doc ? context.errors : [];
  const hasError = (spec: ChipSpec) => errors.some(([from, to]) => from < spec.to && to > spec.from);
  const ranges: Range<Decoration>[] = [];
  const atoms: Range<Decoration>[] = [];
  for (const spec of chipSpecs(doc, (tag) => tagColors.get(tag) ?? null, context.t)) {
    if (spec.kind === "style") {
      const css = [
        spec.color ? `color: ${cssColor(spec.color)}` : "",
        spec.italic ? "font-style: italic" : "",
        spec.bold ? "font-weight: 700" : "",
      ].filter(Boolean).join("; ");
      ranges.push(Decoration.mark({ attributes: { style: css } }).range(spec.from, spec.to));
      continue;
    }
    const widget = spec.kind === "chip"
      ? new ChipWidget(spec.label, spec.tone, spec.icon, hasError(spec))
      : new MarkerWidget(spec.side, spec.color, hasError(spec));
    const range = Decoration.replace({ widget }).range(spec.from, spec.to);
    ranges.push(range);
    atoms.push(range);
    if (spec.kind === "chip" && spec.tone === "break") ranges.push(lineEnd.range(spec.to));
  }
  return { decorations: Decoration.set(ranges, true), atoms: Decoration.set(atoms, true) };
}

/**
 * A cursor right after `<br>` belongs to the next line: it is drawn there,
 * where typing puts text, not at the end of the line the break ends.
 */
const cursorAfterBreak = EditorState.transactionFilter.of((transaction) => {
  const selection = transaction.selection;
  if (!selection) return transaction;
  const doc = transaction.newDoc;
  let changed = false;
  const ranges = selection.ranges.map((range) => {
    if (!range.empty || range.assoc === 1 || range.head < 4 || doc.sliceString(range.head - 4, range.head) !== "<br>") return range;
    changed = true;
    return EditorSelection.cursor(range.head, 1);
  });
  if (!changed) return transaction;
  return [transaction, { selection: EditorSelection.create(ranges, selection.mainIndex), sequential: true }];
});

/** The chip view for `context`; reconfigure it when the context changes. */
export function macroChips(context: ChipContext): Extension {
  const field = StateField.define<Chips>({
    create: (state) => build(state.doc.toString(), context),
    update: (value, transaction) => (transaction.docChanged ? build(transaction.state.doc.toString(), context) : value),
    provide: (chips) => [
      EditorView.decorations.from(chips, (value) => value.decorations),
      EditorView.atomicRanges.of((view) => view.state.field(chips).atoms),
    ],
  });
  return [field, cursorAfterBreak];
}
