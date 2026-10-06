import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { bindingKey, domKey, rowKey } from "../binding";
import type { EntryChangeKind, SourceBinding, TranslationCellDto, TranslationRowDto } from "../types";
import { diffWords } from "../textDiff";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { MacroEditor, focusMacroEditor, type MacroEditorApi } from "./MacroEditor";
import { InsertMacroButton, InsertMacroContextMenu } from "./InsertMacroMenu";
import { useMacroView } from "../ui/useMacroView";
import { speakerMarkers } from "../macroTokens";
import { ReviewDot, stateLabel, stringState } from "./ReviewDot";
import { OtherLanguages } from "./OtherLanguages";
import { StringHistory } from "./StringHistory";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import type { PaneMode, SidePaneTab } from "../ui/preferencesModel";
import type { MessageKey } from "../i18n/translate";
import { clearEditorFocus, setEditorFocus, type EditorFocus } from "../ui/editorFocus";

export type CellDraft = {
  target: string;
  note: string;
};

export type CellMutation = {
  kind: "target" | "note";
  bindingKey: string;
};

export type SaveTargetHandler = (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void, advance: boolean, review: boolean) => void;

/** Marks the saved translation of a string as reviewed by a person, or removes the mark. */
export type ReviewHandler = (cell: TranslationCellDto, reviewed: boolean) => void;

/** The committed state of the selected string when it has uncommitted changes. */
export type CheckpointBaseline = {
  kind: EntryChangeKind;
  /** Target at the last checkpoint; null when the string was not translated then. */
  target: string | null;
  fuzzyChanged: boolean;
  noteChanged: boolean;
};

function unchangedTextLabel(baseline: CheckpointBaseline): MessageKey {
  if (baseline.fuzzyChanged && baseline.noteChanged) return "editor.diff.fuzzyAndNoteChanged";
  if (baseline.fuzzyChanged) return "editor.diff.fuzzyChanged";
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

/** Marks the string reviewed, saving an edited target first, and moves on. */
export type ApproveHandler = (cell: TranslationCellDto, draft: CellDraft, targetDirty: boolean, otherDirty: boolean, discardOtherDrafts: () => void) => void;

export type TranslationEditorHandle = {
  saveTarget: (advance: boolean) => void;
  approve: () => void;
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
  onApprove: ApproveHandler;
  onReview: ReviewHandler;
  onSaveNote: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => void;
  onNavigate: (direction: 1 | -1) => void;
  /** Returns true once when the target should take focus after navigation. */
  takeFocusRequest: () => boolean;
  checkpoint: CheckpointBaseline | null;
  /** Bumps when the string's history may have changed (save, checkpoint, sync). */
  historyRevision?: number | undefined;
};

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
    draft.note !== (cell.translation?.translatorNote ?? "");
}

function hasOtherDirtyDraft(row: TranslationRowDto, targetCell: TranslationCellDto, drafts: Record<string, CellDraft>, field: "target" | "note"): boolean {
  const targetKey = bindingKey(targetCell.sourceBinding);
  return row.cells.some((cell) => {
    const key = bindingKey(cell.sourceBinding);
    const draft = draftForCell(cell, drafts);
    if (key !== targetKey) return cellIsDirty(cell, draft);
    return field === "target"
      ? draft.note !== (cell.translation?.translatorNote ?? "")
      : draft.target !== (cell.translation?.targetMacro ?? "");
  });
}

