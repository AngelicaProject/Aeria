import type { SourceBinding } from "../types";
import { displayPath } from "../pathDisplay";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

type StatusBarProps = {
  sheetName: string | null;
  rowCount: number;
  loading: boolean;
  repositoryRoot: string;
  sourceLanguage: string;
  sourceSnapshotId: string;
  selectedBinding: SourceBinding | null;
  dirty: boolean;
  projectProgress: { translated: number; total: number } | null;
  detachedCount: number;
  onShowDetached: () => void;
};

function shortenSnapshot(snapshotId: string): string {
  const value = snapshotId.replace(/^sha256:/, "");
  return value.length > 12 ? value.slice(0, 12) : value;
}

function shortenPath(path: string): string {
  const display = displayPath(path);
  const parts = display.split(/[\\/]/).filter(Boolean);
  return parts.length <= 3 ? display : `${parts[0]}\\…\\${parts.slice(-2).join("\\")}`;
}

function formatPercent(share: number, locale: string): string {
  const digits = share > 0 && share < 0.001 ? 2 : 1;
  return (share * 100).toLocaleString(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

export function StatusBar({ sheetName, rowCount, loading, repositoryRoot, sourceLanguage, sourceSnapshotId, selectedBinding, dirty, projectProgress, detachedCount, onShowDetached }: StatusBarProps) {
  const { t, locale } = useI18n();
  const selection = selectedBinding
    ? t("common.cellLocation", { sheet: selectedBinding.sheetName, row: String(selectedBinding.rowId), subrow: String(selectedBinding.subrowId), column: String(selectedBinding.columnIndex) })
    : sheetName ?? t("status.noSheet");
  const share = projectProgress && projectProgress.total > 0 ? projectProgress.translated / projectProgress.total : null;

  return (
    <footer className="statusbar" aria-live="polite">
      <span className="status-item status-selection mono">{selection}</span>
      {dirty ? <span className="status-item status-dirty"><span className="status-dot" aria-hidden="true" />{t("status.unsavedChanges")}</span> : null}
      <span className="spacer" />
      <span className="status-item">{loading ? t("status.loadingRows") : t("status.rowsLoaded", { count: rowCount })}</span>
      {share !== null && projectProgress ? (
        <span className="status-item" title={t("status.projectProgress", { translated: projectProgress.translated, total: projectProgress.total })}>
          <span className="meter meter-inline" aria-hidden="true"><span className="meter-translated" style={{ width: `${Math.min(1, share) * 100}%` }} /></span>
          {t("status.translated", { percent: formatPercent(share, locale) })}
        </span>
      ) : null}
      {detachedCount > 0 ? (
        <button className="status-item status-button status-attention" type="button" title={t("status.showDetached")} onClick={onShowDetached}>
          <UiIcon icon="triangleAlert" size="xs" />{t("status.detached", { count: detachedCount })}
        </button>
      ) : null}
      <span className="status-item" title={t("status.sourceLanguage")}><UiIcon icon="languages" size="xs" />{sourceLanguage.toUpperCase()}</span>
      <span className="status-item mono" title={t("status.sourceSnapshot", { id: sourceSnapshotId })}>{shortenSnapshot(sourceSnapshotId)}</span>
      <span className="status-item status-path" title={displayPath(repositoryRoot)}><UiIcon icon="folder" size="xs" />{shortenPath(repositoryRoot)}</span>
    </footer>
  );
}
