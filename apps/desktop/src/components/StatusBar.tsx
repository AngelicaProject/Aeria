type StatusBarProps = {
  sheetName: string | null;
  rowCount: number;
  loading: boolean;
  repositoryRoot: string;
  sourceLanguage: string;
  targetLanguage: string;
};

function shortenPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 3) {
    return path;
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
      <span className="status-item status-repository" title={repositoryRoot}>
        {shortenPath(repositoryRoot)}
      </span>
      <span className="status-item">{sourceLanguage} → {targetLanguage}</span>
      <span className="status-item status-sheet">
        {sheetName ?? "No sheet selected"}
      </span>
      <span className="status-item status-rows">
        {loading ? "Loading rows…" : `${rowCount.toLocaleString()} rows loaded`}
      </span>
    </footer>
  );
}