/** Switches a pane between text with tag chips and the macro code. */
function PaneModeSwitch({ value, onChange }: { value: PaneMode; onChange: (mode: PaneMode) => void }) {
  const { t } = useI18n();
  return (
    <Segmented<PaneMode>
      label={t("preview.mode")}
      value={value}
      onChange={onChange}
      options={[
        { value: "text", label: t("preview.mode.text"), title: t("preview.mode.textHint") },
        { value: "code", label: t("preview.mode.code"), title: t("preview.mode.codeHint") },
      ]}
    />
  );
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
  onApprove,
  onReview,
  onSaveNote,
  onNavigate,
  takeFocusRequest,
  checkpoint,
  historyRevision = 0,
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
  const { preferences, setPreference } = usePreferences();
  const sourceMode = preferences.sourcePaneMode;
  const targetMode = preferences.targetPaneMode;
  // Kept across rows: the editor is created anew for every row.
  const sideTab = preferences.sidePaneTab;
  const sourceView = useMacroView(selectedCell?.sourceMacro ?? null);
  // Clicking a tag of the source adds it to the translation at its cursor.
  const targetApi = useRef<MacroEditorApi | null>(null);
  const targetView = useMacroView(draft === null ? null : draft.target);
  // A speaker name the translation lost or added: valid text, but the game shows another name.
  const speakerNote = useMemo(() => {
    if (!selectedCell || draft === null || draft.target.trim().length === 0) return null;
    const source = speakerMarkers(selectedCell.sourceMacro) !== null;
    const target = speakerMarkers(draft.target) !== null;
    if (source === target) return null;
    return t(source ? "editor.speakerMissing" : "editor.speakerAdded");
  }, [selectedCell, draft, t]);

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

  const saveTarget = useCallback((advance: boolean, review = false) => {
    const currentRow = rowRef.current;
    if (!currentRow || !selectedCell || cellBusy) return;
    if (!targetCanSave) {
      if (advance) onNavigate(1);
      return;
    }
    const currentDraft = draftForCell(selectedCell, draftsRef.current);
    const otherDirty = hasOtherDirtyDraft(currentRow, selectedCell, draftsRef.current, "target");
    onSaveTarget(selectedCell, currentDraft, otherDirty, () => discardDrafts(bindingKey(selectedCell.sourceBinding), "target"), advance, review);
  }, [cellBusy, discardDrafts, onNavigate, onSaveTarget, selectedCell, targetCanSave]);

  // Saving and marking: an unchanged translation is only marked.
  const saveReviewed = useCallback(() => {
    if (!selectedCell || cellBusy) return;
    if (targetCanSave) saveTarget(false, true);
    else if (selectedCell.translation?.targetMacro && !selectedCell.translation.reviewed) onReview(selectedCell, true);
  }, [cellBusy, onReview, saveTarget, selectedCell, targetCanSave]);

  const approve = useCallback(() => {
    const currentRow = rowRef.current;
    if (!currentRow || !selectedCell || cellBusy) return;
    const currentDraft = draftForCell(selectedCell, draftsRef.current);
    const dirtyTarget = currentDraft.target !== (selectedCell.translation?.targetMacro ?? "");
    if (currentDraft.target.trim().length === 0) return;
    // A saved translation whose source did not change is accepted already.
    if (!dirtyTarget && selectedCell.translation !== null && !selectedCell.translation.fuzzy) {
      onNavigate(1);
      return;
    }
    const otherDirty = hasOtherDirtyDraft(currentRow, selectedCell, draftsRef.current, "target");
    onApprove(selectedCell, currentDraft, dirtyTarget, otherDirty, () => discardDrafts(bindingKey(selectedCell.sourceBinding), "target"));
  }, [cellBusy, discardDrafts, onApprove, onNavigate, selectedCell]);

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


  useImperativeHandle(ref, () => ({ saveTarget, approve, revert, copySource }), [approve, copySource, revert, saveTarget]);

  // The string guide reads the selected string and its translation as typed.
  const publishedFocus = useRef<EditorFocus | null>(null);
  const focusBinding = selectedCell?.sourceBinding ?? null;
  const focusSource = selectedCell?.sourceMacro ?? null;
  const focusDraft = draft?.target ?? null;
  const focusSaved = selectedCell?.translation?.targetMacro ?? "";
  const focusFuzzy = selectedCell?.translation?.fuzzy ?? false;
  const focusReviewed = selectedCell?.translation?.reviewed ?? false;
  useEffect(() => {
    const next = focusBinding === null || focusSource === null || focusDraft === null ? null : {
      binding: focusBinding,
      source: focusSource,
      draft: focusDraft,
      saved: focusSaved,
      fuzzy: focusFuzzy,
      reviewed: focusReviewed,
      busy: cellBusy,
      apply: (pick: Parameters<EditorFocus["apply"]>[0]) => { if (!cellBusy) targetApi.current?.apply(pick); },
    };
    if (next === null) clearEditorFocus(publishedFocus.current);
    else setEditorFocus(next);
    publishedFocus.current = next;
  }, [cellBusy, focusBinding, focusDraft, focusFuzzy, focusReviewed, focusSaved, focusSource]);
  useEffect(() => () => clearEditorFocus(publishedFocus.current), []);

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
          <ReviewDot state={stringState(translation)} />
          <span className="editor-coord mono" title={t("editor.rowTitle", { sheet: row.sheetName, row: String(row.rowId), subrow: String(row.subrowId) })}>{row.rowId}:{row.subrowId}</span>
          {row.cells.length > 1 ? (
            <div className="field-tabs" role="tablist" aria-label={t("editor.fields")}>
              {row.cells.map((cell) => {
                const key = bindingKey(cell.sourceBinding);
                const active = key === selectedKey;
                const dirty = cellIsDirty(cell, draftForCell(cell, drafts));
                return (
                  <button className={active ? "field-tab active" : "field-tab"} type="button" role="tab" aria-selected={active} key={key} onClick={() => onSelectCell(cell.sourceBinding)}>
                    <ReviewDot state={stringState(cell.translation)} />
                    {t("common.column", { column: String(cell.sourceBinding.columnIndex) })}
                    {dirty ? <span className="dirty-mark" aria-label={t("common.edited")} /> : null}
                  </button>
                );
              })}
            </div>
          ) : <span className="editor-field mono">{t("common.column", { column: String(selectedCell.sourceBinding.columnIndex) })}</span>}
        </div>
        <div className="editor-bar-end">
          {rowDirty ? <span className="pill pill-warn">{t("common.unsaved")}</span> : null}
          {translation?.fuzzy ? <span className="pill pill-warn" title={t("editor.fuzzyHint")}>{t("review.fuzzy")}</span> : null}
          {translation?.reviewStale ? <span className="pill pill-warn" title={t("editor.reviewStaleTitle")}>{t("editor.reviewStale")}</span> : null}
          {translation?.reviewed ? (
            // Shows the mark, and removes it; marking is the footer's.
            <button className="review-toggle is-reviewed" type="button" aria-pressed disabled={cellBusy} title={t("editor.reviewedTitle")} onClick={() => onReview(selectedCell, false)}>
              <ReviewDot decorative state="reviewed" />
              {t("editor.reviewed")}
            </button>
          ) : null}
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
            <IconButton icon="copyPlus" label={t("editor.copySource")} disabled={cellBusy} onClick={copySource} />
            <PaneModeSwitch value={sourceMode} onChange={(mode) => setPreference("sourcePaneMode", mode)} />
          </div>
          <MacroEditor className="editor-surface" value={selectedCell.sourceMacro} readOnly view={sourceView} presentation={sourceMode === "code" ? "code" : "chips"} onPick={cellBusy ? undefined : (pick) => targetApi.current?.apply(pick)} ariaLabel={t("editor.sourceText", { column: String(selectedCell.sourceBinding.columnIndex) })} placeholder={t("editor.emptySource")} onNavigate={onNavigate} />
          {translation?.fuzzy && translation.previousSource !== null ? (
            <div className="editor-previous-source">
              <span className="eyebrow">{t("editor.previousSource")}</span>
              <div className="git-diff">{diffWords(translation.previousSource, selectedCell.sourceMacro).map((part, index) => part.kind === "removed" ? <del key={index}>{part.text}</del> : part.kind === "added" ? <ins key={index}>{part.text}</ins> : <span key={index}>{part.text}</span>)}</div>
            </div>
          ) : null}
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
            <InsertMacroButton editor={targetApi} disabled={cellBusy} />
            {checkpoint ? <IconButton icon="gitCompareArrows" label={t(showDiff ? "editor.hideDiff" : "editor.showDiff")} pressed={showDiff} onClick={() => setShowDiff((current) => !current)} className={`git-mark git-mark-${checkpoint.kind === "translated" ? "added" : "modified"}`} /> : null}
            <PaneModeSwitch value={targetMode} onChange={(mode) => setPreference("targetPaneMode", mode)} />
          </div>
          {checkpoint && showDiff ? <CheckpointDiff baseline={checkpoint} current={draft.target} /> : null}
          <InsertMacroContextMenu editor={targetApi} disabled={cellBusy}>
            <div className="editor-context">
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
                onApproveAndNext={approve}
                onNavigate={onNavigate}
                view={targetView}
                presentation={targetMode === "code" ? "code" : "chips"}
                apiRef={targetApi}
              />
            </div>
          </InsertMacroContextMenu>
          <div className="editor-pane-foot">
            <span className="editor-hint">
              {targetIsBlank ? t("editor.enterTranslation") : speakerNote ? <span className="editor-warning" title={speakerNote}>{speakerNote}</span> : targetDirty ? t("common.unsaved") : null}
            </span>
            {/* The buttons move as one group: they never split across rows. */}
            <div className="editor-actions">
            {translation?.fuzzy ? (
              <button className="button button-ghost" type="button" disabled={cellBusy || targetIsBlank} title={t("editor.approveNextTitle")} onClick={approve}>
                <UiIcon icon="check" size="xs" />{t("editor.approveNext")}
              </button>
            ) : null}
            <button className="button button-secondary" type="button" disabled={cellBusy} title={t(targetCanSave ? "editor.saveNextTitle" : "editor.nextTitle")} onClick={() => saveTarget(true)}>
              {t(targetCanSave ? "editor.saveNext" : "editor.next")}<UiIcon icon="arrowDown" size="xs" />
            </button>
            {targetCanSave || (translation?.targetMacro && !translation.reviewed) ? (
              // With edits it saves and marks; without, it only marks the saved translation.
              <button className="button button-secondary" type="button" disabled={cellBusy || targetIsBlank} aria-label={t(targetCanSave ? "editor.saveReviewed" : "editor.review")} title={t(targetCanSave ? "editor.saveReviewedTitle" : "editor.reviewTitle")} onClick={saveReviewed}>
                <UiIcon icon="circleCheck" size="xs" /><span className="button-label">{t(targetCanSave ? "editor.saveReviewed" : "editor.review")}</span>
              </button>
            ) : null}
            <button className="button button-primary" type="button" disabled={!targetCanSave} title={t(targetIsBlank ? "editor.saveBlankTitle" : "editor.saveTitle")} onClick={() => saveTarget(false)}>
              {t(mutation === "target" ? "common.saving" : "common.save")}<kbd className="button-kbd">Ctrl S</kbd>
            </button>
            </div>
          </div>
        </div>

        <aside className="editor-pane editor-note">
          <div className="editor-pane-head">
            <Segmented<SidePaneTab>
              label={t("editor.sidePane")}
              value={sideTab}
              onChange={(tab) => setPreference("sidePaneTab", tab)}
              options={[
                { value: "note", label: <>{t("editor.note")}{noteDirty ? <span className="dirty-mark" aria-label={t("common.edited")} /> : null}</> },
                { value: "languages", label: t("editor.languages") },
                { value: "history", label: t("editor.history") },
              ]}
            />
          </div>
          {sideTab === "languages" ? (
            <div className="editor-languages">
              <OtherLanguages
                binding={selectedCell.sourceBinding}
                presentation={sourceMode === "code" ? "code" : "chips"}
                onPick={cellBusy ? undefined : (pick) => targetApi.current?.apply(pick)}
              />
            </div>
          ) : sideTab === "history" ? (
            <div className="editor-history">
              <StringHistory
                binding={selectedCell.sourceBinding}
                revision={historyRevision}
                onUseText={(target) => updateDraft(selectedCell, "target", target)}
              />
            </div>
          ) : <>
          <textarea
            id={`translator-note-${domId}`}
            aria-label={t("editor.note")}
            className="note-input"
            value={draft.note}
            onChange={(event) => updateDraft(selectedCell, "note", event.target.value)}
            onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") { event.preventDefault(); saveNote(); } }}
            placeholder={t("editor.notePlaceholder")}
            disabled={cellBusy}
          />
          <div className="editor-pane-foot">
            <span className="editor-hint">{t(stateLabel(stringState(translation)))}</span>
            <button className="button button-secondary" type="button" onClick={saveNote} disabled={!noteCanSave}>{t(mutation === "note" ? "common.saving" : "editor.saveNote")}</button>
          </div>
          </>}
        </aside>
      </div>
    </section>
  );
});

export const TranslationEditor = memo(TranslationEditorImpl);
