import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { UiIcon } from "../ui/primitives/UiIcon";
import type { ProjectSheetDto } from "../types";
import {
  buildSheetTree,
  collapseSheetTree,
  expandSheetAncestors,
  formatSheetCount,
  visibleSheetTreeEntries,
  type SheetTree,
  type SheetTreeEntry,
  type SheetTreeFolder,
} from "../sheetExplorer";

const SHEET_ROW_HEIGHT = 28;
const SHEET_OVERSCAN = 12;

type SheetSidebarProps = {
  sheets: readonly ProjectSheetDto[];
  selectedSheetName: string | null;
  disabled: boolean;
  active?: boolean;
  hideEmpty: boolean;
  onHideEmptyChange: (hide: boolean) => void;
  filterOpen: boolean;
  onFilterOpenChange: (open: boolean) => void;
  onOpenFilter: () => void;
  quickFindSignal?: number;
  revealSignal?: number;
  collapseSignal?: number;
  onSelect: (sheetName: string, pin?: boolean) => void;
};

function entryDepth(entry: SheetTreeEntry): number {
  return entry.kind === "folder" ? entry.path.split("/").length - 1 : entry.parentPath ? entry.parentPath.split("/").length : 0;
}

function treeRowStyle(depth: number): CSSProperties {
  return { "--tree-offset": `${Math.min(depth, 3) * 4}px` } as CSSProperties;
}

function folderContainsSelected(folder: SheetTreeFolder, selectedSheetName: string | null): boolean {
  return Boolean(selectedSheetName && selectedSheetName.startsWith(`${folder.path}/`));
}

function collectFolderPaths(tree: SheetTree): Set<string> {
  const paths = new Set<string>();
  function visit(entries: readonly SheetTreeEntry[]) {
    for (const entry of entries) {
      if (entry.kind === "folder") {
        paths.add(entry.path);
        visit(entry.children);
      }
    }
  }
  visit(tree.children);
  return paths;
}

function treeItemPositions(tree: SheetTree): WeakMap<SheetTreeEntry, { position: number; size: number }> {
  const positions = new WeakMap<SheetTreeEntry, { position: number; size: number }>();
  function visit(entries: readonly SheetTreeEntry[]) {
    entries.forEach((entry, index) => {
      positions.set(entry, { position: index + 1, size: entries.length });
      if (entry.kind === "folder") visit(entry.children);
    });
  }
  visit(tree.children);
  return positions;
}

