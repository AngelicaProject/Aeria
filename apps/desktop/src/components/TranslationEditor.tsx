import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { bindingKey, domKey, rowKey } from "../binding";
import type { ReviewState, SourceBinding, TranslationCellDto, TranslationRowDto, UnitChangeKind } from "../types";
import { diffWords } from "../textDiff";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { MacroEditor, focusMacroEditor } from "./MacroEditor";
import { ReviewDot, reviewLabel } from "./ReviewDot";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

export type CellDraft = {
  target: string;
  note: string;
};

export type CellMutation = {
  kind: "target" | "note" | "review";
  bindingKey: string;
};

export type SaveTargetHandler = (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void, advance: boolean) => void;

/** The committed state of the selected string when it has uncommitted changes. */
export type CheckpointBaseline = {
  kind: UnitChangeKind;
  /** Target at the last checkpoint; null when the string is new since then. */
  target: string | null;
  reviewChanged: boolean;
  noteChanged: boolean;
};

function unchangedTextLabel(baseline: CheckpointBaseline): MessageKey {
  if (baseline.reviewChanged && baseline.noteChanged) return "editor.diff.reviewAndNoteChanged";
  if (baseline.reviewChanged) return "editor.diff.reviewChanged";
  if (baseline.noteChanged) return "editor.diff.noteChanged";
  return "editor.diff.same";
}

function CheckpointDiff({ baseline, current }: { baseline: CheckpointBaseline; current: string }) {
  const { t } = useI18n();
  if (baseline.target === null) {
    return <div className="checkpoint-diff"><span className="checkpoint-diff-label added">{t("editor.diff.new")}</span></div>;
  }
  const textChanged = baseline.target !== current;
  return (
    <div className="checkpoint-diff">
      <span className="checkpoint-diff-label">{t(textChanged ? "editor.diff.changed" : unchangedTextLabel(baseline))}</span>
      {textChanged ? (
        <div className="checkpoint-diff-text">
          {diffWords(baseline.target, current).map((part, index) => part.kind === "same"
            ? <span key={index}>{part.text}</span>
            : part.kind === "added" ? <ins key={index}>{part.text}</ins> : <del key={index}>{part.text}</del>)}
        </div>
      ) : null}
    </div>
  );
}

export type TranslationEditorHandle = {
  saveTarget: (advance: boolean) => void;
  revert: () => void;
  copySource: () => void;
};

type TranslationEditorProps = {
  row: TranslationRowDto | null;
  selectedBinding: SourceBinding | null;
  sourceLanguage: string;
  mutations: CellMutation[];
  onDirtyChange: (dirty: boolean) => void;
  onSelectCell: (binding: SourceBinding) => void;
  onSaveTarget: SaveTargetHandler;
  onSaveNote: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => void;
  onReviewChange: (cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => void;
  onNavigate: (direction: 1 | -1) => void;
  /** Returns true once when the target should take focus after navigation. */
  takeFocusRequest: () => boolean;
  checkpoint: CheckpointBaseline | null;
  /** Drafts a translation with Angelica; resolves to `null` when it failed. */
  onDraftWithAngelica?: ((cell: TranslationCellDto) => Promise<string | null>) | undefined;
};

const reviewOptions: readonly ReviewState[] = ["draft", "needsReview", "reviewed"];

function draftsForRow(row: TranslationRowDto | null): Record<string, CellDraft> {
  if (!row) return {};
  return Object.fromEntries(row.cells.map((cell) => [bindingKey(cell.sourceBinding), {
    target: cell.translation?.targetMacro ?? "",
    note: cell.translation?.translatorNote ?? "",
  }]));
}

function draftForCell(cell: TranslationCellDto, drafts: Record<string, CellDraft>): CellDraft {
  return drafts[bindingKey(cell.sourceBinding)] ?? {
    target: cell.translation?.targetMacro ?? "",
    note: cell.translation?.translatorNote ?? "",
  };
}

function cellIsDirty(cell: TranslationCellDto, draft: CellDraft): boolean {
  return draft.target !== (cell.translation?.targetMacro ?? "") ||
    (cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? ""));
}

function hasOtherDirtyDraft(row: TranslationRowDto, targetCell: TranslationCellDto, drafts: Record<string, CellDraft>, field: "target" | "note"): boolean {
  const targetKey = bindingKey(targetCell.sourceBinding);
  return row.cells.some((cell) => {
    const key = bindingKey(cell.sourceBinding);
    const draft = draftForCell(cell, drafts);
    if (key !== targetKey) return cellIsDirty(cell, draft);
    return field === "target"
      ? cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? "")
      : draft.target !== (cell.translation?.targetMacro ?? "");
  });
}

function Kbd({ keys }: { keys: string[] }) {
  return <span className="kbd-combo">{keys.map((key) => <kbd key={key}>{key}</kbd>)}</span>;
}

