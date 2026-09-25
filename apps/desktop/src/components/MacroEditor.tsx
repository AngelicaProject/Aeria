import { useEffect, useRef } from "react";
import { Annotation, Compartment, EditorState, Prec, type Extension, type Range } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  drawSelection,
  highlightSpecialChars,
  keymap,
  placeholder as placeholderExtension,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { scanMacros } from "../macroTokens";
import { usePreferences } from "../ui/preferences";

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

export function MacroEditor({ value, ariaLabel, onChange, readOnly = false, disabled = false, placeholder = "", className, onSave, onSaveAndNext, onApproveAndNext, onNavigate }: MacroEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const handlers = useRef({ onChange, onSave, onSaveAndNext, onApproveAndNext, onNavigate });
  handlers.current = { onChange, onSave, onSaveAndNext, onApproveAndNext, onNavigate };
  const compartments = useRef({ editable: new Compartment(), placeholder: new Compartment(), label: new Compartment(), highlight: new Compartment(), specialChars: new Compartment() });
  const { preferences } = usePreferences();

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
          highlight.of(preferences.highlightMacros ? macroHighlighter : []),
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
    return () => {
      view.destroy();
      viewRef.current = null;
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
    viewRef.current?.dispatch({ effects: [
      compartments.current.highlight.reconfigure(preferences.highlightMacros ? macroHighlighter : []),
      compartments.current.specialChars.reconfigure(preferences.showControlCharacters ? highlightSpecialChars() : []),
    ] });
  }, [preferences.highlightMacros, preferences.showControlCharacters]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: compartments.current.placeholder.reconfigure(placeholder ? placeholderExtension(placeholder) : []) });
  }, [placeholder]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: compartments.current.label.reconfigure(EditorView.contentAttributes.of({ "aria-label": ariaLabel, spellcheck: "false" })) });
  }, [ariaLabel]);

  return <div ref={hostRef} className={`macro-editor${readOnly ? " is-readonly" : ""}${disabled ? " is-disabled" : ""}${className ? ` ${className}` : ""}`} />;
}

function editableExtensions(readOnly: boolean, disabled: boolean): Extension {
  const locked = readOnly || disabled;
  return [EditorState.readOnly.of(locked), EditorView.editable.of(!disabled)];
}

/** Focuses the editor inside `host` once any pending remount has settled. */
export function focusMacroEditor(host: HTMLElement | null): void {
  window.setTimeout(() => host?.querySelector<HTMLElement>(".cm-content")?.focus(), 0);
}
