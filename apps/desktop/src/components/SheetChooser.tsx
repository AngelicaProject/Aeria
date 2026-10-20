import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { buildSheetTree, findSheetMatches, type SheetTreeEntry, type SheetTreeFolder } from "../sheetExplorer";
import type { ProjectSheetDto } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

/** Most matches of a search listed. */
const SEARCH_LIMIT = 300;

/** The sheet names under a folder of the tree. */
function leavesOf(folder: SheetTreeFolder): string[] {
  const names: string[] = [];
  const visit = (entries: readonly SheetTreeEntry[]) => {
    for (const entry of entries) {
      if (entry.kind === "leaf") names.push(entry.sheet.name);
      else visit(entry.children);
    }
  };
  visit(folder.children);
  return names;
}

/** The sheets a run can take, as a tree with the sheet names under each folder. */
export function useSheetTree(sheets: readonly ProjectSheetDto[]) {
  const translatable = useMemo(() => sheets.filter((sheet) => sheet.translatableCellCount > 0 && !sheet.unavailable), [sheets]);
  const tree = useMemo(() => buildSheetTree(translatable), [translatable]);
  const leaves = useMemo(() => {
    const byFolder = new Map<string, string[]>();
    const visit = (entries: readonly SheetTreeEntry[]) => {
      for (const entry of entries) {
        if (entry.kind === "folder") {
          byFolder.set(entry.path, leavesOf(entry));
          visit(entry.children);
        }
      }
    };
    visit(tree.children);
    return byFolder;
  }, [tree]);
  const allNames = useMemo(() => translatable.map((sheet) => sheet.name), [translatable]);
  /**
   * The scope of a run: whole folders as `folder/`, other chosen sheets by
   * name, and an empty scope when every sheet is chosen.
   */
  const scopeOf = useCallback((selected: ReadonlySet<string>): string[] => {
    if (selected.size === allNames.length) return [];
    const scope: string[] = [];
    const visit = (children: readonly SheetTreeEntry[]) => {
      for (const entry of children) {
        if (entry.kind === "leaf") {
          if (selected.has(entry.sheet.name)) scope.push(entry.sheet.name);
          continue;
        }
        const names = leaves.get(entry.path) ?? [];
        if (names.length > 0 && names.every((name) => selected.has(name))) scope.push(`${entry.path}/`);
        else if (names.some((name) => selected.has(name))) visit(entry.children);
      }
    };
    visit(tree.children);
    return scope;
  }, [allNames.length, leaves, tree]);
  return { translatable, tree, leaves, allNames, scopeOf };
}

function Check({ checked, mixed, label, onChange }: { checked: boolean; mixed: boolean; label: string; onChange: (checked: boolean) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = mixed;
  }, [mixed]);
  return <input ref={ref} type="checkbox" aria-label={label} checked={checked} onChange={(event) => onChange(event.target.checked)} />;
}

type SheetChooserProps = {
  sheets: readonly ProjectSheetDto[];
  selected: ReadonlySet<string>;
  onToggle: (names: readonly string[], on: boolean) => void;
  /** The number shown beside a sheet; null while it is unknown. */
  countOf: (name: string) => number | null;
  /** How a count is shown. */
  renderCount: (count: number | null) => ReactNode;
  disabled: boolean;
  label: string;
  /** Quick choices shown above the tree. */
  presets?: ReactNode;
};

/** The project's sheets and folders of sheets, each chosen by a check, with a search. */
export function SheetChooser({ sheets, selected, onToggle, countOf, renderCount, disabled, label, presets }: SheetChooserProps) {
  const { t } = useI18n();
  const { translatable, tree, leaves } = useSheetTree(sheets);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [query, setQuery] = useState("");
  const toggleFolder = useCallback((path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);
  const countOfAll = (names: readonly string[]): number | null => {
    let sum = 0;
    for (const name of names) {
      const count = countOf(name);
      if (count === null) return null;
      sum += count;
    }
    return sum;
  };

  function renderEntries(entries: readonly SheetTreeEntry[], depth: number): ReactNode[] {
    const rows: ReactNode[] = [];
    for (const entry of entries) {
      const indent = { paddingInlineStart: `${depth * 16 + 6}px` };
      if (entry.kind === "leaf") {
        const name = entry.sheet.name;
        rows.push(
          <label key={name} className="translate-row" style={indent}>
            <span className="translate-twisty" />
            <Check checked={selected.has(name)} mixed={false} label={name} onChange={(on) => onToggle([name], on)} />
            <span className="translate-name">{entry.name}</span>
            {renderCount(countOf(name))}
          </label>,
        );
        continue;
      }
      const names = leaves.get(entry.path) ?? [];
      const chosen = names.filter((name) => selected.has(name)).length;
      const isOpen = expanded.has(entry.path);
      rows.push(
        <div key={`folder:${entry.path}`} className="translate-row" style={indent}>
          <button className="translate-twisty" type="button" aria-label={entry.path} aria-expanded={isOpen} onClick={() => toggleFolder(entry.path)}>
            <UiIcon icon={isOpen ? "chevronDown" : "chevronRight"} size="xs" />
          </button>
          <Check checked={chosen > 0 && chosen === names.length} mixed={chosen > 0 && chosen < names.length} label={entry.path} onChange={(on) => onToggle(names, on)} />
          <button className="translate-name translate-folder" type="button" onClick={() => toggleFolder(entry.path)}>{entry.name}</button>
          {renderCount(countOfAll(names))}
        </div>,
      );
      if (isOpen) rows.push(...renderEntries(entry.children, depth + 1));
    }
    return rows;
  }

  const matches = useMemo(() => query.trim() ? findSheetMatches(translatable, query) : null, [query, translatable]);
  return (
    <div className="translate-picker">
      <input className="input" type="search" placeholder={t("translate.search")} aria-label={t("translate.search")} value={query} disabled={disabled} onChange={(event) => setQuery(event.target.value)} />
      {presets ? <div className="translate-presets">{presets}</div> : null}
      <fieldset className="translate-tree" disabled={disabled} aria-label={label}>
        {matches
          ? matches.slice(0, SEARCH_LIMIT).map((match) => (
              <label key={match.sheet.name} className="translate-row">
                <span className="translate-twisty" />
                <Check checked={selected.has(match.sheet.name)} mixed={false} label={match.sheet.name} onChange={(on) => onToggle([match.sheet.name], on)} />
                <span className="translate-name">{match.basename}{match.breadcrumb ? <span className="muted"> · {match.breadcrumb}</span> : null}</span>
                {renderCount(countOf(match.sheet.name))}
              </label>
            ))
          : renderEntries(tree.children, 0)}
        {matches && matches.length > SEARCH_LIMIT ? <p className="field-hint">{t("translate.moreMatches", { shown: SEARCH_LIMIT, count: matches.length })}</p> : null}
        {matches && matches.length === 0 ? <p className="field-hint">{t("translate.noMatches")}</p> : null}
      </fieldset>
    </div>
  );
}
