import { useEffect, useRef } from "react";
import { Annotation, Compartment, EditorState, Prec, StateEffect, StateField, type Extension, type Range } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  drawSelection,
  highlightSpecialChars,
  hoverTooltip,
  keymap,
  placeholder as placeholderExtension,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { describeTag, tagAt } from "../macroLabels";
import { scanMacros } from "../macroTokens";
import type { Translate } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import type { MacroViewState } from "../ui/useMacroView";
import { learnTags, macroChips, type ChipContext, type ChipPick } from "./macroChipsExtension";

/** How the editor draws tags: as chips and styled text, or as the macro text itself. */
export type MacroPresentation = "chips" | "code";

/** Edits another component can make in an editor. */
export type MacroEditorApi = {
  /** Replaces the selection with `text`, or wraps it in a pair of tags, and focuses the editor. */
  apply: (pick: ChipPick) => void;
};

type MacroEditorProps = {
  value: string;
  ariaLabel: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  disabled?: boolean;
  placeholder?: string;
  className?: string;
  onSave?: () => void;
  onSaveAndNext?: () => void;
  onApproveAndNext?: () => void;
  onNavigate?: (direction: 1 | -1) => void;
  /** The Rust description of the text, for hovers and error marks. */
  view?: MacroViewState | null;
  presentation?: MacroPresentation;
  /** Called when a chip is clicked; chips of an editor with this handler can be picked. */
  onPick?: ((pick: ChipPick) => void) | undefined;
  /** Receives the editor's API while it is mounted. */
  apiRef?: { current: MacroEditorApi | null };
};

const externalChange = Annotation.define<boolean>();
const macroMark = Decoration.mark({ class: "cm-macro" });
const macroNameMark = Decoration.mark({ class: "cm-macro-name" });

function macroDecorations(view: EditorView): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  for (const span of scanMacros(view.state.doc.toString())) {
    ranges.push(macroMark.range(span.from, span.to));
    for (const [from, to] of span.names) ranges.push(macroNameMark.range(from, to));
  }
  return Decoration.set(ranges, true);
}

const setDiagnostics = StateEffect.define<DecorationSet>();

/** Error marks from the latest Rust view, mapped through later edits. */
const diagnosticsField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    let next = value.map(transaction.changes);
    for (const effect of transaction.effects) if (effect.is(setDiagnostics)) next = effect.value;
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

function diagnosticMarks(state: MacroViewState, length: number): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  for (const diagnostic of state.view.diagnostics) {
    let from = Math.min(diagnostic.from, length);
    let to = Math.min(diagnostic.to, length);
    if (from === to) {
      if (length === 0) continue;
      from = Math.max(0, from - 1);
      to = Math.max(to, from + 1);
    }
    ranges.push(Decoration.mark({ class: "cm-macro-error", attributes: { title: diagnostic.message } }).range(from, to));
  }
  return Decoration.set(ranges, true);
}

function hoverDom(title: string, lines: readonly string[], error: boolean): HTMLElement {
  const dom = document.createElement("div");
  dom.className = `macro-hover${error ? " is-error" : ""}`;
  const heading = document.createElement("div");
  heading.className = "macro-hover-title";
  heading.textContent = title;
  dom.append(heading);
  for (const line of lines) {
    const row = document.createElement("div");
    row.textContent = line;
    dom.append(row);
  }
  return dom;
}

/** Describes the tag or error under the pointer, from the latest Rust view. */
function macroHover(source: { current: { view: MacroViewState | null; t: Translate } }): Extension {
  return hoverTooltip((editor, position) => {
    const { view, t } = source.current;
    if (!view || view.text !== editor.state.doc.toString()) return null;
    const diagnostic = view.view.diagnostics.find((candidate) => candidate.from <= position && position <= Math.max(candidate.to, candidate.from + 1));
    if (diagnostic) {
      return { pos: diagnostic.from, end: Math.max(diagnostic.to, diagnostic.from), above: true, create: () => ({ dom: hoverDom("⚠", [diagnostic.message], true) }) };
    }
    const tag = tagAt(view.view.tags, position);
    if (!tag) return null;
    const { title, lines } = describeTag(tag, t);
    return { pos: tag.from, end: tag.to, above: true, create: () => ({ dom: hoverDom(title, lines, false) }) };
  });
}

