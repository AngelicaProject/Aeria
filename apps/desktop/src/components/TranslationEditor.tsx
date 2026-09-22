import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bindingKey, domKey, rowKey } from "../binding";
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
  mutations: CellMutation[];
  onDirtyChange: (dirty: boolean) => void;
  onSaveTarget: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => void;
  onSaveNote: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => void;
  onReviewChange: (cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => void;
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
  mutation: CellMutation["kind"] | null;
  onDraftChange: (cell: TranslationCellDto, field: keyof CellDraft, value: string) => void;
  onSaveTarget: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => void;
  onSaveNote: (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => void;
  onReviewChange: (cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => void;
  getOtherDirty: (cell: TranslationCellDto, field: "target" | "note") => boolean;
  discardDrafts: (keepBindingKey: string | null, keepField: keyof CellDraft | null) => void;
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
  discardDrafts,
}: TranslationCellEditorProps) {
  const key = bindingKey(cell.sourceBinding);
  const domId = domKey(key);
  const targetDirty = draft.target !== (cell.translation?.targetMacro ?? "");
  const noteDirty = cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? "");
  const cellBusy = mutation !== null;
  const targetCanSave = !cellBusy && (cell.translation === null || targetDirty);
  const noteCanSave = !cellBusy && cell.translation !== null && noteDirty;

  return (
    <div className="editor-section translation-cell-editor" aria-busy={cellBusy}>
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
        disabled={cellBusy}
      />
      <div className="field-actions">
        <button
          className="primary-button"
          type="button"
          onClick={() => onSaveTarget(cell, draft, getOtherDirty(cell, "target"), () => discardDrafts(key, "target"))}
          disabled={!targetCanSave}
        >
          {mutation === "target" ? "Saving…" : "Save target"}
        </button>
        {cell.translation === null ? <span className="field-hint">Saving an empty target creates an explicit translation entry.</span> : null}
      </div>

      <div className="review-section cell-review-section">
        <div className="field-heading">
          <label>Review</label>
          {mutation === "review" ? <span className="field-hint mutation-status">Saving…</span> : !cell.translation ? <span className="field-hint">Save the target first.</span> : null}
        </div>
        <div className="review-controls" role="group" aria-label={`Review state for column ${cell.sourceBinding.columnIndex}`}>
          {reviewStates.map((state) => (
            <button
              className={cell.translation?.reviewState === state.value ? "review-button active" : "review-button"}
              type="button"
              key={state.value}
              aria-pressed={cell.translation?.reviewState === state.value}
              disabled={cell.translation === null || cellBusy}
              onClick={() => onReviewChange(cell, state.value, () => discardDrafts(null, null))}
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
        disabled={!cell.translation || cellBusy}
      />
      <div className="field-actions">
        <button
          className="secondary-button"
          type="button"
          onClick={() => onSaveNote(cell, draft, getOtherDirty(cell, "note"), () => discardDrafts(key, "note"))}
          disabled={!noteCanSave}
        >
          {mutation === "note" ? "Saving…" : "Save note"}
        </button>
        {!cell.translation ? <span className="field-hint">Save the target first to create this translation entry.</span> : null}
      </div>
    </div>
  );
});

export const TranslationEditor = memo(function TranslationEditor({
  row,
  mutations,
  onDirtyChange,
  onSaveTarget,
  onSaveNote,
  onReviewChange,
}: TranslationEditorProps) {
  const [drafts, setDrafts] = useState<Record<string, CellDraft>>({});
  const draftsRef = useRef(drafts);
  const rowRef = useRef(row);
  const previousRowRef = useRef<TranslationRowDto | null>(null);
  draftsRef.current = drafts;
  rowRef.current = row;

  useEffect(() => {
    setDrafts((current) => {
      if (!row || !previousRowRef.current || rowKey(previousRowRef.current) !== rowKey(row)) {
        return draftsForRow(row);
      }

      const next = draftsForRow(row);
      for (const cell of row.cells) {
        const previousCell = previousRowRef.current.cells.find(
          (candidate) => bindingKey(candidate.sourceBinding) === bindingKey(cell.sourceBinding),
        );
        const currentDraft = current[bindingKey(cell.sourceBinding)];
        if (previousCell && currentDraft && cellIsDirty(previousCell, currentDraft)) {
          next[bindingKey(cell.sourceBinding)] = currentDraft;
        }
      }
      return next;
    });
    previousRowRef.current = row;
  }, [row]);

  const rowDirty = useMemo(
    () => row?.cells.some((cell) => cellIsDirty(cell, draftForCell(cell, drafts))) ?? false,
    [drafts, row],
  );

  useEffect(() => {
    onDirtyChange(rowDirty);
  }, [onDirtyChange, rowDirty]);

  const handleRevert = useCallback(() => {
    setDrafts(draftsForRow(row));
    onDirtyChange(false);
  }, [onDirtyChange, row]);

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

  const discardDrafts = useCallback((keepBindingKey: string | null, keepField: keyof CellDraft | null) => {
    const currentRow = rowRef.current;
    if (!currentRow) return;

    setDrafts((current) => Object.fromEntries(currentRow.cells.map((cell) => {
      const key = bindingKey(cell.sourceBinding);
      const authoritative = {
        target: cell.translation?.targetMacro ?? "",
        note: cell.translation?.translatorNote ?? "",
      };
      if (key === keepBindingKey && keepField) {
        authoritative[keepField] = current[key]?.[keepField] ?? authoritative[keepField];
      }
      return [key, authoritative];
    })));
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
            <h2>Editor</h2>
            <span className="pane-subtitle">{row.sheetName} · {row.rowId}:{row.subrowId}</span>
          </div>
          <div className="editor-heading-actions">
            {rowDirty ? <span className="dirty-indicator">Unsaved changes</span> : null}
            <button className="secondary-button compact-button" type="button" onClick={handleRevert} disabled={!rowDirty || mutations.length > 0}>
              Revert
            </button>
          </div>
        </div>

        {row.context.length > 0 ? (
          <div className="editor-section context-section">
            <div className="field-heading"><label>Context cells</label></div>
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
            mutation={mutations.find((candidate) => candidate.bindingKey === bindingKey(cell.sourceBinding))?.kind ?? null}
            onDraftChange={updateDraft}
            onSaveTarget={onSaveTarget}
            onSaveNote={onSaveNote}
            onReviewChange={onReviewChange}
            getOtherDirty={getOtherDirty}
            discardDrafts={discardDrafts}
          />
        ))}
      </div>
    </section>
  );
});