export const SheetSidebar = memo(function SheetSidebar({
  sheets,
  selectedSheetName,
  disabled,
  active = true,
  hideEmpty,
  onHideEmptyChange,
  filterOpen,
  onFilterOpenChange,
  onOpenFilter,
  quickFindSignal = 0,
  revealSignal = 0,
  collapseSignal = 0,
  onSelect,
}: SheetSidebarProps) {
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(() => new Set());
  const [query, setQuery] = useState("");
  const selectedRef = useRef<HTMLButtonElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const treeViewportRef = useRef<HTMLDivElement>(null);
  const pendingRevealScroll = useRef<number | null>(null);
  const lastRevealSignal = useRef(revealSignal);
  const lastCollapseSignal = useRef(collapseSignal);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(420);

  const filteredSheets = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return sheets.filter((sheet) =>
      (!hideEmpty || sheet.translatableCellCount > 0)
      && (!normalized || sheet.name.toLocaleLowerCase().includes(normalized)),
    );
  }, [hideEmpty, query, sheets]);
  const tree = useMemo(() => buildSheetTree(filteredSheets), [filteredSheets]);
  const positions = useMemo(() => treeItemPositions(tree), [tree]);
  const visibleEntries = useMemo(
    () => visibleSheetTreeEntries(tree, query.trim() ? collectFolderPaths(tree) : expandedFolders),
    [expandedFolders, query, tree],
  );
  const effectiveScrollTop = Math.min(scrollTop, Math.max(0, visibleEntries.length * SHEET_ROW_HEIGHT - viewportHeight));
  const firstVisibleIndex = Math.min(
    visibleEntries.length,
    Math.max(0, Math.floor(effectiveScrollTop / SHEET_ROW_HEIGHT) - SHEET_OVERSCAN),
  );
  const lastVisibleIndex = Math.min(
    visibleEntries.length,
    Math.ceil((effectiveScrollTop + viewportHeight) / SHEET_ROW_HEIGHT) + SHEET_OVERSCAN,
  );
  const renderedEntries = visibleEntries.slice(firstVisibleIndex, lastVisibleIndex);

  useEffect(() => {
    if (selectedSheetName) setExpandedFolders((current) => expandSheetAncestors(current, selectedSheetName));
  }, [selectedSheetName]);

  useEffect(() => {
    if (!active) return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.ctrlKey && event.key.toLocaleLowerCase() === "f") {
        event.preventDefault();
        onOpenFilter();
      } else if (event.key === "Escape" && filterOpen) {
        if (query) setQuery("");
        else onFilterOpenChange(false);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [active, filterOpen, onFilterOpenChange, onOpenFilter, query]);

  useEffect(() => {
    if (filterOpen && quickFindSignal > 0) {
      filterRef.current?.focus();
      filterRef.current?.select();
    }
  }, [filterOpen, quickFindSignal]);

  useEffect(() => {
    const element = treeViewportRef.current;
    if (!element) return;
    const updateHeight = () => setViewportHeight(element.clientHeight);
    updateHeight();
    const observer = new ResizeObserver(updateHeight);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const element = treeViewportRef.current;
    if (!element) return;
    const targetScroll = pendingRevealScroll.current ?? 0;
    const isReveal = pendingRevealScroll.current !== null;
    pendingRevealScroll.current = null;
    element.scrollTop = targetScroll;
    setScrollTop(targetScroll);
    if (isReveal) requestAnimationFrame(() => requestAnimationFrame(() => selectedRef.current?.scrollIntoView({ block: "nearest" })));
  }, [hideEmpty, query]);

  const revealSelected = useCallback(() => {
    if (!selectedSheetName) return;
    const expanded = expandSheetAncestors(expandedFolders, selectedSheetName);
    const allTree = buildSheetTree(sheets);
    const revealEntries = visibleSheetTreeEntries(allTree, expanded);
    const selectedIndex = revealEntries.findIndex((entry) => entry.kind === "leaf" && entry.sheet.name === selectedSheetName);
    const targetScroll = selectedIndex < 0 ? 0 : Math.min(
      selectedIndex * SHEET_ROW_HEIGHT,
      Math.max(0, revealEntries.length * SHEET_ROW_HEIGHT - viewportHeight),
    );
    const filtersWillChange = hideEmpty || query.length > 0;
    pendingRevealScroll.current = filtersWillChange ? targetScroll : null;
    onHideEmptyChange(false);
    setQuery("");
    onFilterOpenChange(false);
    setExpandedFolders(expanded);
    if (!filtersWillChange) {
      if (treeViewportRef.current) treeViewportRef.current.scrollTop = targetScroll;
      setScrollTop(targetScroll);
      requestAnimationFrame(() => selectedRef.current?.scrollIntoView({ block: "nearest" }));
    }
  }, [expandedFolders, hideEmpty, onFilterOpenChange, onHideEmptyChange, query, selectedSheetName, sheets, viewportHeight]);

  function toggleFolder(path: string) {
    setExpandedFolders((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  const collapseAll = useCallback(() => {
    setQuery("");
    setExpandedFolders(collapseSheetTree());
  }, []);

  useEffect(() => {
    if (revealSignal === lastRevealSignal.current) return;
    lastRevealSignal.current = revealSignal;
    revealSelected();
  }, [revealSignal, revealSelected]);

  useEffect(() => {
    if (collapseSignal === lastCollapseSignal.current) return;
    lastCollapseSignal.current = collapseSignal;
    collapseAll();
  }, [collapseAll, collapseSignal]);

  return (
    <section className={`sheet-sidebar-body${filterOpen ? " has-sheet-filter" : ""}`} aria-label="Sheet Explorer">
      {filterOpen ? (
        <div className="sheet-filter">
          <UiIcon icon="search" size="sm" />
          <input
            ref={filterRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Filter sheets by name"
            aria-label="Filter sheets by name"
          />
          <button
            className="icon-button"
            type="button"
            aria-label="Close sheet filter"
            onClick={() => {
              setQuery("");
              onFilterOpenChange(false);
            }}
          ><UiIcon icon="x" size="xs" /></button>
        </div>
      ) : null}

      <div
        ref={treeViewportRef}
        className="sheet-tree"
        role="tree"
        aria-label="Project sheets"
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      >
        {visibleEntries.length === 0 ? (
          <div className="sheet-empty-state">
            <strong>{query.trim() ? "No matching sheets" : hideEmpty ? "No sheets with translatable strings" : "No sheets"}</strong>
            <p>{query.trim() ? "Try another name or clear the filter." : hideEmpty ? "Turn off Hide empty to browse the full source." : "This source does not contain any browsable sheets."}</p>
          </div>
        ) : (
          <div className="sheet-tree-spacer" style={{ height: visibleEntries.length * SHEET_ROW_HEIGHT }}>
            <div className="sheet-tree-rows" style={{ top: firstVisibleIndex * SHEET_ROW_HEIGHT }}>
              {renderedEntries.map((entry) => {
                const position = positions.get(entry)!;
                const depth = entryDepth(entry);
                if (entry.kind === "folder") {
                  const open = query.trim().length > 0 || expandedFolders.has(entry.path);
                  return (
                    <button
                      className={folderContainsSelected(entry, selectedSheetName) ? "sheet-tree-row folder-row contains-selected" : "sheet-tree-row folder-row"}
                      style={treeRowStyle(depth)}
                      type="button"
                      role="treeitem"
                      aria-expanded={open}
                      aria-level={depth + 1}
                      aria-posinset={position.position}
                      aria-setsize={position.size}
                      key={entry.path}
                      onClick={() => {
                        if (query.trim()) {
                          setQuery("");
                          setExpandedFolders((current) => new Set([...current, entry.path]));
                        } else {
                          toggleFolder(entry.path);
                        }
                      }}
                    >
                      <span className="sheet-tree-chevron" aria-hidden="true"><UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" /></span>
                      <span className="sheet-tree-icon sheet-tree-folder-icon" aria-hidden="true"><UiIcon icon={open ? "folderOpen" : "folder"} size="sm" /></span>
                      <span className="sheet-tree-name">{entry.name}</span>
                      <small className="sheet-tree-count">
                        {formatSheetCount(entry.descendantCount)}
                      </small>
                    </button>
                  );
                }
                const selected = entry.sheet.name === selectedSheetName;
                const count = entry.sheet.translatableCellCount;
                return (
                  <button
                    ref={selected ? selectedRef : undefined}
                    className={`${selected ? "sheet-tree-row leaf-row active" : "sheet-tree-row leaf-row"}${count === 0 ? " is-empty-sheet" : ""}`}
                    style={treeRowStyle(depth)}
                    type="button"
                    role="treeitem"
                    aria-selected={selected}
                    aria-level={depth + 1}
                    aria-posinset={position.position}
                    aria-setsize={position.size}
                    disabled={disabled}
                    key={entry.sheet.name}
                    onClick={() => onSelect(entry.sheet.name)}
                    onDoubleClick={() => onSelect(entry.sheet.name, true)}
                    title={`${entry.sheet.name}\n${entry.sheet.rowCount.toLocaleString()} source rows`}
                  >
                    <span className="sheet-tree-icon sheet-tree-sheet-icon" aria-hidden="true"><UiIcon icon="table2" size="sm" /></span>
                    <span className="sheet-tree-name">{entry.name}</span>
                    <small className="sheet-tree-count">
                      {formatSheetCount(count)}
                    </small>
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </section>
  );
});
