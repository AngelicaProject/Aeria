import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { Effect, getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen } from "@tauri-apps/api/event";
import {
  angelicaDraft,
  closeProject,
  gitPendingChanges,
  normalizeCommandError,
  pageTranslationRows,
  setTranslationNote,
  setTranslationReviewState,
  setTranslationTarget,
  translationProgress,
} from "../ipc";
import { bindingKey, rowKey } from "../binding";
import type {
  TranslationAppliedDto,
  EditorContextDto,
  CommandError,
  ProjectSheetDto,
  ProjectSummaryDto,
  ReviewState,
  SheetProgressDto,
  SourceBinding,
  TranslationCellDto,
  TranslationOverlayDto,
  TranslationRowCursorDto,
  TranslationRowDto,
  UnitChangeDto,
} from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { ConfirmDialog } from "./ConfirmDialog";
import { ActivityRail } from "./ActivityRail";
import { IconButton } from "../ui/primitives/IconButton";
import { formatSheetCount } from "../sheetExplorer";
import { BottomPanel } from "./BottomPanel";
import { DocumentTabs, type DocumentTab } from "./DocumentTabs";
import { DockPanel } from "./DockPanel";
import { ResizeHandle } from "./ResizeHandle";
import { SettingsDialog, type SettingsSection } from "./SettingsDialog";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import type { CheckpointBaseline } from "./TranslationEditor";
import { SheetSidebar } from "./SheetSidebar";
import { StatusBar } from "./StatusBar";
import { TranslationEditor, type CellDraft, type CellMutation, type TranslationEditorHandle } from "./TranslationEditor";
import { TranslationList } from "./TranslationList";
import { WindowChrome } from "./WindowChrome";
import type { ApplicationMenuDefinition } from "./ApplicationMenu";
import { ExportDialog } from "./ExportDialog";
import { ProjectGuideDialog, type ProjectGuideTab } from "./ProjectGuideDialog";
import { WorkbenchToolDock, toolTitle, type WorkbenchTool } from "./WorkbenchToolDock";
import { CommitView } from "./GitHistory";
import type { GitCommitDto } from "../types";
import { detachedPanelTitle, type DetachedPanel } from "./DetachedToolWindow";
import { displayPathName } from "../pathDisplay";
import { initialWorkbenchLayout, reduceWorkbenchLayout } from "../ui/layout";
import {
  adjacentOccurrence,
  emptyOccurrenceFilter,
  filterOccurrences,
  flattenTranslationRows,
  type OccurrenceFilter,
  type TranslationOccurrenceView,
} from "../translationOccurrences";
import { initialDocumentTabsState, reduceDocumentTabs, type DocumentTabsState } from "../documentTabs";
import { initialDockLayout, reduceDockLayout, type DockRegion } from "../dockLayout";
import { createLoadProgress } from "../loadProgress";
import { hasWindowsBackdrop } from "../ui/theme/windowBackdrop";
import { useTheme } from "../ui/theme/theme";
import { themeRegistry } from "../ui/theme/registry";
import { usePreferences } from "../ui/preferences";
import type { RowTarget } from "../commandPalette";
import { UiIcon } from "../ui/primitives/UiIcon";
import { reportUnsavedDraft } from "../ui/appUpdateStore";
import { useI18n, type MessageKey, type Translate } from "../ui/i18n";

/** The Rust page bound (`MAX_TRANSLATION_PAGE_SIZE`); a sheet streams in these pages. */
const PAGE_SIZE = 256;
/**
 * A sheet that loads within this time appears in one piece. A slower sheet
 * shows what has loaded by then and the rest once it is complete, so the
 * workbench re-renders at most twice per load rather than once per batch.
 */
const FIRST_COMMIT_DELAY_MS = 150;

/** One sheet load. Pages are read until the sheet is complete. */
type SheetLoader = {
  generation: number;
  sheetName: string;
  /** Keeps the current rows on screen and swaps them in once complete. */
  refresh: boolean;
  byRowKey: Map<string, TranslationRowDto>;
  uncommitted: TranslationRowDto[];
  committed: boolean;
  startedAt: number;
  /** Strings read so far, including rows not yet handed to the list. */
  loadedStrings: number;
  /** Whether a row has been selected in this sheet, by the loader or the user. */
  selected: boolean;
  /** Last scanned source coordinate; every row up to it is loaded. */
  scannedThrough: TranslationRowCursorDto | null;
  complete: boolean;
  failed: boolean;
};

/** A string to open once its sheet has loaded far enough to contain it. */
type PendingReveal = { sheetName: string; target: RowTarget };

type EditorError = {
  title: string;
  error: CommandError;
  tone?: "error" | "warning";
};

const RECENT_SHEET_LIMIT = 8;

type DiscardRequest = {
  message: string;
  resolve: (confirmed: boolean) => void;
};

type EditorShellProps = {
  project: ProjectSummaryDto;
  applicationWarning: CommandError | null;
  onDismissApplicationWarning: () => void;
  onShowDetachedUnits: () => void;
  onClosed: () => void;
};

const bottomPanelIds = new Set(["tasks", "gitChanges", "diagnostics"]);

function cursorForRow(row: TranslationRowDto): TranslationRowCursorDto {
  return { sheetName: row.sheetName, rowId: row.rowId, subrowId: row.subrowId };
}

function targetCell(row: TranslationRowDto, target: RowTarget): TranslationCellDto | undefined {
  return row.cells.find((cell) => target.columnIndex === null || cell.sourceBinding.columnIndex === target.columnIndex) ?? row.cells[0];
}

function patchRow(row: TranslationRowDto, patches: ReadonlyMap<string, TranslationOverlayDto>): TranslationRowDto {
  let changed = false;
  const cells = row.cells.map((cell) => {
    const translation = patches.get(bindingKey(cell.sourceBinding));
    if (!translation) return cell;
    changed = true;
    return { ...cell, translation };
  });
  return changed ? { ...row, cells } : row;
}

/** True once the loader has scanned the source past `target`. */
function scannedPast(loader: SheetLoader, target: RowTarget): boolean {
  if (loader.complete || loader.failed) return true;
  const last = loader.scannedThrough;
  return last !== null && (last.rowId > target.rowId || (last.rowId === target.rowId && last.subrowId >= target.subrowId));
}

function panelTitle(panelId: string | null): MessageKey {
  if (panelId === "sheets") return "workbench.panel.sheets";
  if (panelId === "search" || panelId === "ai" || panelId === "git") return toolTitle(panelId);
  return "workbench.panel.generic";
}

const moveTargetLabels: Readonly<Record<DockRegion, MessageKey>> = {
  left: "workbench.moveLeft",
  right: "workbench.moveRight",
  bottom: "workbench.moveBottom",
};

function cellCoordinates(sheet: string, target: { rowId: number; subrowId: number }): Parameters<Translate>[1] {
  return { sheet, row: String(target.rowId), subrow: String(target.subrowId) };
}

/**
 * A callback with a stable identity that always runs the latest `callback`, so
 * memoized panels do not re-render when only the closure changed.
 */
function useStableCallback<Args extends unknown[], Result>(callback: (...args: Args) => Result): (...args: Args) => Result {
  const ref = useRef(callback);
  ref.current = callback;
  return useCallback((...args: Args) => ref.current(...args), []);
}

/** Resolves once the browser has painted, or after a short fallback when frames are paused. */
function afterPaint(): Promise<void> {
  return new Promise((resolve) => {
    const fallback = window.setTimeout(resolve, 50);
    requestAnimationFrame(() => window.setTimeout(() => {
      window.clearTimeout(fallback);
      resolve();
    }, 0));
  });
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT";
}

