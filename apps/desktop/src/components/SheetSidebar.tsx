import { memo, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { Icon } from "../ui/primitives/Icon";
import type { ProjectSheetDto } from "../types";
import {
  buildSheetTree,
  collapseSheetTree,
  expandSheetAncestors,
  findSheetMatches,
  visibleSheetTreeEntries,
  type SheetTreeEntry,
  type SheetTreeFolder,
} from "../sheetExplorer";

type SheetSidebarProps = {
  sheets: readonly ProjectSheetDto[];
  selectedSheetName: string | null;
  disabled: boolean;
  active?: boolean;
  quickFindSignal?: number;
  onSelect: (sheetName: string, pin?: boolean) => void;
};

function entryDepth(entry: SheetTreeEntry): number {
  return entry.kind === "folder" ? entry.path.split("/").length - 1 : entry.parentPath ? entry.parentPath.split("/").length : 0;
}

function folderContainsSelected(folder: SheetTreeFolder, selectedSheetName: string | null): boolean {
  return Boolean(selectedSheetName && selectedSheetName.startsWith(`${folder.path}/`));
}

export const SheetSidebar = memo(function SheetSidebar({
  sheets,
  selectedSheetName,
  disabled,
  active = true,
  quickFindSignal = 0,
  onSelect,
}: SheetSidebarProps) {
  const tree = useMemo(() => buildSheetTree(sheets), [sheets]);
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(() => new Set());
  const [quickFindOpen, setQuickFindOpen] = useState(false);
  const [query, setQuery] = useState("");
  const selectedRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (selectedSheetName) setExpandedFolders((current) => expandSheetAncestors(current, selectedSheetName));
  }, [selectedSheetName]);

  useEffect(() => {
    if (!active) return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.ctrlKey && event.key.toLocaleLowerCase() === "f") {
        event.preventDefault();
        setQuickFindOpen(true);
      } else if (event.key === "Escape" && quickFindOpen) {
        setQuickFindOpen(false);
        setQuery("");
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [active, quickFindOpen]);

  useEffect(() => {
    if (quickFindSignal > 0) setQuickFindOpen(true);
  }, [quickFindSignal]);

  const visibleEntries = useMemo(() => visibleSheetTreeEntries(tree, expandedFolders), [expandedFolders, tree]);
  const matches = useMemo(() => findSheetMatches(sheets, query), [query, sheets]);

  function revealSelected() {
    if (!selectedSheetName) return;
    setExpandedFolders((current) => expandSheetAncestors(current, selectedSheetName));
    requestAnimationFrame(() => selectedRef.current?.scrollIntoView({ block: "nearest" }));
  }

  function toggleFolder(path: string) {
    setExpandedFolders((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  return (
    <div className="sheet-sidebar-body" aria-label="Sheet Explorer">
      <div className="sheet-explorer-head">
        <span>{sheets.length.toLocaleString()} sheets</span>
        <div className="sheet-explorer-actions">
          <button className="icon-button" type="button" aria-label="Quick Find sheets" title="Quick Find (Ctrl+F)" disabled={disabled} onClick={() => setQuickFindOpen(true)}><Icon name="search" size={14} /></button>
          <button className="icon-button" type="button" aria-label="Reveal selected sheet" title="Reveal selected sheet" disabled={disabled || !selectedSheetName} onClick={revealSelected}><Icon name="target" size={14} /></button>
          <button className="icon-button" type="button" aria-label="Collapse all sheets" title="Collapse all" disabled={disabled} onClick={() => setExpandedFolders(collapseSheetTree())}><Icon name="collapse" size={14} /></button>
        </div>
      </div>

      {quickFindOpen ? (
        <div className="sheet-quick-find" role="search" aria-label="Quick Find sheets">
          <div className="sheet-quick-find-input">
            <Icon name="search" size={13} />
            <input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Find sheets…" aria-label="Find sheets" />
            <button className="icon-button" type="button" aria-label="Close Quick Find" onClick={() => { setQuickFindOpen(false); setQuery(""); }}><Icon name="close" size={12} /></button>
          </div>
          <div className="sheet-matches" role="listbox" aria-label="Matching sheets">
            {query.trim() === "" ? <span className="sheet-match-empty">Type to find a sheet by name.</span> : matches.length === 0 ? <span className="sheet-match-empty">No sheets match “{query}”.</span> : matches.map((match) => (
              <button className={match.sheet.name === selectedSheetName ? "sheet-match active" : "sheet-match"} type="button" role="option" aria-selected={match.sheet.name === selectedSheetName} key={match.sheet.name} onClick={() => { onSelect(match.sheet.name); setQuickFindOpen(false); setQuery(""); }}>
                <span><strong>{match.basename}</strong>{match.breadcrumb ? <small>{match.breadcrumb}</small> : null}</span>
                <code>{match.sheet.rowCount.toLocaleString()}</code>
              </button>
            ))}
          </div>
        </div>
      ) : (
        <div className="sheet-tree" role="tree" aria-label="Project sheets">
          {visibleEntries.length === 0 ? <div className="empty-pane"><strong>No sheets</strong><p>This source does not contain any browsable sheets.</p></div> : visibleEntries.map((entry) => {
            const depth = entryDepth(entry);
            if (entry.kind === "folder") {
              const open = expandedFolders.has(entry.path);
              return <button className={folderContainsSelected(entry, selectedSheetName) ? "sheet-tree-row folder-row contains-selected" : "sheet-tree-row folder-row"} style={{ "--tree-depth": depth } as CSSProperties} type="button" role="treeitem" aria-expanded={open} key={entry.path} onClick={() => toggleFolder(entry.path)}>
                <Icon name={open ? "chevronDown" : "chevronRight"} size={13} />
                <span className="sheet-tree-name">{entry.name}</span>
                <code>{entry.descendantCount.toLocaleString()}</code>
              </button>;
            }
            const selected = entry.sheet.name === selectedSheetName;
            return <button ref={selected ? selectedRef : undefined} className={selected ? "sheet-tree-row leaf-row active" : "sheet-tree-row leaf-row"} style={{ "--tree-depth": depth } as CSSProperties} type="button" role="treeitem" aria-selected={selected} disabled={disabled} key={entry.sheet.name} onClick={() => onSelect(entry.sheet.name)} onDoubleClick={() => onSelect(entry.sheet.name, true)}>
              <span className="sheet-tree-leaf-mark" aria-hidden="true" />
              <span className="sheet-tree-main"><strong>{entry.name}</strong>{entry.parentPath ? <small>{entry.parentPath.replaceAll("/", " › ")}</small> : null}</span>
              <code>{entry.sheet.rowCount.toLocaleString()}</code>
            </button>;
          })}
        </div>
      )}
    </div>
  );
});
