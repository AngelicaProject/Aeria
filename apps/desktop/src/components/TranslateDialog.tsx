import { memo, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { normalizeCommandError, translationNameSheets, translationRetry, translationStart, translationStatus, translationStop } from "../ipc";
import type { CommandError, ProjectSheetDto, SheetProgressDto, SourceBinding, TranslationStatus, TranslationStop } from "../types";
import type { MessageKey } from "../i18n/translate";
import { buildSheetTree, findSheetMatches, type SheetTreeEntry, type SheetTreeFolder } from "../sheetExplorer";
import { ErrorBanner } from "./ErrorBanner";
import { TranslationRejections } from "./TranslationRejections";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";

type TranslateDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sheets: readonly ProjectSheetDto[];
  progress: ReadonlyMap<string, SheetProgressDto>;
  /** The sheet the editor shows, chosen when the dialog opens with nothing chosen. */
  sheetName: string | null;
  onOpenSettings: () => void;
  /** A run wrote files: sheets and progress should be read again. */
  onFilesChanged: () => void;
  /** Opens a string in the editor. */
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
};

/** How often a running translation's progress is read. */
const POLL_MS = 1000;
/** Most sheets a search lists. */
const SEARCH_LIMIT = 300;

const stopLabels: Readonly<Record<TranslationStop["reason"], MessageKey>> = {
  finished: "translate.stop.finished",
  cancelled: "translate.stop.cancelled",
  usageLimit: "translate.stop.usageLimit",
  signInRequired: "translate.stop.signInRequired",
  failed: "translate.stop.failed",
};

function describeStop(stop: TranslationStop, t: Translate, locale: string): string {
  if (stop.reason === "usageLimit" && stop.resetsAt !== null) {
    return t("translate.stop.usageLimitUntil", { time: new Date(stop.resetsAt * 1000).toLocaleString(locale) });
  }
  if (stop.reason === "failed") return t("translate.stop.failedWith", { message: stop.message });
  return t(stopLabels[stop.reason]);
}

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

/**
 * The scope of a run: whole folders as `folder/`, other chosen sheets by
 * name, and an empty scope when every sheet is chosen.
 */
function scopeOf(entries: readonly SheetTreeEntry[], selected: ReadonlySet<string>, leaves: ReadonlyMap<string, string[]>, total: number): string[] {
  if (selected.size === total) return [];
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
  visit(entries);
  return scope;
}

function Check({ checked, mixed, label, onChange }: { checked: boolean; mixed: boolean; label: string; onChange: (checked: boolean) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = mixed;
  }, [mixed]);
  return <input ref={ref} type="checkbox" aria-label={label} checked={checked} onChange={(event) => onChange(event.target.checked)} />;
}