export function EditorShell({
  project,
  applicationWarning,
  onDismissApplicationWarning,
  onShowDetachedUnits,
  onClosed,
}: EditorShellProps) {
  const firstSheetName = project.sheets.find((sheet) => sheet.translatableCellCount > 0)?.name ?? project.sheets[0]?.name ?? null;
  const sheetsByName = useMemo(() => {
    const byName = new Map<string, ProjectSheetDto>();
    for (const sheet of project.sheets) byName.set(sheet.name, sheet);
    return byName;
  }, [project.sheets]);
  const projectName = displayPathName(project.repositoryRoot);
  const { theme, setThemeId } = useTheme();
  const { t } = useI18n();

  const [selectedSheetName, setSelectedSheetName] = useState<string | null>(firstSheetName);
  const [loadedSheetName, setLoadedSheetName] = useState<string | null>(null);
  const [rows, setRows] = useState<TranslationRowDto[]>([]);
  const [selectedRowCursor, setSelectedRowCursor] = useState<TranslationRowCursorDto | null>(null);
  const [selectedBinding, setSelectedBinding] = useState<SourceBinding | null>(null);
  /** True until the first page of the selected sheet is on screen. */
  const [sheetLoading, setSheetLoading] = useState(false);
  /** True while the rest of the sheet streams in behind the visible rows. */
  const [sheetStreaming, setSheetStreaming] = useState(false);
  const [loadProgress] = useState(createLoadProgress);
  const [mutations, setMutations] = useState<CellMutation[]>([]);
  const [dirty, setDirty] = useState(false);
  const [closing, setClosing] = useState(false);
  const [editorError, setEditorError] = useState<EditorError | null>(null);
  const [discardRequest, setDiscardRequest] = useState<DiscardRequest | null>(null);

  // An application update waits for drafts and saves in flight.
  const unsavedDraft = dirty || mutations.length > 0;
  useEffect(() => reportUnsavedDraft(unsavedDraft), [unsavedDraft]);
  useEffect(() => () => reportUnsavedDraft(false), []);
  const [layout, dispatchLayout] = useReducer(reduceWorkbenchLayout, initialWorkbenchLayout);
  const [documentTabs, setDocumentTabs] = useState<DocumentTabsState>(() => firstSheetName ? reduceDocumentTabs(initialDocumentTabsState, { type: "openSheet", sheetName: firstSheetName }) : initialDocumentTabsState);
  const [quickFindSignal, setQuickFindSignal] = useState(0);
  const [hideEmptySheets, setHideEmptySheets] = useState(true);
  const [sheetFilterOpen, setSheetFilterOpen] = useState(false);
  const [revealSheetSignal, setRevealSheetSignal] = useState(0);
  const [collapseSheetsSignal, setCollapseSheetsSignal] = useState(0);
  const [detachedPanel, setDetachedPanel] = useState<DetachedPanel | null>(null);
  const [dockLayoutState, setDockLayoutState] = useState(initialDockLayout);
  const [activeTool, setActiveTool] = useState<WorkbenchTool>("git");
  /** Bumps when dialogs that write project files close, so Git views reload. */
  const [projectRevision, setProjectRevision] = useState(0);
  const [workspaceRevision, setWorkspaceRevision] = useState(0);
  const [lensFilter, setLensFilter] = useState<OccurrenceFilter>(emptyOccurrenceFilter);
  const [progress, setProgress] = useState<readonly SheetProgressDto[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [guide, setGuide] = useState<{ open: boolean; tab: ProjectGuideTab }>({ open: false, tab: "glossary" });
  const openGuide = useCallback((tab: ProjectGuideTab) => setGuide({ open: true, tab }), []);
  const setGuideOpen = useCallback((open: boolean) => {
    setGuide((current) => ({ ...current, open }));
    if (!open) setProjectRevision((current) => current + 1);
  }, []);
  const [exportOpen, setExportOpenState] = useState(false);
  const setExportOpen = useCallback((open: boolean) => {
    setExportOpenState(open);
    if (!open) setProjectRevision((current) => current + 1);
  }, []);
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("appearance");
  const [palette, setPalette] = useState<{ open: boolean; input: string; key: number }>({ open: false, input: "", key: 0 });
  const [recentSheets, setRecentSheets] = useState<string[]>([]);
  const [pendingChanges, setPendingChanges] = useState<UnitChangeDto[] | null>(null);
  const { preferences } = usePreferences();
  const leftDockOpen = layout.regions.leftDock.visible;
  const rightDockOpen = layout.regions.rightDock.visible;
  const bottomOpen = layout.regions.bottomPanel.visible;
  const requestGeneration = useRef(0);
  const hasDirtyDraft = useRef(false);
  const allowWindowClose = useRef(false);
  const discardRequestRef = useRef<DiscardRequest | null>(null);
  const mutationKeys = useRef(new Set<string>());
  const editorRef = useRef<TranslationEditorHandle>(null);
  const pendingAdvance = useRef<string | null>(null);
  const focusTargetRequest = useRef(false);
  const loaderRef = useRef<SheetLoader | null>(null);
  const pendingReveal = useRef<PendingReveal | null>(null);
  /** Overlays saved while a sheet streams, applied to pages read before the save. */
  const overlayPatches = useRef(new Map<string, TranslationOverlayDto>());
  const selectedRowKeyRef = useRef<string | null>(null);
  selectedRowKeyRef.current = selectedRowCursor ? rowKey(selectedRowCursor) : null;
  const lensFilterRef = useRef(lensFilter);
  lensFilterRef.current = lensFilter;

  const selectedRow = useMemo(
    () => selectedRowCursor
      ? rows.find((row) => row.rowId === selectedRowCursor.rowId && row.subrowId === selectedRowCursor.subrowId && row.sheetName === selectedRowCursor.sheetName) ?? null
      : null,
    [rows, selectedRowCursor],
  );
  const allOccurrences = useMemo(() => flattenTranslationRows(rows), [rows]);
  const visibleOccurrences = useMemo(() => filterOccurrences(allOccurrences, lensFilter), [allOccurrences, lensFilter]);
  const sheetHasNoRows = selectedSheetName !== null && loadedSheetName === selectedSheetName && !sheetLoading && !sheetStreaming && rows.length === 0;

  const progressBySheet = useMemo(() => new Map(progress.map((entry) => [entry.sheetName, entry])), [progress]);
  const projectProgress = useMemo(() => {
    const total = project.sheets.reduce((sum, sheet) => sum + sheet.translatableCellCount, 0);
    const translated = progress.reduce((sum, entry) => sum + entry.translated, 0);
    return total > 0 ? { translated, total } : null;
  }, [progress, project.sheets]);
  const selectedSheet = selectedSheetName ? sheetsByName.get(selectedSheetName) ?? null : null;
  const selectedSheetProgress = useMemo(() => selectedSheet && selectedSheet.translatableCellCount > 0
    ? {
        translated: progressBySheet.get(selectedSheet.name)?.translated ?? 0,
        reviewed: progressBySheet.get(selectedSheet.name)?.reviewed ?? 0,
        total: selectedSheet.translatableCellCount,
      }
    : null, [progressBySheet, selectedSheet]);

  const showError = useCallback((title: string, error: unknown) => {
    setEditorError({ title, error: normalizeCommandError(error) });
  }, []);

  useEffect(() => {
    let cancelled = false;
    translationProgress()
      .then((next) => { if (!cancelled) setProgress(next); })
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [workspaceRevision]);

  const handleDirtyChange = useCallback((nextDirty: boolean) => {
    hasDirtyDraft.current = nextDirty;
    setDirty(nextDirty);
    const activeDocumentId = documentTabs.activeId;
    if (activeDocumentId) {
      setDocumentTabs((current) => reduceDocumentTabs(current, { type: "setDirty", id: activeDocumentId, dirty: nextDirty }));
    }
  }, [documentTabs.activeId]);

  const applyOverlay = useCallback((sourceBinding: TranslationCellDto["sourceBinding"], translation: TranslationOverlayDto) => {
    const targetKey = bindingKey(sourceBinding);
    overlayPatches.current.set(targetKey, translation);
    setWorkspaceRevision((current) => current + 1);
    setRows((current) => current.map((row) => {
      let changed = false;
      const cells = row.cells.map((cell) => {
        if (bindingKey(cell.sourceBinding) !== targetKey) return cell;
        changed = true;
        return { ...cell, translation };
      });
      return changed ? { ...row, cells } : row;
    }));
  }, []);

  /** Selects one cell on behalf of the loader, dropping a draft only when the row changes. */
  const selectLoadedCell = useCallback((loader: SheetLoader, row: TranslationRowDto | undefined, cell: TranslationCellDto | undefined) => {
    loader.selected = true;
    if (row && rowKey(row) !== selectedRowKeyRef.current) {
      hasDirtyDraft.current = false;
      setDirty(false);
    }
    setSelectedRowCursor(row ? cursorForRow(row) : null);
    setSelectedBinding(cell?.sourceBinding ?? null);
  }, []);

  /** Hands streamed rows to the list, patched with overlays saved since they were read. */
  const commitRows = useCallback((loader: SheetLoader) => {
    if (loader.refresh && !loader.complete) return;
    if (loader.committed && loader.uncommitted.length === 0) return;
    const patches = overlayPatches.current;
    const batch = patches.size === 0 ? loader.uncommitted : loader.uncommitted.map((row) => patchRow(row, patches));
    const first = !loader.committed;
    loader.uncommitted = [];
    loader.committed = true;
    if (first) {
      setRows(batch);
      setLoadedSheetName(loader.sheetName);
      setSheetLoading(false);
    } else {
      setRows((current) => current.concat(batch));
    }

    if (loader.refresh) {
      const selected = selectedRowKeyRef.current;
      if (selected === null || !loader.byRowKey.has(selected)) selectLoadedCell(loader, batch[0], batch[0]?.cells[0]);
      return;
    }
    // A string being revealed is selected once it arrives, not the top row first.
    if (pendingReveal.current?.sheetName === loader.sheetName) {
      if (first) {
        selectLoadedCell(loader, undefined, undefined);
        loader.selected = false;
      }
      return;
    }
    // Leading pages may hold no translatable rows; select the first row that arrives.
    if (first) {
      selectLoadedCell(loader, batch[0], batch[0]?.cells[0]);
      loader.selected = batch.length > 0;
    } else if (!loader.selected && batch[0]) {
      selectLoadedCell(loader, batch[0], batch[0].cells[0]);
    }
  }, [selectLoadedCell]);

  /** Opens the pending string once the loader has read far enough to know whether it exists. */
  const settleReveal = useCallback((loader: SheetLoader) => {
    const reveal = pendingReveal.current;
    if (!reveal || reveal.sheetName !== loader.sheetName || loader.generation !== requestGeneration.current) return;
    const { target } = reveal;
    const row = loader.byRowKey.get(rowKey({ sheetName: loader.sheetName, rowId: target.rowId, subrowId: target.subrowId }));
    if (loader.refresh ? !loader.complete : !row && !scannedPast(loader, target)) return;
    pendingReveal.current = null;
    commitRows(loader);
    const cell = row ? targetCell(row, target) : undefined;
    if (row && cell) {
      selectLoadedCell(loader, row, cell);
      setEditorError(null);
      const key = bindingKey(cell.sourceBinding);
      const occurrence = flattenTranslationRows([row]).filter((candidate) => bindingKey(candidate.binding) === key);
      if (filterOccurrences(occurrence, lensFilterRef.current).length === 0) setLensFilter(emptyOccurrenceFilter);
      return;
    }
    if (loader.failed) return;
    setEditorError({
      title: t("workbench.error.stringNotFound"),
      tone: "warning",
      error: { code: "rowNotTranslatable", message: t("workbench.error.rowNotTranslatable", cellCoordinates(loader.sheetName, target)) },
    });
    if (!loader.selected) {
      const first = loader.byRowKey.values().next().value;
      selectLoadedCell(loader, first, first?.cells[0]);
    }
  }, [commitRows, selectLoadedCell, t]);

  /**
   * Loads a whole sheet. The first page shows immediately and the rest streams
   * in behind it, so filtering, navigation, and reveal always see the full
   * sheet once it completes. `refresh` reloads the open sheet in place.
   */
  const beginSheetLoad = useCallback(async (sheetName: string, options: { immediate?: boolean; reveal?: RowTarget | null; refresh?: boolean } = {}) => {
    const { immediate = true, reveal = null, refresh = false } = options;
    const generation = ++requestGeneration.current;
    pendingAdvance.current = null;
    overlayPatches.current = new Map();
    pendingReveal.current = reveal ? { sheetName, target: reveal } : null;
    const loader: SheetLoader = {
      generation,
      sheetName,
      refresh,
      byRowKey: new Map(),
      uncommitted: [],
      committed: false,
      selected: false,
      startedAt: performance.now(),
      loadedStrings: 0,
      scannedThrough: null,
      complete: false,
      failed: false,
    };
    loaderRef.current = loader;
    loadProgress.set(0);
    const resetForSheet = () => {
      setSelectedSheetName(sheetName);
      setSheetStreaming(true);
      if (refresh) return;
      setDirty(false);
      hasDirtyDraft.current = false;
      setSheetLoading(true);
      setEditorError(null);
    };
    if (immediate) flushSync(resetForSheet);
    else resetForSheet();
    if (!refresh) setRecentSheets((current) => [sheetName, ...current.filter((name) => name !== sheetName)].slice(0, RECENT_SHEET_LIMIT));
    // Let the selection and loading state paint before the new rows render,
    // so a click gets feedback even when a small sheet loads within a frame.
    const painted = refresh ? null : afterPaint();

    let after: TranslationRowCursorDto | null = null;
    try {
      do {
        const page = await pageTranslationRows(sheetName, after, PAGE_SIZE);
        if (generation !== requestGeneration.current) return;
        for (const row of page.rows) {
          loader.byRowKey.set(rowKey(row), row);
          loader.uncommitted.push(row);
          loader.loadedStrings += row.cells.length;
        }
        loadProgress.set(loader.loadedStrings);
        after = page.nextAfter;
        loader.scannedThrough = after;
        loader.complete = after === null;
        if (painted && !loader.committed) {
          await painted;
          if (generation !== requestGeneration.current) return;
        }
        if (loader.complete || (!loader.committed && performance.now() - loader.startedAt >= FIRST_COMMIT_DELAY_MS)) commitRows(loader);
        settleReveal(loader);
      } while (after !== null);
    } catch (error) {
      if (generation !== requestGeneration.current) return;
      loader.failed = true;
      if (pendingReveal.current?.sheetName === sheetName) pendingReveal.current = null;
      if (!refresh) commitRows(loader);
      showError(t("workbench.error.loadSheet"), error);
    } finally {
      if (generation === requestGeneration.current) {
        setSheetLoading(false);
        setSheetStreaming(false);
      }
    }
  }, [commitRows, loadProgress, settleReveal, showError, t]);

  useEffect(() => {
    if (firstSheetName) void beginSheetLoad(firstSheetName, { immediate: false });
  }, [beginSheetLoad, firstSheetName]);

  const requestDiscardConfirmation = useCallback((message: string): Promise<boolean> => {
    if (!hasDirtyDraft.current) return Promise.resolve(true);
    if (discardRequestRef.current) return Promise.resolve(false);
    return new Promise((resolve) => {
      const request = { message, resolve };
      discardRequestRef.current = request;
      setDiscardRequest(request);
    });
  }, []);

  const resolveDiscardConfirmation = useCallback((confirmed: boolean) => {
    const request = discardRequestRef.current;
    discardRequestRef.current = null;
    setDiscardRequest(null);
    request?.resolve(confirmed);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onCloseRequested((event) => {
        if (allowWindowClose.current) {
          allowWindowClose.current = false;
          return;
        }
        if (!hasDirtyDraft.current) return;
        event.preventDefault();
        void (async () => {
          if (!(await requestDiscardConfirmation(t("workbench.discard.closeApp")))) return;
          allowWindowClose.current = true;
          try {
            await getCurrentWindow().close();
          } catch {
            allowWindowClose.current = false;
          }
        })();
      })
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [requestDiscardConfirmation, t]);

  const handleSheetSelect = useCallback(async (sheetName: string, pin = false) => {
    if (sheetName === selectedSheetName) {
      const activeId = documentTabs.activeId;
      if (pin && activeId) setDocumentTabs((current) => reduceDocumentTabs(current, { type: "pin", id: activeId }));
      return;
    }
    if (!(await requestDiscardConfirmation(t("workbench.discard.changeSheet")))) return;
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "openSheet", sheetName, pin }));
    void beginSheetLoad(sheetName);
  }, [beginSheetLoad, documentTabs.activeId, requestDiscardConfirmation, selectedSheetName, t]);

  const handleDocumentSelect = useCallback(async (documentId: string) => {
    const document = documentTabs.tabs.find((tab) => tab.id === documentId);
    if (!document || document.id === documentTabs.activeId) return;
    if (!(await requestDiscardConfirmation(t("workbench.discard.changeSheet")))) return;
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "activate", id: documentId }));
    if (document.kind === "sheet") void beginSheetLoad(document.sheetName);
  }, [beginSheetLoad, documentTabs.activeId, documentTabs.tabs, requestDiscardConfirmation, t]);

  const handleDocumentClose = useCallback(async (documentId: string) => {
    const document = documentTabs.tabs.find((tab) => tab.id === documentId);
    if (!document) return;
    if (document.kind === "sheet" && document.id === documentTabs.activeId && !(await requestDiscardConfirmation(t("workbench.discard.closeSheet")))) return;
    const next = reduceDocumentTabs(documentTabs, { type: "close", id: documentId });
    setDocumentTabs(next);
    if (next.activeId && next.activeId !== documentTabs.activeId) {
      const nextDocument = next.tabs.find((tab) => tab.id === next.activeId);
      if (nextDocument?.kind === "sheet") void beginSheetLoad(nextDocument.sheetName);
    }
  }, [beginSheetLoad, documentTabs, requestDiscardConfirmation, t]);

  const openCommit = useCallback(async (commit: GitCommitDto) => {
    const active = documentTabs.tabs.find((document) => document.id === documentTabs.activeId);
    if (active?.kind === "sheet" && !(await requestDiscardConfirmation(t("workbench.discard.changeSheet")))) return;
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "openCommit", commitId: commit.id, label: `${commit.id.slice(0, 7)} ${commit.subject}` }));
  }, [documentTabs.activeId, documentTabs.tabs, requestDiscardConfirmation, t]);
  const stableOpenCommit = useStableCallback((commit: GitCommitDto) => void openCommit(commit));

  const handleDocumentPin = useCallback((documentId: string) => {
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "pin", id: documentId }));
  }, []);

  const handleOccurrenceSelect = useCallback(async (occurrence: TranslationOccurrenceView) => {
    const cursor = {
      sheetName: occurrence.binding.sheetName,
      rowId: occurrence.binding.rowId,
      subrowId: occurrence.binding.subrowId,
    };
    const sameRow = selectedRowCursor !== null && rowKey(selectedRowCursor) === occurrence.rowKey;
    if (!sameRow && !(await requestDiscardConfirmation(t("workbench.discard.changeRow")))) {
      focusTargetRequest.current = false;
      return;
    }
    pendingAdvance.current = null;
    pendingReveal.current = null;
    if (loaderRef.current) loaderRef.current.selected = true;

    flushSync(() => {
      setSelectedRowCursor(cursor);
      setSelectedBinding(occurrence.binding);
      if (!sameRow) {
        hasDirtyDraft.current = false;
        setDirty(false);
      }
      setEditorError(null);
    });
  }, [requestDiscardConfirmation, selectedRowCursor, t]);

  const navigateOccurrence = useCallback((direction: 1 | -1): boolean => {
    const next = adjacentOccurrence(visibleOccurrences, selectedBinding, direction);
    if (next) void handleOccurrenceSelect(next);
    return next !== null;
  }, [handleOccurrenceSelect, selectedBinding, visibleOccurrences]);

  // "Save & next" waits until the saved overlay has been applied and the
  // editor reports no remaining dirty draft before moving on.
  useEffect(() => {
    if (pendingAdvance.current === null || dirty || mutations.length > 0) return;
    if (!selectedBinding || bindingKey(selectedBinding) !== pendingAdvance.current) {
      pendingAdvance.current = null;
      return;
    }
    pendingAdvance.current = null;
    if (!navigateOccurrence(1)) focusTargetRequest.current = false;
  }, [dirty, mutations.length, navigateOccurrence, rows, selectedBinding]);

  const navigateFromEditor = useCallback((direction: 1 | -1) => {
    focusTargetRequest.current = preferences.focusTargetOnNext && adjacentOccurrence(visibleOccurrences, selectedBinding, direction) !== null;
    navigateOccurrence(direction);
  }, [navigateOccurrence, preferences.focusTargetOnNext, selectedBinding, visibleOccurrences]);

  const takeFocusRequest = useCallback(() => {
    const requested = focusTargetRequest.current;
    focusTargetRequest.current = false;
    return requested;
  }, []);

  const handleFieldSelect = useCallback((binding: SourceBinding) => {
    setSelectedBinding(binding);
  }, []);

  const confirmMutationDiscard = useCallback((shouldConfirm: boolean, message: string): Promise<boolean> => {
    return shouldConfirm ? requestDiscardConfirmation(message) : Promise.resolve(true);
  }, [requestDiscardConfirmation]);

  const runMutation = useCallback(async (kind: CellMutation["kind"], key: string, title: string, action: () => Promise<void>): Promise<boolean> => {
    if (mutationKeys.current.has(key)) return false;
    flushSync(() => {
      mutationKeys.current.add(key);
      setMutations((current) => [...current, { kind, bindingKey: key }]);
      setEditorError(null);
    });
    try {
      await action();
      return true;
    } catch (error) {
      showError(title, error);
      return false;
    } finally {
      mutationKeys.current.delete(key);
      setMutations((current) => current.filter((mutation) => mutation.bindingKey !== key));
    }
  }, [showError]);

  const handleSaveTarget = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void, advance: boolean) => {
    if (!selectedRow || !(await confirmMutationDiscard(otherDirty, t("workbench.discard.saveTarget")))) return;
    if (otherDirty) discardOtherDrafts();
    const key = bindingKey(cell.sourceBinding);
    const saved = await runMutation("target", key, t("workbench.error.saveTarget"), async () => {
      const overlay = await setTranslationTarget(cell.sourceBinding, draft.target);
      if (advance) {
        pendingAdvance.current = key;
        focusTargetRequest.current = preferences.focusTargetOnNext;
      }
      applyOverlay(cell.sourceBinding, overlay);
    });
    if (!saved) {
      pendingAdvance.current = null;
      focusTargetRequest.current = false;
    }
  }, [applyOverlay, confirmMutationDiscard, preferences.focusTargetOnNext, runMutation, selectedRow, t]);

  const handleApprove = useCallback(async (cell: TranslationCellDto, draft: CellDraft, targetDirty: boolean, otherDirty: boolean, discardOtherDrafts: () => void) => {
    if (!selectedRow || !(await confirmMutationDiscard(otherDirty, t("workbench.discard.review")))) return;
    if (otherDirty) discardOtherDrafts();
    const key = bindingKey(cell.sourceBinding);
    const approved = await runMutation("review", key, t("workbench.error.review"), async () => {
      let unitId = cell.translation?.translationUnitId ?? null;
      if (targetDirty || unitId === null) {
        const saved = await setTranslationTarget(cell.sourceBinding, draft.target);
        applyOverlay(cell.sourceBinding, saved);
        unitId = saved.translationUnitId;
      }
      const overlay = await setTranslationReviewState(unitId, "reviewed");
      pendingAdvance.current = key;
      focusTargetRequest.current = preferences.focusTargetOnNext;
      applyOverlay(cell.sourceBinding, overlay);
    });
    if (!approved) {
      pendingAdvance.current = null;
      focusTargetRequest.current = false;
    }
  }, [applyOverlay, confirmMutationDiscard, preferences.focusTargetOnNext, runMutation, selectedRow, t]);

  const handleSaveNote = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => {
    const translation = cell.translation;
    if (!translation || !selectedRow || !(await confirmMutationDiscard(otherDirty, t("workbench.discard.saveNote")))) return;
    if (otherDirty) discardOtherDrafts();
    await runMutation("note", bindingKey(cell.sourceBinding), t("workbench.error.saveNote"), async () => {
      const overlay = await setTranslationNote(translation.translationUnitId, draft.note.length === 0 ? null : draft.note);
      applyOverlay(cell.sourceBinding, overlay);
    });
  }, [applyOverlay, confirmMutationDiscard, runMutation, selectedRow, t]);

  const handleReviewChange = useCallback(async (cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => {
    const translation = cell.translation;
    const shouldDiscardDrafts = hasDirtyDraft.current;
    if (!translation || translation.reviewState === reviewState || !(await confirmMutationDiscard(shouldDiscardDrafts, t("workbench.discard.review")))) return;
    if (shouldDiscardDrafts) discardDrafts();
    await runMutation("review", bindingKey(cell.sourceBinding), t("workbench.error.review"), async () => {
      const overlay = await setTranslationReviewState(translation.translationUnitId, reviewState);
      applyOverlay(cell.sourceBinding, overlay);
    });
  }, [applyOverlay, confirmMutationDiscard, runMutation, t]);

  const handleClose = useCallback(async () => {
    if (!(await requestDiscardConfirmation(t("workbench.discard.closeProject")))) return;
    flushSync(() => {
      setClosing(true);
      setEditorError(null);
    });
    try {
      await closeProject();
      setClosing(false);
      onClosed();
    } catch (error) {
      setClosing(false);
      showError(t("workbench.error.closeProject"), error);
    }
  }, [onClosed, requestDiscardConfirmation, showError, t]);

  const handleWindowClose = useCallback(async () => {
    if (!(await requestDiscardConfirmation(t("workbench.discard.closeApp")))) return;
    allowWindowClose.current = true;
    try {
      await getCurrentWindow().close();
    } catch {
      allowWindowClose.current = false;
    }
  }, [requestDiscardConfirmation, t]);

  const selectedUnitId = useMemo(() => {
    if (!selectedRow || !selectedBinding) return null;
    const key = bindingKey(selectedBinding);
    return selectedRow.cells.find((cell) => bindingKey(cell.sourceBinding) === key)?.translation?.translationUnitId ?? null;
  }, [selectedRow, selectedBinding]);

  const handleWorkspaceChanged = useCallback(() => {
    setWorkspaceRevision((current) => current + 1);
    if (selectedSheetName) void beginSheetLoad(selectedSheetName, { immediate: false, refresh: true });
  }, [beginSheetLoad, selectedSheetName]);

  // Uncommitted translation-unit changes drive list markers and the editor
  // diff. Projects without a repository simply have none.
  const refreshPendingChanges = useCallback(async () => {
    try {
      setPendingChanges(await gitPendingChanges());
    } catch {
      setPendingChanges(null);
    }
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => void refreshPendingChanges(), 250);
    return () => window.clearTimeout(timer);
  }, [refreshPendingChanges, workspaceRevision]);

  const pendingByBinding = useMemo(() => {
    const byBinding = new Map<string, UnitChangeDto>();
    for (const change of pendingChanges ?? []) {
      const unit = change.after ?? change.before;
      if (unit) byBinding.set(bindingKey(unit.sourceBinding), change);
    }
    return byBinding;
  }, [pendingChanges]);

  const changedKinds = useMemo(() => new Map([...pendingByBinding].map(([key, change]) => [key, change.kind])), [pendingByBinding]);

  const selectedCheckpoint = useMemo<CheckpointBaseline | null>(() => {
    const change = selectedBinding ? pendingByBinding.get(bindingKey(selectedBinding)) : undefined;
    if (!change) return null;
    return {
      kind: change.kind,
      target: change.before?.targetMacro ?? null,
      reviewChanged: change.reviewChanged,
      noteChanged: change.noteChanged,
    };
  }, [pendingByBinding, selectedBinding]);

  /** Opens a string by coordinate, waiting for its sheet to load far enough to contain it. */
  const revealString = useCallback(async (sheetName: string, target: RowTarget) => {
    if (!sheetsByName.has(sheetName)) {
      setEditorError({ title: t("workbench.error.sheetNotFound"), tone: "warning", error: { code: "sheetNotFound", message: t("workbench.error.sheetMissing", { sheet: sheetName }) } });
      return;
    }
    const loader = loaderRef.current;
    if (sheetName === selectedSheetName && loader?.sheetName === sheetName && !loader.failed) {
      const sameRow = selectedRowKeyRef.current === rowKey({ sheetName, rowId: target.rowId, subrowId: target.subrowId });
      if (!sameRow && !(await requestDiscardConfirmation(t("workbench.discard.openString")))) return;
      if (loaderRef.current !== loader) return;
      pendingReveal.current = { sheetName, target };
      settleReveal(loader);
      return;
    }
    if (!(await requestDiscardConfirmation(t("workbench.discard.openString")))) return;
    if (sheetName !== selectedSheetName) setDocumentTabs((current) => reduceDocumentTabs(current, { type: "openSheet", sheetName, pin: true }));
    setLensFilter(emptyOccurrenceFilter);
    void beginSheetLoad(sheetName, { reveal: target });
  }, [beginSheetLoad, requestDiscardConfirmation, selectedSheetName, settleReveal, sheetsByName, t]);

  const revealBinding = useCallback((binding: SourceBinding) => {
    void revealString(binding.sheetName, { rowId: binding.rowId, subrowId: binding.subrowId, columnIndex: binding.columnIndex });
  }, [revealString]);

  // Angelica's navigate_to tool asks the editor to show one occurrence.
  const stableRevealBinding = useStableCallback(revealBinding);
  const stableWorkspaceChanged = useStableCallback(handleWorkspaceChanged);
  const revealBindingRef = useRef(revealBinding);
  revealBindingRef.current = revealBinding;
  useEffect(() => {
    const subscription = listen<SourceBinding>("angelica://navigate", ({ payload }) => revealBindingRef.current(payload));
    return () => { void subscription.then((unlisten) => unlisten()); };
  }, []);

  // Translations Angelica writes patch their cell like an ordinary save.
  const applyOverlayRef = useRef(applyOverlay);
  applyOverlayRef.current = applyOverlay;
  useEffect(() => {
    const subscription = listen<TranslationAppliedDto>("angelica://translation-applied", ({ payload }) => applyOverlayRef.current(payload.sourceBinding, payload.overlay));
    return () => { void subscription.then((unlisten) => unlisten()); };
  }, []);

  const angelicaContext = useMemo<EditorContextDto>(() => ({
    sheet: selectedSheetName,
    selection: selectedBinding ? { sheet: selectedBinding.sheetName, row: selectedBinding.rowId, subrow: selectedBinding.subrowId, column: selectedBinding.columnIndex } : null,
    unsavedDraft: dirty,
  }), [dirty, selectedBinding, selectedSheetName]);

  const openPalette = useCallback((input: string) => {
    setPalette((current) => ({ open: true, input, key: current.key + 1 }));
  }, []);

  const openSettings = useCallback((section: SettingsSection = "appearance") => {
    setSettingsSection(section);
    setSettingsOpen(true);
  }, []);

  const handleRestoreTarget = useCallback(async (targetMacro: string) => {
    if (!selectedBinding) return;
    if (!(await requestDiscardConfirmation(t("workbench.discard.restore")))) return;
    const binding = selectedBinding;
    await runMutation("target", bindingKey(binding), t("workbench.error.restore"), async () => {
      const overlay = await setTranslationTarget(binding, targetMacro);
      applyOverlay(binding, overlay);
    });
  }, [applyOverlay, requestDiscardConfirmation, runMutation, selectedBinding, t]);

  // Stable handlers keep the memoized list and editor from re-rendering on
  // unrelated workbench updates.
  const selectOccurrence = useStableCallback((occurrence: TranslationOccurrenceView) => void handleOccurrenceSelect(occurrence));
  const stableNavigateOccurrence = useStableCallback(navigateOccurrence);
  const stableNavigateFromEditor = useStableCallback(navigateFromEditor);
  const stableDirtyChange = useStableCallback(handleDirtyChange);
  const saveTarget = useStableCallback((...args: Parameters<typeof handleSaveTarget>) => void handleSaveTarget(...args));
  const approve = useStableCallback((...args: Parameters<typeof handleApprove>) => void handleApprove(...args));
  const saveNote = useStableCallback((...args: Parameters<typeof handleSaveNote>) => void handleSaveNote(...args));
  const changeReview = useStableCallback((...args: Parameters<typeof handleReviewChange>) => void handleReviewChange(...args));
  const draftWithAngelica = useStableCallback(async (cell: TranslationCellDto) => {
    try {
      return (await angelicaDraft(cell.sourceBinding)).target;
    } catch (reason) {
      showError(t("editor.draftFailed"), reason);
      return null;
    }
  });
  const restoreTarget = useStableCallback((target: string) => void handleRestoreTarget(target));
  const openAiSettings = useCallback(() => openSettings("ai"), [openSettings]);
  const openRepositorySettings = useCallback(() => openSettings("repository"), [openSettings]);
  const pendingState = useMemo(() => ({ changes: pendingChanges, refresh: refreshPendingChanges }), [pendingChanges, refreshPendingChanges]);

  // Layout ---------------------------------------------------------------

  const workbenchBodyRef = useRef<HTMLDivElement>(null);
  /** Props for a handle that resizes `regionId` through the CSS `variable` on the workbench body. */
  const resizeProps = (regionId: string, variable: string, direction: 1 | -1) => {
    const region = layout.regions[regionId]!;
    return {
      size: region.size,
      min: region.minSize,
      max: region.maxSize,
      direction,
      target: workbenchBodyRef,
      variable,
      onResize: (delta: number) => dispatchLayout({ type: "resizeRegion", regionId, delta }),
    };
  };

  const setRegionVisible = useCallback((regionId: string, visible: boolean) => {
    dispatchLayout({ type: "setRegionVisibility", regionId, visible });
  }, []);

  const regionPanelId = useCallback((region: DockRegion): string | null => dockLayoutState.groups.find((group) => group.region === region)?.activePanelId ?? null, [dockLayoutState.groups]);
  const leftPanelId = regionPanelId("left");
  const rightPanelId = regionPanelId("right");
  const bottomPanelId = regionPanelId("bottom");

  const handlePanelMove = useCallback((panelId: string, region: string) => {
    if (region !== "left" && region !== "right" && region !== "bottom") return;
    setDockLayoutState((current) => reduceDockLayout(current, { type: "move", panelId, region }));
    if (panelId === "ai" || panelId === "git") setActiveTool(panelId);
    setRegionVisible(region === "left" ? "leftDock" : region === "right" ? "rightDock" : "bottomPanel", true);
  }, [setRegionVisible]);

  const showPanel = useCallback((panelId: string, fallbackRegion: DockRegion, toggle = true) => {
    const placement = dockLayoutState.placements.find((candidate) => candidate.panelId === panelId);
    const region = placement && placement.region !== "floating" ? placement.region : fallbackRegion;
    const regionId = region === "left" ? "leftDock" : region === "right" ? "rightDock" : "bottomPanel";
    const visible = layout.regions[regionId]?.visible ?? false;
    if (toggle && visible && regionPanelId(region) === panelId) {
      setRegionVisible(regionId, false);
      return;
    }
    if (placement?.region === "floating") {
      setDockLayoutState((current) => reduceDockLayout(current, { type: "move", panelId, region }));
    } else {
      setDockLayoutState((current) => reduceDockLayout(current, { type: "activate", panelId }));
    }
    if (panelId === "ai" || panelId === "git") setActiveTool(panelId);
    setRegionVisible(regionId, true);
  }, [dockLayoutState.placements, layout.regions, regionPanelId, setRegionVisible]);

  const panelMoveTargets = useCallback((panelId: string, currentRegion: DockRegion): Array<{ id: string; label: string }> => {
    const restricted = bottomPanelIds.has(panelId) || panelId === "ai" || panelId === "git";
    return (["left", "right", "bottom"] as const)
      .filter((region) => region !== currentRegion && (!restricted || region !== "left"))
      .map((region) => ({ id: region, label: t(moveTargetLabels[region]) }));
  }, [t]);

  const focusSheetFilter = useCallback(() => {
    setSheetFilterOpen(true);
    setQuickFindSignal((current) => current + 1);
  }, []);

  const handleQuickFind = useCallback(() => {
    showPanel("sheets", "left", false);
    focusSheetFilter();
  }, [focusSheetFilter, showPanel]);

  const detachPanel = useCallback(async (panel: DetachedPanel) => {
    if (detachedPanel === panel) return;
    try {
      const placement = dockLayoutState.placements.find((candidate) => candidate.panelId === panel);
      const originRegion = placement?.region === "left" || placement?.region === "right" || placement?.region === "bottom" ? placement.region : null;
      const label = `aeria-tool-${panel}`;
      const wide = bottomPanelIds.has(panel);
      const existing = await WebviewWindow.getByLabel(label);
      const detached = existing ?? new WebviewWindow(label, {
        title: `${t(detachedPanelTitle(panel))} · Aeria`,
        url: `/?detached=${panel}`,
        width: wide ? 720 : 400,
        height: wide ? 320 : 640,
        minWidth: 320,
        minHeight: 220,
        decorations: false,
        resizable: true,
        ...(hasWindowsBackdrop() ? { transparent: true, windowEffects: { effects: [Effect.Acrylic] } } : {}),
      });
      if (existing) {
        await detached.show();
        await detached.setFocus();
      }
      void detached.onCloseRequested(() => {
        setDetachedPanel((current) => current === panel ? null : current);
        if (originRegion) setDockLayoutState((current) => reduceDockLayout(current, { type: "restore", panelId: panel, region: originRegion }));
      });
      if (originRegion) setDockLayoutState((current) => reduceDockLayout(current, { type: "float", panelId: panel }));
      setDetachedPanel(panel);
    } catch (error) {
      showError(t("workbench.error.toolWindow"), error);
    }
  }, [detachedPanel, dockLayoutState.placements, showError, t]);

  // Keyboard -------------------------------------------------------------

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent) {
      if (event.defaultPrevented || discardRequestRef.current) return;
      const key = event.key.toLocaleLowerCase();
      const editable = isEditableTarget(event.target);
      if (event.ctrlKey && event.shiftKey && !event.altKey && key === "p") {
        event.preventDefault();
        openPalette(">");
        return;
      }
      if (event.ctrlKey && event.shiftKey && !event.altKey && key === "enter" && !editable) {
        event.preventDefault();
        editorRef.current?.approve();
        return;
      }
      if (event.ctrlKey && !event.altKey && !event.shiftKey) {
        if (key === "p") {
          event.preventDefault();
          openPalette("");
        } else if (key === "g") {
          event.preventDefault();
          openPalette(":");
        } else if (key === "w" && documentTabs.activeId) {
          event.preventDefault();
          void handleDocumentClose(documentTabs.activeId);
        } else if (key === "b") {
          event.preventDefault();
          dispatchLayout({ type: "toggleRegion", regionId: "leftDock" });
        } else if (key === "j") {
          event.preventDefault();
          dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" });
        } else if (key === ",") {
          event.preventDefault();
          openSettings();
        } else if (key === "s" && !editable) {
          event.preventDefault();
          editorRef.current?.saveTarget(false);
        } else if (key === "enter" && !editable) {
          event.preventDefault();
          editorRef.current?.saveTarget(true);
        }
      } else if (event.altKey && !event.ctrlKey && (event.key === "ArrowDown" || event.key === "ArrowUp") && !editable) {
        event.preventDefault();
        navigateOccurrence(event.key === "ArrowDown" ? 1 : -1);
      }
    }
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [documentTabs.activeId, handleDocumentClose, navigateOccurrence, openPalette, openSettings]);

  // Presentation ---------------------------------------------------------

  const documents: DocumentTab[] = documentTabs.tabs.map((document) => ({
    id: document.id,
    label: document.label,
    closable: true,
    pinned: document.pinned,
    preview: document.preview,
    dirty: document.dirty,
    icon: document.kind === "commit" ? "gitCommit" : "table2",
  }));
  const activeCommitId = documentTabs.tabs.find((document) => document.id === documentTabs.activeId)?.commitId ?? null;

  const sheetHeaderActions = <>
    <IconButton icon={hideEmptySheets ? "eyeOff" : "eye"} label={t(hideEmptySheets ? "workbench.showEmptySheets" : "workbench.hideEmptySheets")} pressed={hideEmptySheets} disabled={closing} onClick={() => setHideEmptySheets((current) => !current)} />
    <IconButton icon="search" label={t("workbench.filterSheets")} shortcut="Ctrl+F" disabled={closing} onClick={focusSheetFilter} />
    <IconButton icon="locateFixed" label={t("workbench.revealSheet")} disabled={closing || !selectedSheetName} onClick={() => setRevealSheetSignal((current) => current + 1)} />
    <IconButton icon="chevronsUp" label={t("workbench.collapseFolders")} disabled={closing} onClick={() => setCollapseSheetsSignal((current) => current + 1)} />
  </>;

  const renderPanelContent = (panelId: string, active: boolean): ReactNode => {
    if (panelId === "sheets") {
      return <SheetSidebar sheets={project.sheets} selectedSheetName={selectedSheetName} disabled={closing} active={active} hideEmpty={hideEmptySheets} onHideEmptyChange={setHideEmptySheets} filterOpen={sheetFilterOpen} onFilterOpenChange={setSheetFilterOpen} onOpenFilter={focusSheetFilter} quickFindSignal={quickFindSignal} revealSignal={revealSheetSignal} collapseSignal={collapseSheetsSignal} onSelect={handleSheetSelect} progress={progressBySheet} />;
    }
    const tool: WorkbenchTool = panelId === "git" ? "git" : panelId === "search" ? "search" : "ai";
    return <WorkbenchToolDock activeTool={tool} selectedBinding={selectedBinding} onOpenCommit={stableOpenCommit} selectedCommitId={activeCommitId} onOpenRepositorySettings={openRepositorySettings} projectRevision={projectRevision} selectedUnitId={selectedUnitId} workspaceRevision={workspaceRevision} onWorkspaceChanged={stableWorkspaceChanged} pending={pendingState} onRevealBinding={stableRevealBinding} editorContext={angelicaContext} onOpenSettings={openAiSettings} onOpenGuide={openGuide} />;
  };

  const renderDock = (region: "left" | "right", panelId: string | null, open: boolean) => {
    const id = panelId ?? (region === "left" ? "sheets" : activeTool);
    const floatable = id === "ai" || id === "git" || id === "search";
    return (
      <DockPanel
        panelId={id}
        title={t(panelTitle(id))}
        meta={id === "sheets" ? formatSheetCount(project.sheets.length) : undefined}
        headerActions={id === "sheets" ? sheetHeaderActions : undefined}
        moveTargets={panelMoveTargets(id, region)}
        onMove={(target) => handlePanelMove(id, target)}
        canFloat={floatable}
        onFloat={() => { if (floatable) void detachPanel(id); }}
        onHide={() => setRegionVisible(region === "left" ? "leftDock" : "rightDock", false)}
        onDropPanel={(dropped) => handlePanelMove(dropped, region)}
        className={`dock-${region}`}
        hidden={!open || detachedPanel === id || panelId === null}
      >
        {renderPanelContent(id, open)}
      </DockPanel>
    );
  };

  const panelActive = (panelId: string, region: DockRegion, open: boolean) => open && regionPanelId(region) === panelId;
  const placementRegion = (panelId: string): DockRegion | null => {
    const placement = dockLayoutState.placements.find((candidate) => candidate.panelId === panelId);
    return placement && placement.region !== "floating" ? placement.region : null;
  };
  const isPanelShown = (panelId: string) => {
    const region = placementRegion(panelId);
    if (!region) return detachedPanel === panelId;
    const open = region === "left" ? leftDockOpen : region === "right" ? rightDockOpen : bottomOpen;
    return panelActive(panelId, region, open);
  };

  const themeLabel = (entry: (typeof themeRegistry)[number]) => t("common.themeWithFamily", { name: entry.displayName, family: entry.family ?? t("common.themesFamily") });

  const menus: readonly ApplicationMenuDefinition[] = [
    {
      id: "file",
      label: t("menu.file"),
      items: [
        { kind: "command", id: "settings", label: t("menu.settings"), shortcut: "Ctrl+,", onSelect: () => openSettings() },
        { kind: "command", id: "export-pack", label: t("menu.exportPack"), onSelect: () => setExportOpen(true) },
        { kind: "separator", id: "file-sep-1" },
        { kind: "command", id: "close-tab", label: t("menu.closeSheet"), shortcut: "Ctrl+W", ...(documentTabs.activeId ? { onSelect: () => void handleDocumentClose(documentTabs.activeId!) } : {}) },
        { kind: "command", id: "close-project", label: t("menu.closeProject"), onSelect: () => void handleClose() },
        { kind: "separator", id: "file-sep-2" },
        { kind: "command", id: "exit", label: t("menu.exit"), onSelect: () => void handleWindowClose() },
      ],
    },
    {
      id: "translation",
      label: t("menu.translation"),
      items: [
        { kind: "command", id: "save", label: t("menu.saveTarget"), shortcut: "Ctrl+S", ...(selectedRow ? { onSelect: () => editorRef.current?.saveTarget(false) } : {}) },
        { kind: "command", id: "save-next", label: t("menu.saveAndNext"), shortcut: "Ctrl+Enter", ...(selectedRow ? { onSelect: () => editorRef.current?.saveTarget(true) } : {}) },
        { kind: "command", id: "approve-next", label: t("menu.approveAndNext"), shortcut: "Ctrl+Shift+Enter", ...(selectedRow ? { onSelect: () => editorRef.current?.approve() } : {}) },
        { kind: "command", id: "copy-source", label: t("menu.copySource"), ...(selectedRow ? { onSelect: () => editorRef.current?.copySource() } : {}) },
        { kind: "command", id: "revert", label: t("menu.revert"), ...(dirty ? { onSelect: () => editorRef.current?.revert() } : {}) },
        { kind: "separator", id: "translation-sep-1" },
        { kind: "command", id: "glossary", label: t("menu.glossary"), onSelect: () => openGuide("glossary") },
        { kind: "command", id: "guidance", label: t("menu.guidance"), onSelect: () => openGuide("guidance") },
      ],
    },
    {
      id: "go",
      label: t("menu.go"),
      items: [
        { kind: "command", id: "go-sheet", label: t("menu.goToSheet"), shortcut: "Ctrl+P", onSelect: () => openPalette("") },
        { kind: "command", id: "go-row", label: t("menu.goToRow"), shortcut: "Ctrl+G", ...(selectedSheetName ? { onSelect: () => openPalette(":") } : {}) },
        { kind: "command", id: "command-palette", label: t("menu.commandPalette"), shortcut: "Ctrl+Shift+P", onSelect: () => openPalette(">") },
        { kind: "separator", id: "go-sep" },
        { kind: "command", id: "go-next", label: t("menu.nextString"), shortcut: "Alt+Down", onSelect: () => navigateOccurrence(1) },
        { kind: "command", id: "go-previous", label: t("menu.previousString"), shortcut: "Alt+Up", onSelect: () => navigateOccurrence(-1) },
      ],
    },
    {
      id: "view",
      label: t("menu.view"),
      items: [
        { kind: "command", id: "sheets", label: t("workbench.panel.sheets"), shortcut: "Ctrl+B", checked: isPanelShown("sheets"), onSelect: () => showPanel("sheets", "left") },
        { kind: "command", id: "search", label: t("workbench.tool.search"), checked: isPanelShown("search"), onSelect: () => showPanel("search", "left") },
        { kind: "command", id: "git", label: t("workbench.tool.git"), checked: isPanelShown("git"), onSelect: () => showPanel("git", "right") },
        { kind: "command", id: "ai", label: t("workbench.tool.ai"), checked: isPanelShown("ai"), onSelect: () => showPanel("ai", "right") },
        { kind: "command", id: "bottom", label: t("menu.bottomPanel"), shortcut: "Ctrl+J", checked: bottomOpen, onSelect: () => dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" }) },
        { kind: "separator", id: "view-sep-1" },
        { kind: "command", id: "filter-sheets", label: t("workbench.filterSheets"), shortcut: "Ctrl+F", onSelect: handleQuickFind },
        { kind: "command", id: "reveal-sheet", label: t("workbench.revealSheet"), ...(selectedSheetName ? { onSelect: () => { showPanel("sheets", "left", false); setRevealSheetSignal((current) => current + 1); } } : {}) },
        { kind: "separator", id: "view-sep-2" },
        {
          kind: "submenu",
          id: "theme",
          label: t("menu.theme"),
          items: [{ kind: "radio", id: "theme-radio", label: t("menu.theme"), value: theme.id, items: themeRegistry.map((entry) => ({ value: entry.id, label: themeLabel(entry) })), onSelect: setThemeId }],
        },
      ],
    },
  ];

  const category = {
    go: t("command.category.go"),
    translation: t("command.category.translation"),
    strings: t("command.category.strings"),
    view: t("command.category.view"),
    git: t("command.category.git"),
    preferences: t("command.category.preferences"),
    theme: t("command.category.theme"),
    file: t("command.category.file"),
  };
  const commands: PaletteCommand[] = [
    { id: "go-sheet", category: category.go, title: t("command.goToSheet"), shortcut: "Ctrl+P", icon: "table2", run: () => openPalette("") },
    { id: "go-row", category: category.go, title: t("command.goToRow"), shortcut: "Ctrl+G", icon: "arrowRight", enabled: selectedSheetName !== null, run: () => openPalette(":") },
    { id: "go-next", category: category.go, title: t("menu.nextString"), shortcut: "Alt+Down", icon: "arrowDown", run: () => navigateOccurrence(1) },
    { id: "go-previous", category: category.go, title: t("menu.previousString"), shortcut: "Alt+Up", icon: "arrowUp", run: () => navigateOccurrence(-1) },
    { id: "save", category: category.translation, title: t("menu.saveTarget"), shortcut: "Ctrl+S", icon: "save", enabled: selectedRow !== null, run: () => editorRef.current?.saveTarget(false) },
    { id: "approve-next", category: category.translation, title: t("menu.approveAndNext"), shortcut: "Ctrl+Shift+Enter", icon: "check", enabled: selectedRow !== null, run: () => editorRef.current?.approve() },
    { id: "save-next", category: category.translation, title: t("command.saveAndNext"), shortcut: "Ctrl+Enter", icon: "save", enabled: selectedRow !== null, run: () => editorRef.current?.saveTarget(true) },
    { id: "copy-source", category: category.translation, title: t("menu.copySource"), icon: "copyPlus", enabled: selectedRow !== null, run: () => editorRef.current?.copySource() },
    { id: "revert", category: category.translation, title: t("menu.revert"), icon: "undo", enabled: dirty, run: () => editorRef.current?.revert() },
    { id: "filter-untranslated", category: category.strings, title: t("command.showUntranslated"), icon: "listFilter", run: () => setLensFilter({ ...emptyOccurrenceFilter, status: "untranslated" }) },
    { id: "filter-review", category: category.strings, title: t("command.showNeedsReview"), icon: "listFilter", run: () => setLensFilter({ ...emptyOccurrenceFilter, status: "needsReview" }) },
    { id: "filter-clear", category: category.strings, title: t("command.clearFilters"), icon: "x", run: () => setLensFilter(emptyOccurrenceFilter) },
    { id: "view-sheets", category: category.view, title: t("command.toggleSheets"), shortcut: "Ctrl+B", icon: "table2", run: () => showPanel("sheets", "left") },
    { id: "view-search", category: category.view, title: t("command.toggleSearch"), icon: "search", run: () => showPanel("search", "left") },
    { id: "view-git", category: category.view, title: t("command.toggleGit"), icon: "gitBranch", run: () => showPanel("git", "right") },
    { id: "view-ai", category: category.view, title: t("command.toggleAi"), icon: "sparkles", run: () => showPanel("ai", "right") },
    { id: "view-bottom", category: category.view, title: t("command.toggleBottom"), shortcut: "Ctrl+J", icon: "panelBottom", run: () => dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" }) },
    { id: "view-filter-sheets", category: category.view, title: t("workbench.filterSheets"), shortcut: "Ctrl+F", icon: "search", run: handleQuickFind },
    { id: "view-reveal-sheet", category: category.view, title: t("workbench.revealSheet"), icon: "locateFixed", enabled: selectedSheetName !== null, run: () => { showPanel("sheets", "left", false); setRevealSheetSignal((current) => current + 1); } },
    { id: "project-glossary", category: category.translation, title: t("menu.glossary"), icon: "languages", run: () => openGuide("glossary") },
    { id: "project-guidance", category: category.translation, title: t("menu.guidance"), icon: "messageSquare", run: () => openGuide("guidance") },
    { id: "git-open", category: category.git, title: t("command.showChanges"), icon: "gitBranch", run: () => showPanel("git", "right", false) },
    { id: "prefs-settings", category: category.preferences, title: t("command.openSettings"), shortcut: "Ctrl+,", icon: "settings", run: () => openSettings() },
    { id: "prefs-theme", category: category.preferences, title: t("settings.theme.title"), icon: "palette", run: () => openSettings("appearance") },
    { id: "prefs-shortcuts", category: category.preferences, title: t("settings.section.keyboard"), icon: "listFilter", run: () => openSettings("keyboard") },
    ...themeRegistry.map((entry): PaletteCommand => ({ id: `theme-${entry.id}`, category: category.theme, title: themeLabel(entry), icon: "palette", run: () => setThemeId(entry.id) })),
    { id: "file-close-sheet", category: category.file, title: t("menu.closeSheet"), shortcut: "Ctrl+W", icon: "x", enabled: documentTabs.activeId !== null, run: () => { if (documentTabs.activeId) void handleDocumentClose(documentTabs.activeId); } },
    { id: "file-export-pack", category: category.file, title: t("menu.exportPack"), icon: "arrowUpRight", run: () => setExportOpen(true) },
    { id: "file-close-project", category: category.file, title: t("menu.closeProject"), icon: "folder", run: () => void handleClose() },
  ];

  const bottomIsTool = bottomPanelId !== null && !bottomPanelIds.has(bottomPanelId);

  return (
    <main className="workbench">
      <WindowChrome
        mode="workbench"
        menus={menus}
        center={(
          <button className="command-center" type="button" onClick={() => openPalette("")} title={t("workbench.commandCenter")}>
            <UiIcon icon="search" size="sm" />
            <span className="command-center-label"><strong>{projectName}</strong>{selectedSheetName ? <span> / {selectedSheetName}</span> : null}</span>
            <kbd>Ctrl P</kbd>
          </button>
        )}
        onClose={() => void handleWindowClose()}
        actions={<>
          <IconButton icon="panelLeft" label={t("workbench.toggleLeftPanel")} shortcut="Ctrl+B" pressed={leftDockOpen} onClick={() => dispatchLayout({ type: "toggleRegion", regionId: "leftDock" })} />
          <IconButton icon="panelBottom" label={t("workbench.toggleBottomPanel")} shortcut="Ctrl+J" pressed={bottomOpen} onClick={() => dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" })} />
          <IconButton icon="panelRight" label={t("workbench.toggleRightPanel")} pressed={rightDockOpen} onClick={() => dispatchLayout({ type: "toggleRegion", regionId: "rightDock" })} />
        </>}
      />
      {applicationWarning || editorError ? (
        <div className="notices" aria-live="polite">
          {applicationWarning ? <ErrorBanner tone="warning" title={t("workbench.recentWarning")} error={applicationWarning} onDismiss={onDismissApplicationWarning} /> : null}
          {editorError ? <ErrorBanner title={editorError.title} error={editorError.error} onDismiss={() => setEditorError(null)} /> : null}
        </div>
      ) : null}
      <div
        ref={workbenchBodyRef}
        className="workbench-body"
        style={{
          "--left-dock-width": `${layout.regions.leftDock.size}px`,
          "--right-dock-width": `${layout.regions.rightDock.size}px`,
          "--bottom-panel-height": `${layout.regions.bottomPanel.size}px`,
          "--editor-height": `${layout.regions.editor.size}px`,
        } as CSSProperties}
      >
        <ActivityRail
          side="left"
          items={[
            { id: "sheets", label: t("workbench.panel.sheets"), icon: "table2", shortcut: "Ctrl+B", active: isPanelShown("sheets"), onSelect: () => showPanel("sheets", "left") },
            { id: "search", label: t("workbench.tool.search"), icon: "search", active: isPanelShown("search"), onSelect: () => showPanel("search", "left") },
          ]}
          footer={[{ id: "settings", label: t("common.settings"), icon: "settings", shortcut: "Ctrl+,", onSelect: () => openSettings() }]}
        />
        {renderDock("left", leftPanelId, leftDockOpen)}
        {leftDockOpen && leftPanelId && detachedPanel !== leftPanelId ? <ResizeHandle axis="x" label={t("workbench.resizeLeft")} {...resizeProps("leftDock", "--left-dock-width", 1)} /> : null}

        <div className="workbench-center">
          <section className="panel document">
            <DocumentTabs
              documents={documents}
              activeDocumentId={documentTabs.activeId}
              onSelect={(documentId) => void handleDocumentSelect(documentId)}
              onClose={(documentId) => void handleDocumentClose(documentId)}
              onPin={handleDocumentPin}
              onReorder={(documentId, beforeDocumentId) => setDocumentTabs((current) => reduceDocumentTabs(current, { type: "reorder", id: documentId, beforeId: beforeDocumentId }))}
            />
            {activeCommitId ? (
              <CommitView commitId={activeCommitId} onRevealBinding={stableRevealBinding} />
            ) : !selectedSheetName ? (
              <div className="document-empty empty-state">
                <strong>{t("workbench.noSheetOpen")}</strong>
                <p>{t("workbench.noSheetOpenHint")}</p>
              </div>
            ) : sheetHasNoRows ? (
              <div className="document-empty empty-state">
                <strong>{t("workbench.noRows")}</strong>
                <p>{t("workbench.noRowsHint")}</p>
              </div>
            ) : (
              <div className="document-split">
                <TranslationList
                  occurrences={visibleOccurrences}
                  loadedOccurrenceCount={allOccurrences.length}
                  sheetProgress={selectedSheetProgress}
                  filter={lensFilter}
                  onFilterChange={setLensFilter}
                  selectedBinding={selectedBinding}
                  selectedSheetName={selectedSheetName}
                  loadedSheetName={loadedSheetName}
                  disabled={closing}
                  loading={sheetLoading}
                  streaming={sheetStreaming}
                  loadProgress={loadProgress}
                  sheetStringCount={selectedSheet?.translatableCellCount ?? null}
                  onSelect={selectOccurrence}
                  onNavigate={stableNavigateOccurrence}
                  changedKinds={changedKinds}
                />
                <ResizeHandle axis="y" label={t("workbench.resizeEditor")} {...resizeProps("editor", "--editor-height", -1)} />
                <TranslationEditor
                  ref={editorRef}
                  onDraftWithAngelica={draftWithAngelica}
                  key={selectedRow ? rowKey(selectedRow) : "empty-editor"}
                  row={selectedRow}
                  selectedBinding={selectedBinding}
                  sourceLanguage={project.sourceLanguage}
                  mutations={mutations}
                  onDirtyChange={stableDirtyChange}
                  onSelectCell={handleFieldSelect}
                  onSaveTarget={saveTarget}
                  onApprove={approve}
                  onSaveNote={saveNote}
                  onReviewChange={changeReview}
                  onNavigate={stableNavigateFromEditor}
                  takeFocusRequest={takeFocusRequest}
                  checkpoint={selectedCheckpoint}
                  historyRevision={workspaceRevision + projectRevision}
                />
              </div>
            )}
          </section>
          {bottomOpen && bottomPanelId && detachedPanel !== bottomPanelId ? <>
            <ResizeHandle axis="y" label={t("workbench.resizeBottom")} {...resizeProps("bottomPanel", "--bottom-panel-height", -1)} />
            {bottomIsTool ? (
              <DockPanel
                panelId={bottomPanelId}
                title={t(panelTitle(bottomPanelId))}
                headerActions={bottomPanelId === "sheets" ? sheetHeaderActions : undefined}
                moveTargets={panelMoveTargets(bottomPanelId, "bottom")}
                onMove={(target) => handlePanelMove(bottomPanelId, target)}
                onHide={() => setRegionVisible("bottomPanel", false)}
                onDropPanel={(dropped) => handlePanelMove(dropped, "bottom")}
                className="dock-bottom"
              >
                {renderPanelContent(bottomPanelId, true)}
              </DockPanel>
            ) : (
              <BottomPanel
                activeTab={bottomPanelId === "gitChanges" ? "gitChanges" : bottomPanelId === "diagnostics" ? "diagnostics" : "tasks"}
                onTabChange={(tab) => setDockLayoutState((current) => reduceDockLayout(current, { type: "activate", panelId: tab }))}
                onCollapse={() => setRegionVisible("bottomPanel", false)}
                onDetach={() => void detachPanel(bottomPanelId === "gitChanges" ? "gitChanges" : bottomPanelId === "diagnostics" ? "diagnostics" : "tasks")}
                onDropPanel={(dropped) => handlePanelMove(dropped, "bottom")}
              />
            )}
          </> : null}
        </div>

        {rightDockOpen && rightPanelId && detachedPanel !== rightPanelId ? <ResizeHandle axis="x" label={t("workbench.resizeRight")} {...resizeProps("rightDock", "--right-dock-width", -1)} /> : null}
        {renderDock("right", rightPanelId, rightDockOpen)}
        <ActivityRail
          side="right"
          items={[
            { id: "git", label: t("workbench.tool.git"), icon: "gitBranch", active: isPanelShown("git"), onSelect: () => showPanel("git", "right") },
            { id: "ai", label: t("workbench.tool.ai"), icon: "sparkles", active: isPanelShown("ai"), onSelect: () => showPanel("ai", "right") },
          ]}
        />
      </div>
      <StatusBar
        sheetName={selectedSheetName}
        rowCount={rows.length}
        loading={sheetLoading}
        repositoryRoot={project.repositoryRoot}
        sourceLanguage={project.sourceLanguage}
        sourceSnapshotId={project.sourceSnapshotId}
        selectedBinding={selectedBinding}
        dirty={dirty}
        projectProgress={projectProgress}
        detachedCount={project.detachedUnitCount}
        onShowDetached={onShowDetachedUnits}
      />
      <ConfirmDialog
        open={discardRequest !== null}
        message={discardRequest?.message ?? ""}
        onKeepEditing={() => resolveDiscardConfirmation(false)}
        onDiscard={() => resolveDiscardConfirmation(true)}
      />
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} initialSection={settingsSection} projectOpen />
      <ProjectGuideDialog open={guide.open} initialTab={guide.tab} onOpenChange={setGuideOpen} />
      <ExportDialog open={exportOpen} onOpenChange={setExportOpen} onOpenChanges={() => { setExportOpen(false); showPanel("git", "right", false); }} />
      {palette.open ? (
        <CommandPalette
          key={palette.key}
          open={palette.open}
          initialInput={palette.input}
          onOpenChange={(open) => setPalette((current) => ({ ...current, open }))}
          commands={commands}
          sheets={project.sheets}
          progress={progressBySheet}
          recentSheets={recentSheets}
          currentSheet={selectedSheetName}
          onOpenSheet={(sheetName) => void handleSheetSelect(sheetName, true)}
          onGoToRow={(target) => { if (selectedSheetName) void revealString(selectedSheetName, target); }}
        />
      ) : null}
    </main>
  );
}
