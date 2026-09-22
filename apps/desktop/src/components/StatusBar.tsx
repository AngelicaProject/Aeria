import { displayPath } from "../pathDisplay";

type StatusBarProps = {
  sheetName: string | null;
  rowCount: number;
  loading: boolean;
  repositoryRoot: string;
  sourceLanguage: string;
  targetLanguage: string | null;
};

function shortenPath(path: string): string {
  const display = displayPath(path);
  const parts = display.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 3) {
    return display;
  }
  return `${parts[0]}\\…\\${parts.slice(-2).join("\\")}`;
}

export function StatusBar({
  sheetName,
  rowCount,
  loading,
  repositoryRoot,
  sourceLanguage,
  targetLanguage,
}: StatusBarProps) {
  return (
    <footer className="status-bar" aria-live="polite">
      <span className="status-item status-repository" title={displayPath(repositoryRoot)}>
        {shortenPath(repositoryRoot)}
      </span>
      <span className="status-item">{sourceLanguage} → {targetLanguage ?? "target not configured"}</span>
      <span className="status-item status-sheet">
        {sheetName ?? "No sheet selected"}
      </span>
      <span className="status-item status-rows">
        {loading ? "Loading rows…" : `${rowCount.toLocaleString()} rows loaded`}
      </span>
    </footer>
  );
}
