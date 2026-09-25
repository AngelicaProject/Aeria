import { AlertDialog } from "radix-ui";
import { useI18n } from "../ui/i18n";

type ConfirmDialogProps = {
  open: boolean;
  message: string;
  onKeepEditing: () => void;
  onDiscard: () => void;
  /** Title and labels for other confirmations than discarding edits. */
  title?: string;
  confirmLabel?: string;
  cancelLabel?: string;
};

export function ConfirmDialog({ open, message, onKeepEditing, onDiscard, title, confirmLabel, cancelLabel }: ConfirmDialogProps) {
  const { t } = useI18n();
  return (
    <AlertDialog.Root open={open} onOpenChange={(next) => { if (!next) onKeepEditing(); }}>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className="dialog-overlay" />
        <AlertDialog.Content className="dialog confirm-dialog">
          <AlertDialog.Title className="dialog-title">{title ?? t("confirm.title")}</AlertDialog.Title>
          <AlertDialog.Description className="dialog-description">{message}</AlertDialog.Description>
          <div className="dialog-actions">
            <AlertDialog.Cancel className="button button-secondary">{cancelLabel ?? t("confirm.keepEditing")}</AlertDialog.Cancel>
            <AlertDialog.Action className="button button-danger" onClick={onDiscard}>{confirmLabel ?? t("confirm.discard")}</AlertDialog.Action>
          </div>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}
