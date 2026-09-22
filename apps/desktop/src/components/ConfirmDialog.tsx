import { useEffect, useRef } from "react";

type ConfirmDialogProps = {
  open: boolean;
  message: string;
  onKeepEditing: () => void;
  onDiscard: () => void;
};

export function ConfirmDialog({ open, message, onKeepEditing, onDiscard }: ConfirmDialogProps) {
  const keepEditingRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;

    keepEditingRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onKeepEditing();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onKeepEditing, open]);

  if (!open) return null;

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onKeepEditing();
    }}>
      <section className="confirm-dialog" role="dialog" aria-modal="true" aria-labelledby="discard-dialog-title" aria-describedby="discard-dialog-message">
        <h2 id="discard-dialog-title">Unsaved changes</h2>
        <p id="discard-dialog-message">{message}</p>
        <div className="dialog-actions">
          <button ref={keepEditingRef} className="secondary-button" type="button" onClick={onKeepEditing}>
            Keep editing
          </button>
          <button className="danger-button" type="button" onClick={onDiscard}>
            Discard changes
          </button>
        </div>
      </section>
    </div>
  );
}
