import { Dialog } from "radix-ui";
import { sourceUpdateFacts } from "../sourceUpdate";
import type { SourceUpdateNeededDto, SourceUpdateReportDto } from "../types";
import { useI18n } from "../ui/i18n";

type SourceUpdateDialogProps = {
  open: boolean;
  /** `confirm` asks before a project is updated to the installed game; `summary` shows an update that ran. */
  mode: "confirm" | "summary";
  /** The versions of a project that needs an update, for `confirm`. */
  needed?: SourceUpdateNeededDto | null;
  /** What an update did, for `summary`. */
  report?: SourceUpdateReportDto | null;
  busy?: boolean;
  onConfirm?: () => void;
  onClose: () => void;
};

/** Asks before updating a project to a new game version, or tells what the update did. */
export function SourceUpdateDialog({ open, mode, needed = null, report = null, busy = false, onConfirm, onClose }: SourceUpdateDialogProps) {
  const { t } = useI18n();
  const version = (mode === "confirm" ? needed?.gameVersion : report?.gameVersion) ?? "";
  const previous = (mode === "confirm" ? needed?.previousGameVersion : report?.previousGameVersion) ?? "";

  return (
    <Dialog.Root open={open} onOpenChange={(next) => { if (!next && !busy) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog source-update-dialog" aria-describedby="source-update-description">
          <Dialog.Title className="dialog-title">{t(mode === "confirm" ? "sourceUpdate.confirmTitle" : "sourceUpdate.summaryTitle")}</Dialog.Title>
          <p id="source-update-description" className="dialog-description">
            {t(mode === "confirm" ? "sourceUpdate.confirmDescription" : "sourceUpdate.summaryDescription", { version, previous })}
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
          ) : <p className="dialog-description">{t("sourceUpdate.preservedNote")}</p>}
          <div className="dialog-actions">
            {mode === "confirm" ? (
              <>
                <button className="button button-secondary" type="button" disabled={busy} onClick={onClose}>{t("common.cancel")}</button>
                <button className="button button-primary" type="button" disabled={busy || !onConfirm} onClick={onConfirm}>
                  {t(busy ? "sourceUpdate.updating" : "sourceUpdate.confirm")}
                </button>
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
