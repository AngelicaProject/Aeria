import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bindingKey, domKey } from "../binding";
import type { ReviewState, TranslationCellDto, TranslationRowDto } from "../types";

export type CellDraft = {
  target: string;
  note: string;
};

export type CellMutation = {
  kind: "target" | "note" | "review";
  bindingKey: string;
};

type TranslationEditorProps = {
  row: TranslationRowDto | null;
  mutation: CellMutation | null;
  onDirtyChange: (dirty: boolean) => void;
  onSaveTarget: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => void;
  onSaveNote: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => void;
  onReviewChange: (cell: TranslationCellDto, reviewState: ReviewState) => void;
};

const reviewStates: Array<{ value: ReviewState; label: string }> = [
  { value: "draft", label: "Draft" },
  { value: "reviewed", label: "Reviewed" },
  { value: "needsReview", label: "Needs review" },
];

function draftsForRow(row: TranslationRowDto | null): Record<string, CellDraft> {
  if (!row) {
    return {};
  }
  return Object.fromEntries(
    row.cells.map((cell) => [
      bindingKey(cell.sourceBinding),
      {
        target: cell.translation?.targetMacro ?? "",
        note: cell.translation?.translatorNote ?? "",
      },
    ]),
  );
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

function hasOtherDirtyDraft(
  row: TranslationRowDto,
  targetCell: TranslationCellDto,
  drafts: Record<string, CellDraft>,
  field: "target" | "note",
): boolean {
  const targetKey = bindingKey(targetCell.sourceBinding);
  return row.cells.some((cell) => {
    const key = bindingKey(cell.sourceBinding);
    const draft = draftForCell(cell, drafts);
    if (key !== targetKey) {
      return cellIsDirty(cell, draft);
    }
    return field === "target"
      ? cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? "")
      : draft.target !== (cell.translation?.targetMacro ?? "");
  });
}

type TranslationCellEditorProps = {
  cell: TranslationCellDto;
  draft: CellDraft;
  mutation: CellMutation | null;
  onDraftChange: (cell: TranslationCellDto, field: keyof CellDraft, value: string) => void;
  onSaveTarget: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => void;
  onSaveNote: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => void;
  onReviewChange: (cell: TranslationCellDto, reviewState: ReviewState) => void;
  getOtherDirty: (cell: TranslationCellDto, field: "target" | "note") => boolean;
};

const TranslationCellEditor = memo(function TranslationCellEditor({
  cell,
  draft,
  mutation,
  onDraftChange,
  onSaveTarget,
  onSaveNote,
  onReviewChange,
  getOtherDirty,
}: TranslationCellEditorProps) {
  const key = bindingKey(cell.sourceBinding);
  const domId = domKey(key);
  const targetDirty = draft.target !== (cell.translation?.targetMacro ?? "");
  const noteDirty = cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? "");
  const fieldMutation = mutation?.bindingKey === key ? mutation.kind : null;
  const targetCanSave = mutation === null && (cell.translation === null || targetDirty);
  const noteCanSave = mutation === null && cell.translation !== null && noteDirty;

  return (
    <div className="editor-section translation-cell-editor">
      <div className="field-heading">
        <label>Column {cell.sourceBinding.columnIndex}</label>
        {targetDirty || noteDirty ? <span className="field-dirty">edited</span> : null}
      </div>
      <div className="macro-display" aria-label={`Source macro for column ${cell.sourceBinding.columnIndex}`}>
        {cell.sourceMacro}
      </div>

      <div className="field-heading target-heading">
        <label htmlFor={`target-text-${domId}`}>Target</label>
        {targetDirty ? <span className="field-dirty">edited</span> : null}
      </div>
      <textarea
        id={`target-text-${domId}`}
        value={draft.target}
        onChange={(event) => onDraftChange(cell, "target", event.target.value)}
        placeholder="Enter a translation…"
        rows={7}
        disabled={mutation !== null}
      />
      <div className="field-actions">
        <button
          className="primary-button"
          type="button"
          onClick={() => onSaveTarget(cell, draft, getOtherDirty(cell, "target"))}
          disabled={!targetCanSave}
        >
          {fieldMutation === "target" ? "Saving…" : "Save target"}
        </button>
        {cell.translation === null ? <span className="field-hint">Saving an empty target creates an explicit translation entry.</span> : null}
      </div>

      <div className="review-section cell-review-section">
        <div className="field-heading">
          <label>Review</label>
          {fieldMutation === "review" ? <span className="field-hint">Saving…</span> : !cell.translation ? <span className="field-hint">Save the target first.</span> : null}
        </div>
        <div className="review-controls" role="group" aria-label={`Review state for column ${cell.sourceBinding.columnIndex}`}>
          {reviewStates.map((state) => (
            <button
              className={cell.translation?.reviewState === state.value ? "review-button active" : "review-button"}
              type="button"
              key={state.value}
              aria-pressed={cell.translation?.reviewState === state.value}
              disabled={cell.translation === null || mutation !== null}
              onClick={() => onReviewChange(cell, state.value)}
            >
              {state.label}
            </button>
          ))}
        </div>
      </div>

      <div className="field-heading note-heading">
        <label htmlFor={`translator-note-${domId}`}>Translator note</label>
        {noteDirty ? <span className="field-dirty">edited</span> : null}
      </div>
      <textarea
        id={`translator-note-${domId}`}
        value={draft.note}
        onChange={(event) => onDraftChange(cell, "note", event.target.value)}
        placeholder={cell.translation ? "Add context for another translator…" : "Save the target first to enable notes."}
        rows={4}
        disabled={!cell.translation || mutation !== null}
      />
      <div className="field-actions">
        <button
          className="secondary-button"
          type="button"
          onClick={() => onSaveNote(cell, draft, getOtherDirty(cell, "note"))}
          disabled={!noteCanSave}
        >
          {fieldMutation === "note" ? "Saving…" : "Save note"}
        </button>
        {!cell.translation ? <span className="field-hint">Save the target first to create this translation entry.</span> : null}
      </div>
    </div>
  );
});

