import { bindingKey } from "../binding";
import type { SourceBinding } from "../types";
import { displayPath } from "../pathDisplay";

type StatusBarProps = {
  sheetName: string | null;
  rowCount: number;
  loading: boolean;
  repositoryRoot: string;
  sourceLanguage: string;
  sourceSnapshotId: string;
  selectedBinding: SourceBinding | null;
  dirty: boolean;
};

function shortenSnapshot(snapshotId: string): string {
  return snapshotId.length > 18 ? `${snapshotId.slice(0, 10)}…${snapshotId.slice(-6)}` : snapshotId;
}

function shortenPath(path: string): string {
  const display = displayPath(path);
  const parts = display.split(/[\\/]/).filter(Boolean);
  return parts.length <= 3 ? display : `${parts[0]}\\…\\${parts.slice(-2).join("\\")}`;
}

export function StatusBar({ sheetName, rowCount, loading, repositoryRoot, sourceLanguage, sourceSnapshotId, selectedBinding, dirty }: StatusBarProps) {
  const selection = selectedBinding
    ? `${selectedBinding.sheetName} · ${selectedBinding.rowId}:${selectedBinding.subrowId} · col ${selectedBinding.columnIndex}`
    : sheetName ? `${sheetName} · no occurrence selected` : "No selection";

  return (
    <footer className="status-bar" aria-live="polite">
      <span className="status-item status-selection" title={selectedBinding ? bindingKey(selectedBinding) : undefined}>{selection}</span>
      {dirty ? <span className="status-item status-dirty">Draft changed</span> : null}
      <span className="status-spacer" />
      <span className="status-item status-source">Source attached</span>
      <span className="status-item">{sourceLanguage}</span>
      <span className="status-item" title={sourceSnapshotId}>snapshot {shortenSnapshot(sourceSnapshotId)}</span>
      <span className="status-item status-rows">{loading ? "Loading rows…" : `${rowCount.toLocaleString()} rows loaded`}</span>
      <span className="status-path" title={displayPath(repositoryRoot)}>{shortenPath(repositoryRoot)}</span>
    </footer>
  );
}
