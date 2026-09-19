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
  drafts: Record<string, CellDraft>;
  mutation: CellMutation | null;
  onTargetChange: (cell: TranslationCellDto, value: string) => void;
  onNoteChange: (cell: TranslationCellDto, value: string) => void;
  onSaveTarget: (cell: TranslationCellDto) => void;
  onSaveNote: (cell: TranslationCellDto) => void;
  onReviewChange: (cell: TranslationCellDto, reviewState: ReviewState) => void;
};

const reviewStates: Array<{ value: ReviewState; label: string }> = [
  { value: "draft", label: "Draft" },
  { value: "reviewed", label: "Reviewed" },
  { value: "needsReview", label: "Needs review" },
];

export function TranslationEditor({
  row,
  drafts,
  mutation,
  onTargetChange,
  onNoteChange,
  onSaveTarget,
  onSaveNote,
  onReviewChange,
}: TranslationEditorProps) {
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

  const rowDirty = row.cells.some((cell) => {
    const key = bindingKey(cell.sourceBinding);
    const draft = drafts[key] ?? { target: cell.translation?.targetMacro ?? "", note: cell.translation?.translatorNote ?? "" };
    return draft.target !== (cell.translation?.targetMacro ?? "") ||
      (cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? ""));
  });

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

        {row.cells.map((cell) => {
          const key = bindingKey(cell.sourceBinding);
          const domId = domKey(key);
          const draft = drafts[key] ?? { target: cell.translation?.targetMacro ?? "", note: cell.translation?.translatorNote ?? "" };
          const targetDirty = draft.target !== (cell.translation?.targetMacro ?? "");
          const noteDirty = cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? "");
          const fieldMutation = mutation?.bindingKey === key ? mutation.kind : null;
          const targetCanSave = mutation === null && (cell.translation === null || targetDirty);
          const noteCanSave = mutation === null && cell.translation !== null && noteDirty;

          return (
            <div className="editor-section translation-cell-editor" key={key}>
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
                onChange={(event) => onTargetChange(cell, event.target.value)}
                placeholder="Enter a translation…"
                rows={7}
                disabled={mutation !== null}
              />
              <div className="field-actions">
                <button className="primary-button" type="button" onClick={() => onSaveTarget(cell)} disabled={!targetCanSave}>
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
                onChange={(event) => onNoteChange(cell, event.target.value)}
                placeholder={cell.translation ? "Add context for another translator…" : "Save the target first to enable notes."}
                rows={4}
                disabled={!cell.translation || mutation !== null}
              />
              <div className="field-actions">
                <button className="secondary-button" type="button" onClick={() => onSaveNote(cell)} disabled={!noteCanSave}>
                  {fieldMutation === "note" ? "Saving…" : "Save note"}
                </button>
                {!cell.translation ? <span className="field-hint">Save the target first to create this translation entry.</span> : null}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
