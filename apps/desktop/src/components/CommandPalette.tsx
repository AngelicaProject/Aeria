import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { palettePrefixes, parsePaletteQuery, parseRowTarget, type RowTarget } from "../commandPalette";
import { fuzzyFilter } from "../fuzzy";
import { normalizeCommandError, projectSearch } from "../ipc";
import type { ProjectSheetDto, SearchResultDto, SheetProgressDto, SourceBinding } from "../types";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

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
  /** Opens a string found by `#` in the editor. */
  onRevealString: (binding: SourceBinding) => void;
  /** Shows the Search tool with the text of a `#` query. */
  onOpenSearch: (text: string) => void;
};

/** Strings a `#` query lists. */
const STRING_RESULTS = 30;
/** How long typing pauses before a `#` query searches. */
const STRING_DEBOUNCE_MS = 250;

function Highlighted({ text, indices = [] }: { text: string; indices?: number[] | undefined }) {
  if (indices.length === 0) return <>{text}</>;
  const marked = new Set(indices);
  return <>{[...text].map((character, index) => marked.has(index) ? <mark key={index}>{character}</mark> : character)}</>;
}

/** Mount with a fresh `key` per opening so `initialInput` seeds the query. */
export function CommandPalette({ open, initialInput, onOpenChange, commands, sheets, progress, recentSheets, currentSheet, onOpenSheet, onGoToRow, onRevealString, onOpenSearch }: CommandPaletteProps) {
  const { t } = useI18n();
  const [input, setInput] = useState(initialInput);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const query = parsePaletteQuery(input);
  const stringTerm = query.mode === "strings" ? query.term.trim() : "";
  const [strings, setStrings] = useState<{ term: string; result: SearchResultDto | null; error: string | null }>({ term: "", result: null, error: null });

  useEffect(() => {
    if (stringTerm.length < 2) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      projectSearch({ text: stringTerm, kind: "text", caseSensitive: false, fields: ["translation", "source"], paths: [], contexts: [], states: [], check: "any" })
        .then((result) => { if (!cancelled) setStrings({ term: stringTerm, result, error: null }); })
        .catch((error: unknown) => { if (!cancelled) setStrings({ term: stringTerm, result: null, error: normalizeCommandError(error).message }); });
    }, STRING_DEBOUNCE_MS);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [stringTerm]);

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
        if (!currentSheet) return [{ key: "no-sheet", icon: "info", label: t("palette.openSheetFirst"), disabled: true }];
        if (!query.term) return [{ key: "hint", icon: "info", label: t("palette.rowPrompt", { sheet: currentSheet }), detail: t("palette.rowFormat"), disabled: true }];
        if (!target) return [{ key: "invalid", icon: "circleAlert", label: t("palette.invalidRow"), detail: t("palette.invalidRowHint"), disabled: true }];
        const coordinate = `${target.rowId}:${target.subrowId}${target.columnIndex === null ? "" : ` · ${t("common.column", { column: String(target.columnIndex) })}`}`;
        return [{ key: "goto", icon: "arrowRight", label: t("palette.goTo", { coordinate }), detail: currentSheet, run: close(() => onGoToRow(target)) }];
      }
      case "strings": {
        if (stringTerm.length < 2) return [{ key: "strings-hint", icon: "search", label: t("palette.searchStrings"), detail: t("palette.searchStringsHint"), disabled: true }];
        if (strings.error !== null && strings.term === stringTerm) return [{ key: "strings-error", icon: "circleAlert", label: strings.error, disabled: true }];
        if (strings.result === null || strings.term !== stringTerm) return [{ key: "strings-searching", icon: "search", label: t("palette.searchStringsFor", { term: stringTerm }), detail: t("search.searching"), disabled: true }];
        const found = strings.result;
        const items: PaletteItem[] = found.hits.slice(0, STRING_RESULTS).map((hit) => {
          const binding = hit.binding;
          return {
            key: `${hit.path}|${hit.context}`,
            icon: hit.translation ? "languages" : "circleDot",
            label: (hit.translation || hit.source).replace(/\s+/g, " ").slice(0, 120),
            detail: binding ? `${binding.sheetName} ${binding.rowId}:${binding.subrowId}${hit.translation ? ` · ${hit.source.replace(/\s+/g, " ").slice(0, 60)}` : ""}` : hit.context,
            disabled: binding === null,
            run: binding ? close(() => onRevealString(binding)) : undefined,
          };
        });
        if (found.total === 0) items.push({ key: "strings-none", icon: "info", label: t("search.none"), disabled: true });
        items.push({ key: "strings-all", icon: "search", label: t("palette.searchInPanel", { count: found.total }), run: close(() => onOpenSearch(stringTerm)) });
        return items;
      }
      case "help":
        return palettePrefixes.filter((entry) => entry.mode !== "help").map((entry): PaletteItem => ({
          key: entry.prefix,
          icon: "info",
          label: `${entry.prefix}  ${t(entry.label)}`,
          run: () => { setInput(entry.prefix); inputRef.current?.focus(); },
        })).concat([{ key: "sheets", icon: "table2", label: t("palette.noPrefix"), run: () => { setInput(""); inputRef.current?.focus(); } }]);
      case "sheets": {
        const sheetItem = (sheet: ProjectSheetDto, indices?: number[]): PaletteItem => {
          const sheetProgress = progress.get(sheet.name);
          const detail = sheet.translatableCellCount === 0
            ? t("palette.noStrings")
            : t("palette.sheetProgress", { translated: sheetProgress?.translated ?? 0, total: sheet.translatableCellCount });
          return { key: sheet.name, icon: "table2", label: sheet.name, indices, detail, hint: sheet.name === currentSheet ? <span className="palette-badge">{t("palette.openBadge")}</span> : undefined, run: close(() => onOpenSheet(sheet.name)) };
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
  }, [commands, currentSheet, onGoToRow, onOpenChange, onOpenSearch, onOpenSheet, onRevealString, progress, query.mode, query.term, recentSheets, sheets, stringTerm, strings, t]);

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

  const placeholder = t(query.mode === "commands" ? "palette.placeholder.commands" : query.mode === "goto" ? "palette.placeholder.goto" : "palette.placeholder.sheets");
  const sectionLabel = t(query.mode === "commands" ? "palette.section.commands" : query.mode === "goto" ? "palette.section.goto" : query.mode === "strings" ? "palette.section.strings" : query.mode === "help" ? "palette.section.help" : query.term.trim() ? "palette.section.sheets" : "palette.section.recent");

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
          <Dialog.Title className="visually-hidden">{t("palette.title")}</Dialog.Title>
          <div className="palette-input">
            <UiIcon icon={query.mode === "commands" ? "chevronRight" : query.mode === "goto" ? "arrowRight" : "search"} size="sm" />
            <input
              ref={inputRef}
              value={input}
              onChange={(event) => setInput(event.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={placeholder}
              aria-label={t("palette.title")}
              aria-controls="palette-list"
              aria-activedescendant={items[active] ? `palette-item-${active}` : undefined}
              spellCheck={false}
            />
          </div>
          <div className="palette-section">{sectionLabel}</div>
          <div className="palette-list" id="palette-list" role="listbox" ref={listRef}>
            {items.length === 0 ? <div className="palette-empty">{t("palette.noResults")}</div> : items.map((item, index) => (
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
            <span><kbd>Up</kbd><kbd>Down</kbd> {t("palette.foot.navigate")}</span>
            <span><kbd>Enter</kbd> {t("palette.foot.open")}</span>
            <span><kbd>&gt;</kbd> {t("palette.foot.commands")}</span>
            <span><kbd>:</kbd> {t("palette.foot.goto")}</span>
            <span><kbd>#</kbd> {t("palette.foot.strings")}</span>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