/** Presentation-only highlighting; Rust validates macro structure on save. */
const macroHighlighter = ViewPlugin.fromClass(class {
  decorations: DecorationSet;
  constructor(view: EditorView) {
    this.decorations = macroDecorations(view);
  }
  update(update: ViewUpdate) {
    if (update.docChanged) this.decorations = macroDecorations(update.view);
  }
}, { decorations: (plugin) => plugin.decorations });

/** Enter adds a line break as the game writes it. */
function insertLineBreak(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  view.dispatch(view.state.replaceSelection("<br>"), { scrollIntoView: true, userEvent: "input" });
  return true;
}

function chipContext(macroView: MacroViewState | null | undefined, t: Translate, version: number, onPick: ((pick: ChipPick) => void) | undefined): ChipContext {
  return {
    t,
    onPick,
    text: macroView?.text ?? null,
    errors: macroView?.view.diagnostics.map((diagnostic) => [diagnostic.from, Math.max(diagnostic.to, diagnostic.from + 1)] as const) ?? [],
    version,
  };
}

function presentationExtensions(presentation: MacroPresentation, highlight: boolean, context: ChipContext): Extension {
  if (presentation === "chips") return macroChips(context);
  return highlight ? macroHighlighter : [];
}

export function MacroEditor({ value, ariaLabel, onChange, readOnly = false, disabled = false, placeholder = "", className, onSave, onSaveAndNext, onApproveAndNext, onNavigate, view: macroView, presentation = "code", onPick, apiRef }: MacroEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const { t } = useI18n();
  const handlers = useRef({ onChange, onSave, onSaveAndNext, onApproveAndNext, onNavigate });
  handlers.current = { onChange, onSave, onSaveAndNext, onApproveAndNext, onNavigate };
  const hoverSource = useRef<{ view: MacroViewState | null; t: Translate }>({ view: macroView ?? null, t });
  hoverSource.current = { view: macroView ?? null, t };
  const compartments = useRef({ editable: new Compartment(), placeholder: new Compartment(), label: new Compartment(), highlight: new Compartment(), specialChars: new Compartment() });
  const { preferences } = usePreferences();
  const colorVersion = useRef(0);
  // Picks go through a ref so the chip view is not rebuilt for a new handler.
  const pickHandler = useRef(onPick);
  pickHandler.current = onPick;
  const pick = onPick ? (value: ChipPick) => pickHandler.current?.(value) : undefined;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const { editable, placeholder: placeholderCompartment, label, highlight, specialChars } = compartments.current;
    const commandKeys: Extension = Prec.highest(keymap.of([
      { key: "Mod-s", preventDefault: true, run: () => { handlers.current.onSave?.(); return Boolean(handlers.current.onSave); } },
      { key: "Mod-Shift-Enter", preventDefault: true, run: () => { handlers.current.onApproveAndNext?.(); return Boolean(handlers.current.onApproveAndNext); } },
      { key: "Mod-Enter", preventDefault: true, run: () => { handlers.current.onSaveAndNext?.(); return Boolean(handlers.current.onSaveAndNext); } },
      { key: "Alt-ArrowDown", preventDefault: true, run: () => { handlers.current.onNavigate?.(1); return Boolean(handlers.current.onNavigate); } },
      { key: "Alt-ArrowUp", preventDefault: true, run: () => { handlers.current.onNavigate?.(-1); return Boolean(handlers.current.onNavigate); } },
      { key: "Enter", preventDefault: true, run: insertLineBreak },
      { key: "Shift-Enter", preventDefault: true, run: insertLineBreak },
    ]));
    const view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          commandKeys,
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          drawSelection(),
          specialChars.of(preferences.showControlCharacters ? highlightSpecialChars() : []),
          EditorView.lineWrapping,
          diagnosticsField,
          macroHover(hoverSource),
          highlight.of(presentationExtensions(presentation, preferences.highlightMacros, chipContext(macroView, t, colorVersion.current, pick))),
          editable.of(editableExtensions(readOnly, disabled)),
          placeholderCompartment.of(placeholder ? placeholderExtension(placeholder) : []),
          label.of(EditorView.contentAttributes.of({ "aria-label": ariaLabel, spellcheck: "false" })),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged || update.transactions.some((transaction) => transaction.annotation(externalChange))) return;
            handlers.current.onChange?.(update.state.doc.toString());
          }),
        ],
      }),
    });
    viewRef.current = view;
    if (apiRef) {
      apiRef.current = {
        apply(picked) {
          if (view.state.readOnly) return;
          const range = view.state.selection.main;
          const insert = "insert" in picked
            ? picked.insert
            : `${picked.wrap[0]}${view.state.sliceDoc(range.from, range.to)}${picked.wrap[1]}`;
          const cursor = "insert" in picked || range.empty
            ? range.from + ("insert" in picked ? insert.length : picked.wrap[0].length)
            : range.from + insert.length;
          view.dispatch({ changes: { from: range.from, to: range.to, insert }, selection: { anchor: cursor }, scrollIntoView: true, userEvent: "input" });
          view.focus();
        },
      };
    }
    return () => {
      view.destroy();
      viewRef.current = null;
      if (apiRef) apiRef.current = null;
    };
    // The view is created once; later prop changes are applied by the effects below.
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === value) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: value },
      annotations: [externalChange.of(true)],
    });
  }, [value]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: compartments.current.editable.reconfigure(editableExtensions(readOnly, disabled)) });
  }, [disabled, readOnly]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    if (!macroView) {
      view.dispatch({ effects: setDiagnostics.of(Decoration.none) });
      return;
    }
    if (macroView.text !== view.state.doc.toString()) return;
    view.dispatch({ effects: setDiagnostics.of(diagnosticMarks(macroView, view.state.doc.length)) });
  }, [macroView]);

  useEffect(() => {
    if (macroView && learnTags(macroView.text, macroView.view.tags)) colorVersion.current += 1;
    viewRef.current?.dispatch({ effects: [
      compartments.current.highlight.reconfigure(presentationExtensions(presentation, preferences.highlightMacros, chipContext(macroView, t, colorVersion.current, pick))),
      compartments.current.specialChars.reconfigure(preferences.showControlCharacters ? highlightSpecialChars() : []),
    ] });
  }, [macroView, presentation, preferences.highlightMacros, preferences.showControlCharacters, t, Boolean(onPick)]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: compartments.current.placeholder.reconfigure(placeholder ? placeholderExtension(placeholder) : []) });
  }, [placeholder]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: compartments.current.label.reconfigure(EditorView.contentAttributes.of({ "aria-label": ariaLabel, spellcheck: "false" })) });
  }, [ariaLabel]);

  return <div ref={hostRef} className={`macro-editor is-${presentation}${readOnly ? " is-readonly" : ""}${disabled ? " is-disabled" : ""}${className ? ` ${className}` : ""}`} />;
}

function editableExtensions(readOnly: boolean, disabled: boolean): Extension {
  const locked = readOnly || disabled;
  return [EditorState.readOnly.of(locked), EditorView.editable.of(!disabled)];
}

/** Focuses the editor inside `host` once any pending remount has settled. */
export function focusMacroEditor(host: HTMLElement | null): void {
  window.setTimeout(() => host?.querySelector<HTMLElement>(".cm-content")?.focus(), 0);
}
