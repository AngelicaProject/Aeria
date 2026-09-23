import { AlertDialog } from "radix-ui";

type ConfirmDialogProps = {
  open: boolean;
  message: string;
  onKeepEditing: () => void;
  onDiscard: () => void;
};

export function ConfirmDialog({ open, message, onKeepEditing, onDiscard }: ConfirmDialogProps) {
  return (
    <AlertDialog.Root open={open} onOpenChange={(next) => { if (!next) onKeepEditing(); }}>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className="dialog-overlay" />
        <AlertDialog.Content className="dialog confirm-dialog">
          <AlertDialog.Title className="dialog-title">Discard unsaved changes?</AlertDialog.Title>
          <AlertDialog.Description className="dialog-description">{message}</AlertDialog.Description>
          <div className="dialog-actions">
            <AlertDialog.Cancel className="button button-secondary">Keep editing</AlertDialog.Cancel>
            <AlertDialog.Action className="button button-danger" onClick={onDiscard}>Discard changes</AlertDialog.Action>
          </div>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}
