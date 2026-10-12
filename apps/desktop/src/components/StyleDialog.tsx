import { memo, useEffect, useMemo, useState } from "react";
import { Dialog } from "radix-ui";
import { normalizeCommandError, projectKnowledge, saveKnowledgeStyle } from "../ipc";
import type { CommandError } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";

type StyleDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

/** The largest knowledge file Aeria reads. */
const FILE_LIMIT = 8 * 1024 * 1024;

/**
 * The project's style, `aeria-knowledge/style.md`, as one text.
 * Memoized so the closed dialog does not re-render with the workbench.
 */
export const StyleDialog = memo(function StyleDialog({ open, onOpenChange }: StyleDialogProps) {
  const { t } = useI18n();
  // The style as saved; `undefined` until it is read, `null` for no file.
  const [saved, setSaved] = useState<string | null | undefined>(undefined);
  const [readError, setReadError] = useState<string | null>(null);
  const [style, setStyle] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const [confirm, setConfirm] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSaved(undefined);
    setError(null);
    void projectKnowledge()
      .then((knowledge) => { setSaved(knowledge.style); setReadError(knowledge.styleError); setStyle(knowledge.style ?? ""); })
      .catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, [open]);

  const dirty = saved !== undefined && style !== (saved ?? "");
  const bytes = useMemo(() => new TextEncoder().encode(style).length, [style]);

  const save = () => {
    if (saved === undefined) return;
    setBusy(true);
    setError(null);
    void saveKnowledgeStyle(saved, style)
      .then((knowledge) => { setSaved(knowledge.style); setReadError(knowledge.styleError); setStyle(knowledge.style ?? ""); })
      .catch((reason: unknown) => setError(normalizeCommandError(reason)))
      .finally(() => setBusy(false));
  };

  const close = (next: boolean) => {
    if (next || !dirty) { onOpenChange(next); return; }
    setConfirm(true);
  };

  return (
    <Dialog.Root open={open} onOpenChange={close}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog style-dialog" aria-describedby={undefined}>
          <header className="dialog-header">
            <Dialog.Title className="dialog-title">{t("guide.title")}</Dialog.Title>
            <Dialog.Close className="icon-button icon-button-ghost" aria-label={t("settings.closeLabel")}><UiIcon icon="x" size="sm" /></Dialog.Close>
          </header>
          {error ? <ErrorBanner title={t("guide.error")} error={error} onDismiss={() => setError(null)} /> : null}
          {saved === undefined ? (
            error ? null : <p className="muted">{t("common.loading")}</p>
          ) : (
            <section className="style-body">
              <p className="field-hint">{t("guide.style.hint")}</p>
              {readError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{readError}</p> : null}
              <textarea className="input style-text" value={style} spellCheck placeholder={t("guide.style.placeholder")} aria-label={t("guide.title")} onChange={(event) => setStyle(event.target.value)} />
              <div className="dialog-actions">
                <span className={bytes > FILE_LIMIT ? "style-too-large" : "muted"}>{t("guide.size", { kib: (bytes / 1024).toFixed(1) })}</span>
                <button className="button button-ghost" type="button" disabled={busy || !dirty} onClick={() => setStyle(saved ?? "")}>{t("guide.revert")}</button>
                <button className="button button-primary" type="button" disabled={busy || !dirty || bytes > FILE_LIMIT} onClick={save}>{t("guide.save")}</button>
              </div>
            </section>
          )}
          <ConfirmDialog open={confirm} message={t("guide.discard")} onKeepEditing={() => setConfirm(false)} onDiscard={() => { setConfirm(false); onOpenChange(false); }} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
