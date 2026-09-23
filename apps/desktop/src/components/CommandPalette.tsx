import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { palettePrefixes, parsePaletteQuery, parseRowTarget, type RowTarget } from "../commandPalette";
import { fuzzyFilter } from "../fuzzy";
import type { ProjectSheetDto, SheetProgressDto } from "../types";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";

export type PaletteCommand = {
  id: string;
  title: string;
  category: string;
  shortcut?: string | undefined;
  icon?: UiIconName | undefined;
  enabled?: boolean | undefined;
  run: () => void;
};

type PaletteItem = {
  key: string;
  icon: UiIconName;
  label: string;
  indices?: number[] | undefined;
  detail?: string | undefined;
  hint?: ReactNode;
  disabled?: boolean | undefined;
  run?: (() => void) | undefined;
};

type CommandPaletteProps = {
  open: boolean;
  initialInput: string;
  onOpenChange: (open: boolean) => void;
  commands: readonly PaletteCommand[];
  sheets: readonly ProjectSheetDto[];
  progress: ReadonlyMap<string, SheetProgressDto>;
  recentSheets: readonly string[];
  currentSheet: string | null;
  onOpenSheet: (sheetName: string) => void;
  onGoToRow: (target: RowTarget) => void;
};

function Highlighted({ text, indices = [] }: { text: string; indices?: number[] | undefined }) {
  if (indices.length === 0) return <>{text}</>;
  const marked = new Set(indices);
  return <>{[...text].map((character, index) => marked.has(index) ? <mark key={index}>{character}</mark> : character)}</>;
}

