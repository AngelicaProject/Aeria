import type { SourceBinding } from "../types";
import { displayPath } from "../pathDisplay";
import { UiIcon } from "../ui/primitives/UiIcon";

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

export function StatusBar({ sheetName, rowCount, loading, repositoryRoot, sourceLanguage, sourceSnapshotId, selectedBinding, dirty, projectProgress }: StatusBarProps) {
  const selection = selectedBinding
    ? `${selectedBinding.sheetName} ${selectedBinding.rowId}:${selectedBinding.subrowId} · col ${selectedBinding.columnIndex}`
    : sheetName ?? "No sheet";
  const share = projectProgress && projectProgress.total > 0 ? projectProgress.translated / projectProgress.total : null;

  return (
    <footer className="statusbar" aria-live="polite">
      <span className="status-item status-selection mono">{selection}</span>
      {dirty ? <span className="status-item status-dirty"><span className="status-dot" aria-hidden="true" />Unsaved changes</span> : null}
      <span className="spacer" />
      <span className="status-item">{loading ? "Loading rows…" : `${rowCount.toLocaleString()} rows loaded`}</span>
      {share !== null && projectProgress ? (
        <span className="status-item" title={`${projectProgress.translated.toLocaleString()} of ${projectProgress.total.toLocaleString()} translatable strings in the project`}>
          <span className="meter meter-inline" aria-hidden="true"><span className="meter-translated" style={{ width: `${Math.min(1, share) * 100}%` }} /></span>
          {(share * 100).toFixed(share > 0 && share < 0.001 ? 2 : 1)}% translated
        </span>
      ) : null}
      <span className="status-item" title="Source language"><UiIcon icon="languages" size="xs" />{sourceLanguage.toUpperCase()}</span>
      <span className="status-item mono" title={`Source snapshot ${sourceSnapshotId}`}>{shortenSnapshot(sourceSnapshotId)}</span>
      <span className="status-item status-path" title={displayPath(repositoryRoot)}><UiIcon icon="folder" size="xs" />{shortenPath(repositoryRoot)}</span>
    </footer>
  );
}