export const TranslationEditor = memo(function TranslationEditor({
  row,
  mutation,
  onDirtyChange,
  onSaveTarget,
  onSaveNote,
  onReviewChange,
}: TranslationEditorProps) {
  const [drafts, setDrafts] = useState<Record<string, CellDraft>>({});
  const draftsRef = useRef(drafts);
  const rowRef = useRef(row);
  draftsRef.current = drafts;
  rowRef.current = row;

  useEffect(() => {
    setDrafts(draftsForRow(row));
    onDirtyChange(false);
  }, [onDirtyChange, row]);

  const rowDirty = useMemo(
    () => row?.cells.some((cell) => cellIsDirty(cell, draftForCell(cell, drafts))) ?? false,
    [drafts, row],
  );

  useEffect(() => {
    onDirtyChange(rowDirty);
  }, [onDirtyChange, rowDirty]);

  const updateDraft = useCallback((cell: TranslationCellDto, field: keyof CellDraft, value: string) => {
    const key = bindingKey(cell.sourceBinding);
    setDrafts((current) => ({
      ...current,
      [key]: { ...draftForCell(cell, current), [field]: value },
    }));
  }, []);

  const getOtherDirty = useCallback((cell: TranslationCellDto, field: "target" | "note") => {
    const currentRow = rowRef.current;
    return currentRow ? hasOtherDirtyDraft(currentRow, cell, draftsRef.current, field) : false;
  }, []);

  if (!row) {
    return (
      <section className="editor-pane empty-editor" aria-label="Translation editor">
        <div>
          <span className="empty-editor-mark" aria-hidden="true">↗</span>
          <h2>Select a row to edit</h2>
          <p>Choose a source row from the list to inspect its context and translation fields.</p>
        </div>
      </section>
    );
  }

  return (
    <section className="editor-pane" aria-label="Translation editor">
      <div className="editor-scroll">
        <div className="editor-heading">
          <div>
            <p className="pane-kicker">Selected row</p>
            <h2>Editor</h2>
          </div>
          {rowDirty ? <span className="dirty-indicator">Unsaved changes</span> : null}
        </div>

        <div className="row-coordinate editor-section">
          <span className="pane-kicker">Source coordinate</span>
          <code className="coordinate">{row.sheetName} · {row.rowId}:{row.subrowId}</code>
        </div>

        {row.context.length > 0 ? (
          <div className="editor-section context-section">
            <div className="field-heading"><label>Context</label></div>
            <div className="context-list">
              {row.context.map((cell) => (
                <code key={`${cell.columnIndex}:${cell.sourceMacro}`}>
                  Column {cell.columnIndex} · {cell.sourceMacro}
                </code>
              ))}
            </div>
          </div>
        ) : null}

        {row.cells.map((cell) => (
          <TranslationCellEditor
            key={bindingKey(cell.sourceBinding)}
            cell={cell}
            draft={draftForCell(cell, drafts)}
            mutation={mutation}
            onDraftChange={updateDraft}
            onSaveTarget={onSaveTarget}
            onSaveNote={onSaveNote}
            onReviewChange={onReviewChange}
            getOtherDirty={getOtherDirty}
          />
        ))}
      </div>
    </section>
  );
});
