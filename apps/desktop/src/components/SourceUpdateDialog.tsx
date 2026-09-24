import { useEffect, useState } from "react";
import { Dialog } from "radix-ui";
import { listDetachedUnits, normalizeCommandError } from "../ipc";
import { detachReasonLabels, migratesWorkspaceFormat, sourceUpdateFacts } from "../sourceUpdate";
import type { CommandError, DetachedUnitDto, SourceUpdateReportDto } from "../types";
import { useI18n } from "../ui/i18n";
import { ErrorBanner } from "./ErrorBanner";

type SourceUpdateDialogProps = {
  open: boolean;
  /**
   * `confirm` asks before an update is applied. `summary` shows an applied
   * update, or only the detached translations when `report` is null.
   */
  mode: "confirm" | "summary";
  report: SourceUpdateReportDto | null;
  busy?: boolean;
  onConfirm?: () => void;
  onClose: () => void;
};

type DetachedState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "loaded"; units: DetachedUnitDto[] }
  | { status: "failed"; error: CommandError };

export function SourceUpdateDialog({ open, mode, report, busy = false, onConfirm, onClose }: SourceUpdateDialogProps) {
  const { t } = useI18n();
  const [detached, setDetached] = useState<DetachedState>({ status: "idle" });
  const showDetached = mode === "summary" && (report === null || report.detached > 0);

  useEffect(() => {
    if (!open || !showDetached) return;
    let active = true;
    setDetached({ status: "loading" });
    void listDetachedUnits()
      .then((units) => { if (active) setDetached({ status: "loaded", units }); })
      .catch((error: unknown) => { if (active) setDetached({ status: "failed", error: normalizeCommandError(error) }); });
    return () => { active = false; };
  }, [open, showDetached]);

  const title = mode === "confirm"
    ? t("sourceUpdate.confirmTitle")
    : report ? t("sourceUpdate.summaryTitle") : t("sourceUpdate.detachedTitle");

  return (
    <Dialog.Root open={open} onOpenChange={(next) => { if (!next && !busy) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog source-update-dialog" aria-describedby="source-update-description">
          <Dialog.Title className="dialog-title">{title}</Dialog.Title>
          <p id="source-update-description" className="dialog-description">
            {mode === "confirm" && report
              ? t("sourceUpdate.confirmDescription", { version: report.gameVersion })
              : report
                ? t("sourceUpdate.summaryDescription", { version: report.gameVersion })
                : t("sourceUpdate.detachedDescription")}
          </p>
          {report ? (
            <>
              <ul className="source-update-facts">
                {sourceUpdateFacts(report, mode === "summary").map((fact) => (
                  <li key={fact.key} className={fact.tone === "attention" ? "attention" : undefined}>
                    {t(fact.key, { count: fact.count })}
                  </li>
                ))}
                {migratesWorkspaceFormat(report) ? <li>{t("sourceUpdate.fact.formatMigration")}</li> : null}
              </ul>
              {report.sheetSchemaUpdates.length > 0 ? (
                <div className="source-update-section">
                  <strong>{t("sourceUpdate.schemaTitle")}</strong>
                  <ul className="source-update-list">
                    {report.sheetSchemaUpdates.map((sheet, index) => (
                      <li key={`${sheet.sheetName}-${index}`}>
                        <span className="mono">{sheet.sheetName}</span>
                        <span className="muted">
                          {sheet.unavailable
                            ? t("sourceUpdate.schemaUnavailable")
                            : sheet.removed
                            ? t("sourceUpdate.schemaRemoved")
                            : t("sourceUpdate.schemaColumns", { mapped: sheet.mappedColumns, unresolved: sheet.unresolvedColumns })}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <p className="dialog-description">{t("sourceUpdate.preservedNote")}</p>
            </>
          ) : null}
          {showDetached ? (
            <div className="source-update-section">
              {report ? <strong>{t("sourceUpdate.detachedTitle")}</strong> : null}
              {detached.status === "loading" ? <div className="launcher-state"><span className="spinner" /> {t("common.loading")}</div> : null}
              {detached.status === "failed" ? <ErrorBanner title={t("sourceUpdate.detachedFailed")} error={detached.error} onDismiss={() => setDetached({ status: "idle" })} /> : null}
              {detached.status === "loaded" && detached.units.length === 0 ? <p className="muted">{t("sourceUpdate.detachedNone")}</p> : null}
              {detached.status === "loaded" && detached.units.length > 0 ? (
                <ul className="source-update-list">
                  {detached.units.map((unit) => (
                    <li key={unit.translationUnitId}>
                      <span className="mono">
                        {t("common.cellLocation", {
                          sheet: unit.lastSourceBinding.sheetName,
                          row: String(unit.lastSourceBinding.rowId),
                          subrow: String(unit.lastSourceBinding.subrowId),
                          column: String(unit.lastSourceBinding.columnIndex),
                        })}
                      </span>
                      <span className="muted">{t(detachReasonLabels[unit.reason])}</span>
                      <span className="source-update-target">{unit.targetMacro || t("common.empty")}</span>
                    </li>
                  ))}
                </ul>
              ) : null}
            </div>
          ) : null}
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
