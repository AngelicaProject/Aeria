import type { ReviewState, TranslationEntryDto } from "../types";

type Mutation = "target" | "note" | "review" | null;

type TranslationEditorProps = {
  entry: TranslationEntryDto | null;
  targetDraft: string;
  noteDraft: string;
  targetDirty: boolean;
  noteDirty: boolean;
  mutation: Mutation;
  onTargetChange: (value: string) => void;
  onNoteChange: (value: string) => void;
  onSaveTarget: () => void;
  onSaveNote: () => void;
  onReviewChange: (reviewState: ReviewState) => void;
};

const reviewStates: Array<{ value: ReviewState; label: string }> = [
  { value: "draft", label: "Draft" },
  { value: "reviewed", label: "Reviewed" },
  { value: "needsReview", label: "Needs review" },
];

export function TranslationEditor({
  entry,
  targetDraft,
  noteDraft,
  targetDirty,
  noteDirty,
  mutation,
  onTargetChange,
  onNoteChange,
  onSaveTarget,
  onSaveNote,
  onReviewChange,
}: TranslationEditorProps) {
  if (!entry) {
    return (
      <section className="editor-pane empty-editor" aria-label="Translation editor">
        <div>
          <span className="empty-editor-mark" aria-hidden="true">↗</span>
          <h2>Select a string to edit</h2>
          <p>Choose a source entry from the list to inspect its source, target, and review state.</p>
        </div>
      </section>
    );
  }

  const translation = entry.translation;
  const targetCanSave = mutation === null && (translation === null || targetDirty);
  const noteCanSave = mutation === null && translation !== null;

  return (
    <section className="editor-pane" aria-label="Translation editor">
      <div className="editor-scroll">
        <div className="editor-heading">
          <div>
            <p className="pane-kicker">Selected entry</p>
            <h2>Editor</h2>
          </div>
          {targetDirty || noteDirty ? <span className="dirty-indicator">Unsaved changes</span> : null}
        </div>

        <div className="editor-section source-section">
          <div className="field-heading">
            <label>Source</label>
            <code className="coordinate">
              {entry.sourceBinding.sheetName} · {entry.sourceBinding.rowId}:{entry.sourceBinding.subrowId}:{entry.sourceBinding.columnIndex}
            </code>
          </div>
          <div className="macro-display" aria-label="Source macro">
            {entry.sourceMacro || "(empty source macro)"}
          </div>
        </div>

        <div className="editor-section">
          <div className="field-heading">
            <label htmlFor="target-text">Target</label>
            {targetDirty ? <span className="field-dirty">edited</span> : null}
          </div>
          <textarea
            id="target-text"
            value={targetDraft}
            onChange={(event) => onTargetChange(event.target.value)}
            placeholder="Enter a translation…"
            rows={7}
            disabled={mutation !== null}
          />
          <div className="field-actions">
            <button className="primary-button" type="button" onClick={onSaveTarget} disabled={!targetCanSave}>
              {mutation === "target" ? "Saving…" : "Save target"}
            </button>
            {translation === null ? <span className="field-hint">Saving an empty target creates an explicit translation entry.</span> : null}
          </div>
        </div>

        <div className="editor-section review-section">
          <div className="field-heading">
            <label>Review</label>
            {mutation === "review" ? <span className="field-hint">Saving…</span> : !translation ? <span className="field-hint">Save the target first.</span> : null}
          </div>
          <div className="review-controls" role="group" aria-label="Review state">
            {reviewStates.map((state) => (
              <button
                className={translation?.reviewState === state.value ? "review-button active" : "review-button"}
                type="button"
                key={state.value}
                aria-pressed={translation?.reviewState === state.value}
                disabled={translation === null || mutation !== null}
                onClick={() => onReviewChange(state.value)}
              >
                {state.label}
              </button>
            ))}
          </div>
        </div>

        <div className="editor-section">
          <div className="field-heading">
            <label htmlFor="translator-note">Translator note</label>
            {noteDirty ? <span className="field-dirty">edited</span> : null}
          </div>
          <textarea
            id="translator-note"
            value={noteDraft}
            onChange={(event) => onNoteChange(event.target.value)}
            placeholder={translation ? "Add context for another translator…" : "Save the target first to enable notes."}
            rows={4}
            disabled={!translation || mutation !== null}
          />
          <div className="field-actions">
            <button className="secondary-button" type="button" onClick={onSaveNote} disabled={!noteCanSave}>
              {mutation === "note" ? "Saving…" : "Save note"}
            </button>
            {!translation ? <span className="field-hint">Save the target first to create this translation entry.</span> : null}
          </div>
        </div>
      </div>
    </section>
  );
}