/** Mount with a fresh `key` per opening so `initialInput` seeds the query. */
export function CommandPalette({ open, initialInput, onOpenChange, commands, sheets, progress, recentSheets, currentSheet, onOpenSheet, onGoToRow }: CommandPaletteProps) {
  const [input, setInput] = useState(initialInput);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const query = parsePaletteQuery(input);

  const items = useMemo<PaletteItem[]>(() => {
    const close = (action: () => void) => () => { onOpenChange(false); action(); };
    switch (query.mode) {
      case "commands":
        return fuzzyFilter(query.term, commands, (command) => `${command.category}: ${command.title}`).map(({ item, match }) => ({
          key: item.id,
          icon: item.icon ?? "chevronRight",
          label: `${item.category}: ${item.title}`,
          indices: match.indices,
          hint: item.shortcut ? <kbd>{item.shortcut}</kbd> : undefined,
          disabled: item.enabled === false,
          run: close(item.run),
        }));
      case "goto": {
        const target = parseRowTarget(query.term);
        if (!currentSheet) return [{ key: "no-sheet", icon: "info", label: "Open a sheet first", disabled: true }];
        if (!query.term) return [{ key: "hint", icon: "info", label: `Type a row number to go to in ${currentSheet}`, detail: "row, row:subrow, or row:subrow:column", disabled: true }];
        if (!target) return [{ key: "invalid", icon: "circleAlert", label: "Not a row coordinate", detail: "Use row, row:subrow, or row:subrow:column", disabled: true }];
        const coordinate = `${target.rowId}:${target.subrowId}${target.columnIndex === null ? "" : ` · col ${target.columnIndex}`}`;
        return [{ key: "goto", icon: "arrowRight", label: `Go to ${coordinate}`, detail: currentSheet, run: close(() => onGoToRow(target)) }];
      }
      case "strings":
        return [{
          key: "strings-unavailable",
          icon: "search",
          label: query.term ? `Search strings for “${query.term}”` : "Search source and target text across the project",
          detail: "Project search is not available in this build",
          disabled: true,
        }];
      case "help":
        return palettePrefixes.filter((entry) => entry.mode !== "help").map((entry): PaletteItem => ({
          key: entry.prefix,
          icon: "info",
          label: `${entry.prefix}  ${entry.label}`,
          run: () => { setInput(entry.prefix); inputRef.current?.focus(); },
        })).concat([{ key: "sheets", icon: "table2", label: "No prefix  Go to a sheet", run: () => { setInput(""); inputRef.current?.focus(); } }]);
      case "sheets": {
        const sheetItem = (sheet: ProjectSheetDto, indices?: number[]): PaletteItem => {
          const sheetProgress = progress.get(sheet.name);
          const detail = sheet.translatableCellCount === 0
            ? "no translatable strings"
            : `${(sheetProgress?.translated ?? 0).toLocaleString()} / ${sheet.translatableCellCount.toLocaleString()} translated`;
          return { key: sheet.name, icon: "table2", label: sheet.name, indices, detail, hint: sheet.name === currentSheet ? <span className="palette-badge">open</span> : undefined, run: close(() => onOpenSheet(sheet.name)) };
        };
        if (!query.term.trim()) {
          const byName = new Map(sheets.map((sheet) => [sheet.name, sheet]));
          const recent = recentSheets.map((name) => byName.get(name)).filter((sheet): sheet is ProjectSheetDto => sheet !== undefined);
          const rest = sheets.filter((sheet) => sheet.translatableCellCount > 0 && !recentSheets.includes(sheet.name)).slice(0, 40);
          return [...recent, ...rest].map((sheet) => sheetItem(sheet));
        }
        return fuzzyFilter(query.term, sheets, (sheet) => sheet.name, 60).map(({ item, match }) => sheetItem(item, match.indices));
      }
    }
  }, [commands, currentSheet, onGoToRow, onOpenChange, onOpenSheet, progress, query.mode, query.term, recentSheets, sheets]);

  useEffect(() => {
    setActive(Math.max(0, items.findIndex((item) => !item.disabled)));
  }, [items]);

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  function move(direction: 1 | -1) {
    if (items.length === 0) return;
    let next = active;
    for (let step = 0; step < items.length; step += 1) {
      next = (next + direction + items.length) % items.length;
      if (!items[next]!.disabled) break;
    }
    setActive(next);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      move(event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const item = items[active];
      if (item && !item.disabled) item.run?.();
    }
  }

  const placeholder = query.mode === "commands" ? "Type a command" : query.mode === "goto" ? "Row number" : "Search sheets by name — type ? for help";
  const sectionLabel = query.mode === "commands" ? "Commands" : query.mode === "goto" ? "Go to row" : query.mode === "strings" ? "Strings" : query.mode === "help" ? "Prefixes" : query.term.trim() ? "Sheets" : "Recently opened and sheets with strings";

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="palette-overlay" />
        <Dialog.Content className="palette" aria-describedby={undefined} onOpenAutoFocus={(event) => {
          event.preventDefault();
          const element = inputRef.current;
          element?.focus();
          element?.setSelectionRange(element.value.length, element.value.length);
        }}>
          <Dialog.Title className="visually-hidden">Command palette</Dialog.Title>
          <div className="palette-input">
            <UiIcon icon={query.mode === "commands" ? "chevronRight" : query.mode === "goto" ? "arrowRight" : "search"} size="sm" />
            <input
              ref={inputRef}
              value={input}
              onChange={(event) => setInput(event.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={placeholder}
              aria-label="Command palette"
              aria-controls="palette-list"
              aria-activedescendant={items[active] ? `palette-item-${active}` : undefined}
              spellCheck={false}
            />
          </div>
          <div className="palette-section">{sectionLabel}</div>
          <div className="palette-list" id="palette-list" role="listbox" ref={listRef}>
            {items.length === 0 ? <div className="palette-empty">No results</div> : items.map((item, index) => (
              <div
                key={item.key}
                id={`palette-item-${index}`}
                data-index={index}
                role="option"
                aria-selected={index === active}
                aria-disabled={item.disabled || undefined}
                className={`palette-item${index === active ? " active" : ""}${item.disabled ? " disabled" : ""}`}
                onMouseMove={() => { if (!item.disabled && index !== active) setActive(index); }}
                onClick={() => { if (!item.disabled) item.run?.(); }}
              >
                <UiIcon icon={item.icon} size="sm" className="palette-item-icon" />
                <span className="palette-item-label"><Highlighted text={item.label} indices={item.indices} /></span>
                {item.detail ? <span className="palette-item-detail">{item.detail}</span> : null}
                <span className="spacer" />
                {item.hint}
              </div>
            ))}
          </div>
          <div className="palette-foot">
            <span><kbd>Up</kbd><kbd>Down</kbd> navigate</span>
            <span><kbd>Enter</kbd> open</span>
            <span><kbd>&gt;</kbd> commands</span>
            <span><kbd>:</kbd> go to row</span>
            <span><kbd>#</kbd> strings</span>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