/** Machine translation of chosen sheets and folders of sheets, and its progress. */
export const TranslateDialog = memo(function TranslateDialog({ open, onOpenChange, sheets, progress, sheetName, onOpenSettings, onFilesChanged, onRevealBinding }: TranslateDialogProps) {
  const { t, locale, formatNumber } = useI18n();
  const { preferences } = usePreferences();
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [query, setQuery] = useState("");
  const [fuzzy, setFuzzy] = useState(false);
  const [nameSheets, setNameSheets] = useState<readonly string[]>([]);
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const wasRunning = useRef(false);

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

  /** Strings of a sheet a run would translate; null while its progress is unknown. */
  const remainingOf = useCallback((name: string): number | null => {
    const entry = progress.get(name);
    if (!entry) return null;
    return entry.strings - entry.translated + (fuzzy ? entry.fuzzy : 0);
  }, [fuzzy, progress]);
  const remainingOfAll = useCallback((names: readonly string[]) => names.reduce((sum, name) => sum + (remainingOf(name) ?? 0), 0), [remainingOf]);

  useEffect(() => {
    if (!open) return;
    translationNameSheets().then(setNameSheets).catch(() => setNameSheets([]));
    setSelected((current) => current.size > 0 || !sheetName || !(remainingOf(sheetName) ?? 1) ? current : new Set([sheetName]));
    // Only when the dialog opens: the choice stays as the translator left it.
  }, [open]);

  const readStatus = useCallback(async () => {
    try { setStatus(await translationStatus()); }
    catch (reason) { setError(normalizeCommandError(reason)); }
  }, []);

  useEffect(() => {
    if (!open) return;
    void readStatus();
    const timer = window.setInterval(() => void readStatus(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [open, readStatus]);

  const running = status?.running ?? false;
  useEffect(() => {
    if (wasRunning.current && !running) onFilesChanged();
    wasRunning.current = running;
  }, [onFilesChanged, running]);

  const toggle = useCallback((names: readonly string[], on: boolean) => {
    setSelected((current) => {
      const next = new Set(current);
      for (const name of names) {
        if (on) next.add(name);
        else next.delete(name);
      }
      return next;
    });
  }, []);
  const choose = useCallback((names: readonly string[]) => toggle(names.filter((name) => (remainingOf(name) ?? 1) > 0), true), [remainingOf, toggle]);
  const toggleFolder = useCallback((path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const allNames = useMemo(() => translatable.map((sheet) => sheet.name), [translatable]);
  const selectedNames = useMemo(() => allNames.filter((name) => selected.has(name)), [allNames, selected]);
  const strings = remainingOfAll(selectedNames);
  const noModel = preferences.translationModel === "";
  const cachedShare = status && status.inputTokens > 0 ? status.cachedTokens / status.inputTokens : null;

  async function start() {
    setError(null);
    try {
      await translationStart(scopeOf(tree.children, selected, leaves, allNames.length), fuzzy, preferences.translationModel, preferences.translationEffort || null);
      await readStatus();
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }

  async function retry(contexts: string[]) {
    setError(null);
    try {
      await translationRetry(contexts, preferences.translationModel, preferences.translationEffort || null);
      await readStatus();
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }

  function remainingLabel(count: number | null) {
    if (count === null) return <span className="translate-remaining muted">{t("common.loading")}</span>;
    if (count === 0) return <span className="translate-remaining done">{t("translate.done")}</span>;
    return <span className="translate-remaining">{formatNumber(count)}</span>;
  }

  function renderEntries(entries: readonly SheetTreeEntry[], depth: number): ReactNode[] {
    const rows: ReactNode[] = [];
    for (const entry of entries) {
      const indent = { paddingInlineStart: `${depth * 16 + 6}px` };
      if (entry.kind === "leaf") {
        const name = entry.sheet.name;
        rows.push(
          <label key={name} className="translate-row" style={indent}>
            <span className="translate-twisty" />
            <Check checked={selected.has(name)} mixed={false} label={name} onChange={(on) => toggle([name], on)} />
            <span className="translate-name">{entry.name}</span>
            {remainingLabel(remainingOf(name))}
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
          <Check checked={chosen > 0 && chosen === names.length} mixed={chosen > 0 && chosen < names.length} label={entry.path} onChange={(on) => toggle(names, on)} />
          <button className="translate-name translate-folder" type="button" onClick={() => toggleFolder(entry.path)}>{entry.name}</button>
          {remainingLabel(names.some((name) => !progress.has(name)) ? null : remainingOfAll(names))}
        </div>,
      );
      if (isOpen) rows.push(...renderEntries(entry.children, depth + 1));
    }
    return rows;
  }

  const matches = useMemo(() => query.trim() ? findSheetMatches(translatable, query) : null, [query, translatable]);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog translate-dialog" aria-describedby="translate-description">
          <Dialog.Title className="dialog-title">{t("translate.title")}</Dialog.Title>
          <p id="translate-description" className="dialog-description">{t("translate.description")}</p>
          {error ? <ErrorBanner title={t("translate.error")} error={error} onDismiss={() => setError(null)} /> : null}

          <div className="translate-layout">
            <div className="translate-picker">
              <input className="input" type="search" placeholder={t("translate.search")} aria-label={t("translate.search")} value={query} disabled={running} onChange={(event) => setQuery(event.target.value)} />
              <div className="translate-presets">
                <button className="button button-ghost translate-preset" type="button" disabled={running} onClick={() => choose(nameSheets.filter((name) => allNames.includes(name)))}>{t("translate.preset.names")}</button>
                <button className="button button-ghost translate-preset" type="button" disabled={running} onClick={() => choose(allNames.filter((name) => name.startsWith("quest/")))}>{t("translate.preset.quests")}</button>
                <button className="button button-ghost translate-preset" type="button" disabled={running} onClick={() => choose(allNames)}>{t("translate.preset.untranslated")}</button>
                <button className="button button-ghost translate-preset" type="button" disabled={running || selected.size === 0} onClick={() => setSelected(new Set())}>{t("translate.preset.none")}</button>
              </div>
              <fieldset className="translate-tree" disabled={running} aria-label={t("translate.sheets")}>
                {matches
                  ? matches.slice(0, SEARCH_LIMIT).map((match) => (
                      <label key={match.sheet.name} className="translate-row">
                        <span className="translate-twisty" />
                        <Check checked={selected.has(match.sheet.name)} mixed={false} label={match.sheet.name} onChange={(on) => toggle([match.sheet.name], on)} />
                        <span className="translate-name">{match.basename}{match.breadcrumb ? <span className="muted"> · {match.breadcrumb}</span> : null}</span>
                        {remainingLabel(remainingOf(match.sheet.name))}
                      </label>
                    ))
                  : renderEntries(tree.children, 0)}
                {matches && matches.length > SEARCH_LIMIT ? <p className="field-hint">{t("translate.moreMatches", { shown: SEARCH_LIMIT, count: matches.length })}</p> : null}
                {matches && matches.length === 0 ? <p className="field-hint">{t("translate.noMatches")}</p> : null}
              </fieldset>
            </div>

            <div className="translate-side">
              <p className="translate-summary">{t("translate.selection", { sheets: formatNumber(selectedNames.length), strings: formatNumber(strings) })}</p>
              <label className="field field-inline">
                <input type="checkbox" checked={fuzzy} disabled={running} onChange={(event) => setFuzzy(event.target.checked)} />
                <span>{t("translate.fuzzy")}</span>
              </label>
              <p className="field-hint">{t("translate.namesFirst")}</p>
              {noModel ? (
                <p className="export-warning">
                  <UiIcon icon="circleAlert" size="xs" />{t("translate.noModel")}{" "}
                  <button className="link-button" type="button" onClick={onOpenSettings}>{t("translate.openSettings")}</button>
                </p>
              ) : <p className="field-hint">{t("translate.model", { model: preferences.translationModel })}</p>}

              {status && (running || status.stop) ? (
                <div className="translate-progress" aria-live="polite">
                  <div className="meter" aria-hidden="true">
                    <span className="meter-translated" style={{ width: `${status.strings > 0 ? Math.min(1, (status.written + status.rejected) / status.strings) * 100 : 0}%` }} />
                  </div>
                  <p className="export-facts">
                    <span>{t("translate.progress", { written: formatNumber(status.written), strings: formatNumber(status.strings) })}</span>
                    {status.rejected > 0 ? <span>{t("translate.rejected", { count: status.rejected })}</span> : null}
                    <span>{t("translate.tokens", { input: formatNumber(status.inputTokens), output: formatNumber(status.outputTokens) })}</span>
                    {cachedShare !== null ? <span>{t("translate.cached", { percent: Math.round(cachedShare * 100) })}</span> : null}
                    {running ? <span>{t("translate.pace", { pace: status.pace })}</span> : null}
                  </p>
                  {status.message ? <p className="field-hint">{status.message}</p> : null}
                  {status.stop ? <p className="field-hint"><strong>{describeStop(status.stop, t, locale)}</strong></p> : null}
                  {status.rejections.length > 0 ? (
                    <TranslationRejections rejections={status.rejections} total={status.rejected} running={running} onReveal={onRevealBinding ? (binding) => { onOpenChange(false); onRevealBinding(binding); } : undefined} onRetry={(contexts) => void retry(contexts)} />
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>

          <div className="dialog-actions">
            <button className="button button-ghost" type="button" onClick={() => onOpenChange(false)}>{t(running ? "translate.hide" : "common.close")}</button>
            {running ? (
              <button className="button button-secondary" type="button" onClick={() => void translationStop()}>{t("translate.stopRun")}</button>
            ) : (
              <button className="button button-primary" type="button" disabled={noModel || strings === 0} onClick={() => void start()}>
                {t(status?.stop && status.stop.reason !== "finished" ? "translate.continue" : "translate.start")}
              </button>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
