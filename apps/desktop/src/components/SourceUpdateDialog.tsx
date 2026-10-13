import { Dialog } from "radix-ui";
import { sourceUpdateFacts } from "../sourceUpdate";
import type { IncomingUpdateDto, SourceUpdateNeededDto, SourceUpdateReportDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";

type SourceUpdateDialogProps = {
  open: boolean;
  /** `confirm` asks before a project is updated to the installed game; `summary` shows an update that ran. */
  mode: "confirm" | "summary";
  /** The versions of a project that needs an update, for `confirm`. */
  needed?: SourceUpdateNeededDto | null;
  /** What an update did, for `summary`. */
  report?: SourceUpdateReportDto | null;
  /**
   * For `confirm`: whether the upstream already holds the update, `null`
   * while the remote is asked. The project is updated here only once the
   * remote answered that it does not.
   */
  incoming?: IncomingUpdateDto | null;
  /** Pulling this project's update failed before: updating it here is offered beside the pull. */
  pullFailed?: boolean;
  busy?: boolean;
  onConfirm?: () => void;
  onPull?: () => void;
  onCheckAgain?: () => void;
  onClose: () => void;
};

/** Asks before updating a project to a new game version, or tells what the update did. */
export function SourceUpdateDialog({ open, mode, needed = null, report = null, incoming = null, pullFailed = false, busy = false, onConfirm, onPull, onCheckAgain, onClose }: SourceUpdateDialogProps) {
  const { t } = useI18n();
  const version = (mode === "confirm" ? needed?.gameVersion : report?.gameVersion) ?? "";
  const previous = (mode === "confirm" ? needed?.previousGameVersion : report?.previousGameVersion) ?? "";

  const upstream = incoming?.status === "incoming" ? incoming.upstream : "";
  const confirmText: { title: MessageKey; description: MessageKey; note: MessageKey } = incoming === null
    ? { title: "sourceUpdate.confirmTitle", description: "sourceUpdate.versions", note: "sourceUpdate.checking" }
    : incoming.status === "incoming"
      ? { title: "sourceUpdate.pullTitle", description: "sourceUpdate.pullDescription", note: "sourceUpdate.pullNote" }
      : incoming.status === "unknown"
        ? { title: "sourceUpdate.confirmTitle", description: "sourceUpdate.versions", note: "sourceUpdate.unknown" }
        : { title: "sourceUpdate.confirmTitle", description: "sourceUpdate.confirmDescription", note: "sourceUpdate.preservedNote" };

  return (
    <Dialog.Root open={open} onOpenChange={(next) => { if (!next && !busy) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog source-update-dialog" aria-describedby="source-update-description">
          <Dialog.Title className="dialog-title">{t(mode === "confirm" ? confirmText.title : "sourceUpdate.summaryTitle")}</Dialog.Title>
          <p id="source-update-description" className="dialog-description">
            {t(mode === "confirm" ? confirmText.description : "sourceUpdate.summaryDescription", { version, previous, upstream })}
          </p>
          {mode === "summary" && report ? (
            <>
              <ul className="source-update-facts">
                {sourceUpdateFacts(report).map((fact) => (
                  <li key={fact.key} className={fact.tone === "attention" ? "attention" : undefined}>
                    {t(fact.key, { count: fact.count })}
                  </li>
                ))}
              </ul>
              <p className="dialog-description">
                {report.commit ? t("sourceUpdate.committed", { commit: report.commit.slice(0, 7) }) : t("sourceUpdate.notCommitted")}
              </p>
            </>
          ) : <p className="dialog-description" role="status">{t(confirmText.note)}</p>}
          <div className="dialog-actions">
            {mode === "confirm" ? (
              <>
                <button className="button button-secondary" type="button" disabled={busy} onClick={onClose}>{t("common.cancel")}</button>
                {incoming?.status === "unknown" ? (
                  <button className="button button-secondary" type="button" disabled={busy || !onCheckAgain} onClick={onCheckAgain}>{t("sourceUpdate.checkAgain")}</button>
                ) : null}
                {incoming?.status === "incoming" ? (
                  <>
                    {pullFailed ? (
                      <button className="button button-secondary" type="button" disabled={busy || !onConfirm} onClick={onConfirm}>{t("sourceUpdate.confirm")}</button>
                    ) : null}
                    <button className="button button-primary" type="button" disabled={busy || !onPull} onClick={onPull}>
                      {t(busy ? "sourceUpdate.pulling" : "sourceUpdate.pull")}
                    </button>
                  </>
                ) : (
                  <button className="button button-primary" type="button" disabled={busy || incoming === null || !onConfirm} onClick={onConfirm}>
                    {t(busy ? "sourceUpdate.updating" : "sourceUpdate.confirm")}
                  </button>
                )}
              </>
            ) : (
              <button className="button button-primary" type="button" onClick={onClose}>{t("common.close")}</button>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
