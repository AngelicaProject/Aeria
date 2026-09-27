import { EditorSelection, EditorState, StateField, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, WidgetType, type DecorationSet } from "@codemirror/view";
import type { Translate } from "../i18n/translate";
import { chipSpecs, type ChipSpec, type ChipTone } from "../macroChips";
import type { MacroConditionDto, MacroTagDto } from "../types";
import { loadIcon } from "../ui/gameGlyphs";

/**
 * The text editor's chip view: tags drawn as compact atomic chips,
 * conditions in words, formatting pairs as styled text between thin
 * markers, and line breaks for `<br>` and the branches of complex
 * conditions. The document is the macro text itself, so copying, undo, and
 * saving see exactly what Rust validates.
 */

/** Colors of opening color tags by their source text, learned from Rust views. */
const tagColors = new Map<string, string>();
/** Conditions of opening `<if>` and `<switch>` tags by their source text. */
const tagConditions = new Map<string, MacroConditionDto>();

/** Remembers what a Rust view says about tags; true when anything new was learned. */
export function learnTags(text: string, tags: readonly MacroTagDto[]): boolean {
  let learned = false;
  for (const tag of tags) {
    const source = text.slice(tag.from, tag.to);
    if (tag.color && tagColors.get(source) !== tag.color) {
      tagColors.set(source, tag.color);
      learned = true;
    }
    if (tag.condition && !tagConditions.has(source)) {
      tagConditions.set(source, tag.condition);
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
  constructor(readonly label: string, readonly tone: ChipTone, readonly icon: number | undefined, readonly error: boolean, readonly title: string | undefined) {
    super();
  }

  override eq(other: ChipWidget): boolean {
    return other.label === this.label && other.tone === this.tone && other.icon === this.icon && other.error === this.error && other.title === this.title;
  }

  toDOM(): HTMLElement {
    const chip = document.createElement("span");
    chip.className = `cm-chip cm-chip-${this.tone}${this.error ? " is-error" : ""}`;
    chip.textContent = this.label;
    if (this.title) chip.title = this.title;
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

/** Ends a visual line after a `<br>` chip. */
class BreakWidget extends WidgetType {
  override eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    const wrapper = document.createElement("span");
    wrapper.className = "cm-chip-break-line";
    // An empty box on the new line, so a cursor after the break is drawn
    // there even at the end of the text.
    const line = document.createElement("span");
    line.className = "cm-chip-line-start";
    wrapper.append(document.createElement("br"), line);
    return wrapper;
  }
}

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

/** What picking a chip of the source adds to the translation. */
export type ChipPick = { insert: string } | { wrap: readonly [string, string] };

export type ChipContext = {
  t: Translate;
  /** The text the error ranges describe; errors apply only while it is the document. */
  text: string | null;
  errors: readonly (readonly [number, number])[];
  /** Bumps when new tag facts are learned, so the chips are redrawn. */
  version: number;
  /** Called when a chip is clicked, in an editor whose chips can be picked. */
  onPick?: ((pick: ChipPick) => void) | undefined;
};

type Chips = {
  decorations: DecorationSet;
  atoms: DecorationSet;
  specs: readonly ChipSpec[];
  /** Positions right after a line break, where a cursor belongs on the new line. */
  breaks: ReadonlySet<number>;
};

const lookup = {
  colorOf: (tag: string) => tagColors.get(tag) ?? null,
  conditionOf: (tag: string) => tagConditions.get(tag) ?? null,
};

function build(doc: string, context: ChipContext): Chips {
  const errors = context.text === doc ? context.errors : [];
  const hasError = (from: number, to: number) => errors.some(([start, end]) => start < to && end > from);
  const ranges: Range<Decoration>[] = [];
  const atoms: Range<Decoration>[] = [];
  const breaks = new Set<number>();
  const specs = chipSpecs(doc, lookup, context.t);
  for (const spec of specs) {
    switch (spec.kind) {
      case "style": {
        const css = [
          spec.color ? `color: ${cssColor(spec.color)}` : "",
          spec.italic ? "font-style: italic" : "",
          spec.bold ? "font-weight: 700" : "",
        ].filter(Boolean).join("; ");
        ranges.push(Decoration.mark({ attributes: { style: css } }).range(spec.from, spec.to));
        break;
      }
      case "break":
        ranges.push(Decoration.widget({ widget: new BreakWidget(), side: -1 }).range(spec.at));
        breaks.add(spec.at);
        break;
      default: {
        const widget = spec.kind === "chip"
          ? new ChipWidget(spec.label, spec.tone, spec.icon, hasError(spec.from, spec.to), spec.title)
          : new MarkerWidget(spec.side, spec.color, hasError(spec.from, spec.to));
        const range = Decoration.replace({ widget }).range(spec.from, spec.to);
        ranges.push(range);
        atoms.push(range);
      }
    }
  }
  return { decorations: Decoration.set(ranges, true), atoms: Decoration.set(atoms, true), specs, breaks };
}

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

  // A cursor right after a `<br>` sits on the new line, where typing puts text.
  const cursorAtBreaks = EditorState.transactionFilter.of((transaction) => {
    const selection = transaction.selection;
    if (!selection) return transaction;
    const breaks = transaction.state.field(field).breaks;
    let changed = false;
    const ranges = selection.ranges.map((range) => {
      if (!range.empty || range.assoc === 1 || !breaks.has(range.head)) return range;
      changed = true;
      return EditorSelection.cursor(range.head, 1);
    });
    if (!changed) return transaction;
    return [transaction, { selection: EditorSelection.create(ranges, selection.mainIndex), sequential: true }];
  });

  const picking = context.onPick
    ? [
      EditorView.editorAttributes.of({ class: "is-pickable" }),
      EditorView.domEventHandlers({
        mousedown(event, view) {
          const element = (event.target as HTMLElement | null)?.closest(".cm-chip, .cm-chip-marker");
          if (!element) return false;
          const position = view.posAtDOM(element);
          const spec = view.state.field(field).specs.find((candidate) => candidate.kind !== "style" && candidate.kind !== "break" && candidate.from <= position && position < candidate.to);
          if (!spec) return false;
          if (spec.kind === "chip") context.onPick?.({ insert: spec.insert });
          else if (spec.kind === "marker" && spec.wrap) context.onPick?.({ wrap: spec.wrap });
          else return false;
          event.preventDefault();
          return true;
        },
      }),
    ]
    : [];
  return [field, cursorAtBreaks, picking];
}
