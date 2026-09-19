import type { ProjectSheetDto } from "../types";

type SheetSidebarProps = {
  sheets: ProjectSheetDto[];
  selectedSheetName: string | null;
  disabled: boolean;
  onSelect: (sheetName: string) => void;
};

export function SheetSidebar({ sheets, selectedSheetName, disabled, onSelect }: SheetSidebarProps) {
  return (
    <aside className="pane sheet-pane" aria-labelledby="sheets-heading">
      <div className="pane-heading">
        <div>
          <p className="pane-kicker">Project</p>
          <h2 id="sheets-heading">Sheets</h2>
        </div>
        <span className="pane-count">{sheets.length}</span>
      </div>

      {sheets.length === 0 ? (
        <div className="empty-pane">
          <strong>No sheets</strong>
          <p>This source does not contain any browsable sheets.</p>
        </div>
      ) : (
        <div className="sheet-list">
          {sheets.map((sheet) => {
            const active = sheet.name === selectedSheetName;
            return (
              <button
                className={active ? "sheet-row active" : "sheet-row"}
                type="button"
                key={sheet.name}
                aria-pressed={active}
                disabled={disabled}
                onClick={() => onSelect(sheet.name)}
              >
                <span className="sheet-row-name">{sheet.name}</span>
                <span className="sheet-row-meta">
                  {sheet.rowCount.toLocaleString()} rows · {sheet.effectiveLanguage}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </aside>
  );
}