const TranslationEditorImpl = forwardRef<TranslationEditorHandle, TranslationEditorProps>(function TranslationEditor({
  row,
  selectedBinding,
  sourceLanguage,
  mutations,
  onDirtyChange,
  onSelectCell,
  onSaveTarget,
  onSaveNote,
  onReviewChange,
  onNavigate,
  takeFocusRequest,
  checkpoint,
  onDraftWithAngelica,
}, ref) {
  const { t } = useI18n();
  const [showDiff, setShowDiff] = useState(true);
  const [drafts, setDrafts] = useState<Record<string, CellDraft>>({});
  const draftsRef = useRef(drafts);
  const rowRef = useRef(row);
  const previousRowRef = useRef<TranslationRowDto | null>(null);
  const targetHostRef = useRef<HTMLDivElement>(null);
  draftsRef.current = drafts;
  rowRef.current = row;

  useEffect(() => {
    setDrafts((current) => {
      if (!row || !previousRowRef.current || rowKey(previousRowRef.current) !== rowKey(row)) return draftsForRow(row);
      const next = draftsForRow(row);
      for (const cell of row.cells) {
        const previousCell = previousRowRef.current.cells.find((candidate) => bindingKey(candidate.sourceBinding) === bindingKey(cell.sourceBinding));
        const currentDraft = current[bindingKey(cell.sourceBinding)];
        if (previousCell && currentDraft && cellIsDirty(previousCell, currentDraft)) next[bindingKey(cell.sourceBinding)] = currentDraft;
      }
      return next;
    });
    previousRowRef.current = row;
  }, [row]);

  const rowDirty = useMemo(() => row?.cells.some((cell) => cellIsDirty(cell, draftForCell(cell, drafts))) ?? false, [drafts, row]);
  useEffect(() => onDirtyChange(rowDirty), [onDirtyChange, rowDirty]);

  const selectedCell = row?.cells.find((cell) => selectedBinding && bindingKey(cell.sourceBinding) === bindingKey(selectedBinding)) ?? row?.cells[0] ?? null;
  const selectedKey = selectedCell ? bindingKey(selectedCell.sourceBinding) : null;
  const mutation = mutations.find((candidate) => candidate.bindingKey === selectedKey)?.kind ?? null;
  const draft = selectedCell ? draftForCell(selectedCell, drafts) : null;
  const targetDirty = selectedCell !== null && draft !== null && draft.target !== (selectedCell.translation?.targetMacro ?? "");
  const targetIsBlank = draft !== null && draft.target.trim().length === 0;
  const noteDirty = selectedCell?.translation != null && draft !== null && draft.note !== (selectedCell.translation.translatorNote ?? "");
  const cellBusy = mutation !== null;
  const targetCanSave = selectedCell !== null && !cellBusy && !targetIsBlank && (selectedCell.translation === null || targetDirty);
  const noteCanSave = selectedCell?.translation != null && !cellBusy && noteDirty;

  const updateDraft = useCallback((cell: TranslationCellDto, field: keyof CellDraft, value: string) => {
    const key = bindingKey(cell.sourceBinding);
    setDrafts((current) => ({ ...current, [key]: { ...draftForCell(cell, current), [field]: value } }));
  }, []);

  const discardDrafts = useCallback((keepBindingKey: string | null, keepField: keyof CellDraft | null) => {
    const currentRow = rowRef.current;
    if (!currentRow) return;
    setDrafts((current) => Object.fromEntries(currentRow.cells.map((cell) => {
      const key = bindingKey(cell.sourceBinding);
      const authoritative = { target: cell.translation?.targetMacro ?? "", note: cell.translation?.translatorNote ?? "" };
      if (key === keepBindingKey && keepField) authoritative[keepField] = current[key]?.[keepField] ?? authoritative[keepField];
      return [key, authoritative];
    })));
  }, []);

  const saveTarget = useCallback((advance: boolean) => {
    const currentRow = rowRef.current;
    if (!currentRow || !selectedCell || cellBusy) return;
    if (!targetCanSave) {
      if (advance) onNavigate(1);
      return;
    }
    const currentDraft = draftForCell(selectedCell, draftsRef.current);
    const otherDirty = hasOtherDirtyDraft(currentRow, selectedCell, draftsRef.current, "target");
    onSaveTarget(selectedCell, currentDraft, otherDirty, () => discardDrafts(bindingKey(selectedCell.sourceBinding), "target"), advance);
  }, [cellBusy, discardDrafts, onNavigate, onSaveTarget, selectedCell, targetCanSave]);

  const saveNote = useCallback(() => {
    const currentRow = rowRef.current;
    if (!currentRow || !selectedCell || !noteCanSave) return;
    const otherDirty = hasOtherDirtyDraft(currentRow, selectedCell, draftsRef.current, "note");
    onSaveNote(selectedCell, draftForCell(selectedCell, draftsRef.current), otherDirty, () => discardDrafts(bindingKey(selectedCell.sourceBinding), "note"));
  }, [discardDrafts, noteCanSave, onSaveNote, selectedCell]);

  const revert = useCallback(() => {
    setDrafts(draftsForRow(rowRef.current));
    onDirtyChange(false);
  }, [onDirtyChange]);

  const copySource = useCallback(() => {
    if (selectedCell && !cellBusy) updateDraft(selectedCell, "target", selectedCell.sourceMacro);
  }, [cellBusy, selectedCell, updateDraft]);

  const [drafting, setDrafting] = useState(false);
  const draftWithAngelica = useCallback(async () => {
    if (!selectedCell || cellBusy || !onDraftWithAngelica) return;
    const cell = selectedCell;
    setDrafting(true);
    try {
      const target = await onDraftWithAngelica(cell);
      if (target !== null) updateDraft(cell, "target", target);
    } finally {
      setDrafting(false);
    }
  }, [cellBusy, onDraftWithAngelica, selectedCell, updateDraft]);

  useImperativeHandle(ref, () => ({ saveTarget, revert, copySource }), [copySource, revert, saveTarget]);

  useEffect(() => {
    if (selectedKey !== null && takeFocusRequest()) focusMacroEditor(targetHostRef.current);
  }, [selectedKey, takeFocusRequest]);

  if (!row || !selectedCell || !draft) {
    return (
      <section className="editor editor-empty" aria-label={t("editor.label")}>
        <div className="empty-state">
          <UiIcon icon="languages" size="xl" />
          <strong>{t("editor.emptyTitle")}</strong>
          <p>{t("editor.emptyHintBefore")} <Kbd keys={["Alt", "Down"]} /> {t("editor.emptyHintAfter")}</p>
        </div>
      </section>
    );
  }

  const translation = selectedCell.translation;
  const domId = domKey(bindingKey(selectedCell.sourceBinding));

  return (
    <section className="editor" aria-label={t("editor.label")} aria-busy={cellBusy}>
      <header className="editor-bar">
        <div className="editor-ident">
          <ReviewDot state={translation?.reviewState ?? null} />
          <span className="editor-coord mono" title={t("editor.rowTitle", { sheet: row.sheetName, row: String(row.rowId), subrow: String(row.subrowId) })}>{row.rowId}:{row.subrowId}</span>
          {row.cells.length > 1 ? (
            <div className="field-tabs" role="tablist" aria-label={t("editor.fields")}>
              {row.cells.map((cell) => {
                const key = bindingKey(cell.sourceBinding);
                const active = key === selectedKey;
                const dirty = cellIsDirty(cell, draftForCell(cell, drafts));
                return (
                  <button className={active ? "field-tab active" : "field-tab"} type="button" role="tab" aria-selected={active} key={key} onClick={() => onSelectCell(cell.sourceBinding)}>
                    <ReviewDot state={cell.translation?.reviewState ?? null} />
                    {t("common.column", { column: String(cell.sourceBinding.columnIndex) })}
                    {dirty ? <span className="dirty-mark" aria-label={t("common.edited")} /> : null}
                  </button>
                );
              })}
            </div>
          ) : <span className="editor-field mono">{t("common.column", { column: String(selectedCell.sourceBinding.columnIndex) })}</span>}
        </div>
        <div className="editor-view-switch">
        <Segmented
          label={t("editor.view")}
          value="text"
          onChange={() => undefined}
          options={[
            { value: "text", label: t("editor.viewText") },
            { value: "preview", label: <><UiIcon icon="gamepad" size="xs" /> {t("editor.viewInGame")}</>, disabled: true, title: t("editor.inGameUnavailable") },
          ]}
        />
        </div>
        <div className="editor-bar-end">
          {rowDirty ? <span className="pill pill-warn">{t("common.unsaved")}</span> : null}
          <div className="review-control" title={translation ? undefined : t("editor.saveTargetFirst")}>
            <Segmented
              label={t("editor.reviewState")}
              value={translation?.reviewState ?? null}
              disabled={!translation || cellBusy}
              onChange={(state) => onReviewChange(selectedCell, state, () => discardDrafts(null, null))}
              options={reviewOptions.map((state) => ({ value: state, label: <><ReviewDot decorative state={state} />{t(reviewLabel(state))}</>, className: `review-${state}` }))}
            />
          </div>
          <IconButton icon="undo" label={t("editor.revert")} disabled={!rowDirty || mutations.length > 0} onClick={revert} />
        </div>
      </header>

      <div className="editor-grid">
        <div className="editor-pane editor-source">
          <div className="editor-pane-head">
            <span className="eyebrow">{t("editor.source")}</span>
            <span className="chip">{sourceLanguage.toUpperCase()}</span>
            {selectedCell.formattingOnly ? <span className="chip" title={t("list.formattingHint")}>{t("list.kind.formatting")}</span> : null}
            <span className="spacer" />
            {onDraftWithAngelica ? <IconButton icon="sparkles" label={drafting ? t("editor.drafting") : t("editor.draftWithAngelica")} disabled={cellBusy || drafting} onClick={() => void draftWithAngelica()} /> : null}
            <IconButton icon="copyPlus" label={t("editor.copySource")} disabled={cellBusy} onClick={copySource} />
          </div>
          <MacroEditor className="editor-surface" value={selectedCell.sourceMacro} readOnly ariaLabel={t("editor.sourceText", { column: String(selectedCell.sourceBinding.columnIndex) })} placeholder={t("editor.emptySource")} onNavigate={onNavigate} />
          {row.context.length > 0 ? (
            <details className="context-block">
              <summary><UiIcon icon="chevronRight" size="xs" />{t("editor.context")} <span className="count">{row.context.length}</span></summary>
              <ul>{row.context.map((cell) => <li key={`${cell.columnIndex}:${cell.sourceMacro}`}><span className="mono">{t("common.column", { column: String(cell.columnIndex) })}</span><span>{cell.sourceMacro}</span></li>)}</ul>
            </details>
          ) : null}
        </div>

        <div className="editor-pane editor-target" ref={targetHostRef}>
          <div className="editor-pane-head">
            <span className="eyebrow">{t("editor.target")}</span>
            {targetDirty ? <span className="edited-label">{t("common.edited")}</span> : null}
            {mutation === "target" ? <span className="saving-label"><span className="spinner spinner-xs" />{t("editor.savingInline")}</span> : null}
            <span className="spacer" />
            {checkpoint ? <IconButton icon="gitCompareArrows" label={t(showDiff ? "editor.hideDiff" : "editor.showDiff")} pressed={showDiff} onClick={() => setShowDiff((current) => !current)} className={`git-mark git-mark-${checkpoint.kind}`} /> : null}
          </div>
          {checkpoint && showDiff ? <CheckpointDiff baseline={checkpoint} current={draft.target} /> : null}
          <MacroEditor
            key={bindingKey(selectedCell.sourceBinding)}
            className="editor-surface"
            value={draft.target}
            ariaLabel={t("editor.targetText", { column: String(selectedCell.sourceBinding.columnIndex) })}
            placeholder={t("editor.targetPlaceholder")}
            disabled={cellBusy}
            onChange={(value) => updateDraft(selectedCell, "target", value)}
            onSave={() => saveTarget(false)}
            onSaveAndNext={() => saveTarget(true)}
            onNavigate={onNavigate}
          />
          <div className="editor-pane-foot">
            <span className="editor-hint">
              {targetIsBlank ? t("editor.enterTranslation") : targetDirty ? t("common.unsaved") : null}
            </span>
            <button className="button button-secondary" type="button" disabled={cellBusy} title={t(targetCanSave ? "editor.saveNextTitle" : "editor.nextTitle")} onClick={() => saveTarget(true)}>
              {t(targetCanSave ? "editor.saveNext" : "editor.next")}<UiIcon icon="arrowDown" size="xs" />
            </button>
            <button className="button button-primary" type="button" disabled={!targetCanSave} title={t(targetIsBlank ? "editor.saveBlankTitle" : "editor.saveTitle")} onClick={() => saveTarget(false)}>
              {t(mutation === "target" ? "common.saving" : "common.save")}<kbd className="button-kbd">Ctrl S</kbd>
            </button>
          </div>
        </div>

        <aside className="editor-pane editor-note">
          <div className="editor-pane-head">
            <label className="eyebrow" htmlFor={`translator-note-${domId}`}>{t("editor.note")}</label>
            {noteDirty ? <span className="edited-label">{t("common.edited")}</span> : null}
          </div>
          <textarea
            id={`translator-note-${domId}`}
            className="note-input"
            value={draft.note}
            onChange={(event) => updateDraft(selectedCell, "note", event.target.value)}
            onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") { event.preventDefault(); saveNote(); } }}
            placeholder={t(translation ? "editor.notePlaceholder" : "editor.noteDisabled")}
            disabled={!translation || cellBusy}
          />
          <div className="editor-pane-foot">
            <span className="editor-hint">{t(translation ? reviewLabel(translation.reviewState) : "editor.noTranslation")}</span>
            <button className="button button-secondary" type="button" onClick={saveNote} disabled={!noteCanSave}>{t(mutation === "note" ? "common.saving" : "editor.saveNote")}</button>
          </div>
        </aside>
      </div>
    </section>
  );
});

export const TranslationEditor = memo(TranslationEditorImpl);
