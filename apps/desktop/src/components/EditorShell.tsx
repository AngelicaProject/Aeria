import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { Effect, getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
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
import { WorkbenchToolDock, toolTitle, type GitPresentationMode, type WorkbenchTool } from "./WorkbenchToolDock";
import { detachedPanelTitle, type DetachedPanel } from "./DetachedToolWindow";
import { displayPathName } from "../pathDisplay";
import { initialWorkbenchLayout, reduceWorkbenchLayout } from "../ui/layout";
import {
  adjacentOccurrence,
  cursorBefore,
  emptyOccurrenceFilter,
  filterOccurrences,
  flattenTranslationRows,
  type OccurrenceFilter,
  type TranslationOccurrenceView,
} from "../translationOccurrences";
import { initialDocumentTabsState, reduceDocumentTabs, type DocumentTabsState } from "../documentTabs";
import { initialDockLayout, reduceDockLayout, type DockRegion } from "../dockLayout";
import { hasWindowsBackdrop } from "../ui/theme/windowBackdrop";
import { useTheme } from "../ui/theme/theme";
import { themeRegistry } from "../ui/theme/registry";
import { usePreferences } from "../ui/preferences";
import type { RowTarget } from "../commandPalette";
import { UiIcon } from "../ui/primitives/UiIcon";

const PAGE_SIZE = 100;

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
  onClosed: () => void;
};

const bottomPanelIds = new Set(["tasks", "gitChanges", "diagnostics"]);

function cursorForRow(row: TranslationRowDto): TranslationRowCursorDto {
  return { sheetName: row.sheetName, rowId: row.rowId, subrowId: row.subrowId };
}

function panelTitle(panelId: string | null): string {
  if (panelId === "sheets") return "Sheets";
  if (panelId === "search" || panelId === "ai" || panelId === "git") return toolTitle(panelId);
  return "Panel";
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT";
}

export function EditorShell({
  project,
  applicationWarning,
  onDismissApplicationWarning,
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

  const [selectedSheetName, setSelectedSheetName] = useState<string | null>(firstSheetName);
  const [loadedSheetName, setLoadedSheetName] = useState<string | null>(null);
  const [rows, setRows] = useState<TranslationRowDto[]>([]);
  const [nextAfter, setNextAfter] = useState<TranslationRowCursorDto | null>(null);
  const [selectedRowCursor, setSelectedRowCursor] = useState<TranslationRowCursorDto | null>(null);
  const [selectedBinding, setSelectedBinding] = useState<SourceBinding | null>(null);
  const [sheetLoading, setSheetLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [mutations, setMutations] = useState<CellMutation[]>([]);
  const [dirty, setDirty] = useState(false);
  const [closing, setClosing] = useState(false);
  const [editorError, setEditorError] = useState<EditorError | null>(null);
  const [discardRequest, setDiscardRequest] = useState<DiscardRequest | null>(null);
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
  const [gitMode, setGitMode] = useState<GitPresentationMode>("collaboration");
  const [workspaceRevision, setWorkspaceRevision] = useState(0);
  const [lensFilter, setLensFilter] = useState<OccurrenceFilter>(emptyOccurrenceFilter);
  const [progress, setProgress] = useState<readonly SheetProgressDto[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("appearance");
  const [palette, setPalette] = useState<{ open: boolean; input: string; key: number }>({ open: false, input: "", key: 0 });
  const [recentSheets, setRecentSheets] = useState<string[]>([]);
  const [pendingChanges, setPendingChanges] = useState<UnitChangeDto[] | null>(null);
  /** First row of a page loaded to reveal a string mid-sheet, or null when the list starts at the top. */
  const [listStart, setListStart] = useState<{ rowId: number; subrowId: number } | null>(null);
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

  const selectedRow = useMemo(
    () => selectedRowCursor ? rows.find((row) => rowKey(row) === rowKey(selectedRowCursor)) ?? null : null,
    [rows, selectedRowCursor],
  );
  const allOccurrences = useMemo(() => flattenTranslationRows(rows), [rows]);
  const visibleOccurrences = useMemo(() => filterOccurrences(allOccurrences, lensFilter), [allOccurrences, lensFilter]);
  const sheetHasNoRows = selectedSheetName !== null && loadedSheetName === selectedSheetName && !sheetLoading && rows.length === 0 && nextAfter === null;

  const progressBySheet = useMemo(() => new Map(progress.map((entry) => [entry.sheetName, entry])), [progress]);
  const projectProgress = useMemo(() => {
    const total = project.sheets.reduce((sum, sheet) => sum + sheet.translatableCellCount, 0);
    const translated = progress.reduce((sum, entry) => sum + entry.translated, 0);
    return total > 0 ? { translated, total } : null;
  }, [progress, project.sheets]);
  const selectedSheet = selectedSheetName ? sheetsByName.get(selectedSheetName) ?? null : null;
  const selectedSheetProgress = selectedSheet && selectedSheet.translatableCellCount > 0
    ? {
        translated: progressBySheet.get(selectedSheet.name)?.translated ?? 0,
        reviewed: progressBySheet.get(selectedSheet.name)?.reviewed ?? 0,
        total: selectedSheet.translatableCellCount,
      }
    : null;

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

  const beginSheetLoad = useCallback(async (sheetName: string, immediate = true, reveal: RowTarget | null = null) => {
    const generation = ++requestGeneration.current;
    pendingAdvance.current = null;
    const resetForSheet = () => {
      setSelectedSheetName(sheetName);
      setNextAfter(null);
      setDirty(false);
      hasDirtyDraft.current = false;
      setSheetLoading(true);
      setEditorError(null);
    };
    if (immediate) flushSync(resetForSheet);
    else resetForSheet();
    setRecentSheets((current) => [sheetName, ...current.filter((name) => name !== sheetName)].slice(0, RECENT_SHEET_LIMIT));

    const after = reveal ? cursorBefore(sheetName, reveal.rowId, reveal.subrowId) : null;
    try {
      const page = await pageTranslationRows(sheetName, after, PAGE_SIZE);
      if (generation !== requestGeneration.current) return;
      setRows(page.rows);
      setLoadedSheetName(sheetName);
      setNextAfter(page.nextAfter);
      setListStart(after && reveal ? { rowId: reveal.rowId, subrowId: reveal.subrowId } : null);
      const revealedRow = reveal ? page.rows.find((row) => row.rowId === reveal.rowId && row.subrowId === reveal.subrowId) : undefined;
      const revealedCell = revealedRow
        ? revealedRow.cells.find((cell) => reveal?.columnIndex === null || cell.sourceBinding.columnIndex === reveal?.columnIndex) ?? revealedRow.cells[0]
        : undefined;
      const row = revealedRow ?? page.rows[0];
      setSelectedRowCursor(row ? cursorForRow(row) : null);
      setSelectedBinding((revealedCell ?? row?.cells[0])?.sourceBinding ?? null);
      if (reveal && !revealedRow) {
        setEditorError({
          title: "String not found",
          tone: "warning",
          error: { code: "rowNotTranslatable", message: `${sheetName} ${reveal.rowId}:${reveal.subrowId} has no translatable string in the current source.` },
        });
      }
    } catch (error) {
      if (generation === requestGeneration.current) showError("Could not load sheet", error);
    } finally {
      if (generation === requestGeneration.current) setSheetLoading(false);
    }
  }, [showError]);

  useEffect(() => {
    if (firstSheetName) void beginSheetLoad(firstSheetName, false);
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
          if (!(await requestDiscardConfirmation("Closing Aeria will discard your unsaved changes."))) return;
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
  }, [requestDiscardConfirmation]);

  const handleSheetSelect = useCallback(async (sheetName: string, pin = false) => {
    if (sheetName === selectedSheetName) {
      const activeId = documentTabs.activeId;
      if (pin && activeId) setDocumentTabs((current) => reduceDocumentTabs(current, { type: "pin", id: activeId }));
      return;
    }
    if (!(await requestDiscardConfirmation("Changing sheets will discard your unsaved changes."))) return;
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "openSheet", sheetName, pin }));
    void beginSheetLoad(sheetName);
  }, [beginSheetLoad, documentTabs.activeId, requestDiscardConfirmation, selectedSheetName]);

  const handleDocumentSelect = useCallback(async (documentId: string) => {
    const document = documentTabs.tabs.find((tab) => tab.id === documentId);
    if (!document || document.id === documentTabs.activeId) return;
    if (!(await requestDiscardConfirmation("Changing sheets will discard your unsaved changes."))) return;
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "activate", id: documentId }));
    void beginSheetLoad(document.sheetName);
  }, [beginSheetLoad, documentTabs.activeId, documentTabs.tabs, requestDiscardConfirmation]);

  const handleDocumentClose = useCallback(async (documentId: string) => {
    const document = documentTabs.tabs.find((tab) => tab.id === documentId);
    if (!document) return;
    if (document.id === documentTabs.activeId && !(await requestDiscardConfirmation("Closing this sheet will discard your unsaved changes."))) return;
    const next = reduceDocumentTabs(documentTabs, { type: "close", id: documentId });
    setDocumentTabs(next);
    if (next.activeId && next.activeId !== documentTabs.activeId) {
      const nextDocument = next.tabs.find((tab) => tab.id === next.activeId);
      if (nextDocument) void beginSheetLoad(nextDocument.sheetName);
    }
  }, [beginSheetLoad, documentTabs, requestDiscardConfirmation]);

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
    if (!sameRow && !(await requestDiscardConfirmation("Changing rows will discard your unsaved changes."))) {
      focusTargetRequest.current = false;
      return;
    }
    pendingAdvance.current = null;

    flushSync(() => {
      setSelectedRowCursor(cursor);
      setSelectedBinding(occurrence.binding);
      if (!sameRow) {
        hasDirtyDraft.current = false;
        setDirty(false);
      }
      setEditorError(null);
    });
  }, [requestDiscardConfirmation, selectedRowCursor]);

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
    if (!selectedRow || !(await confirmMutationDiscard(otherDirty, "Saving the target will discard other unsaved changes."))) return;
    if (otherDirty) discardOtherDrafts();
    const key = bindingKey(cell.sourceBinding);
    const saved = await runMutation("target", key, "Could not save target", async () => {
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
  }, [applyOverlay, confirmMutationDiscard, preferences.focusTargetOnNext, runMutation, selectedRow]);

  const handleSaveNote = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => {
    const translation = cell.translation;
    if (!translation || !selectedRow || !(await confirmMutationDiscard(otherDirty, "Saving the note will discard other unsaved changes."))) return;
    if (otherDirty) discardOtherDrafts();
    await runMutation("note", bindingKey(cell.sourceBinding), "Could not save note", async () => {
      const overlay = await setTranslationNote(translation.translationUnitId, draft.note.length === 0 ? null : draft.note);
      applyOverlay(cell.sourceBinding, overlay);
    });
  }, [applyOverlay, confirmMutationDiscard, runMutation, selectedRow]);

  const handleReviewChange = useCallback(async (cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => {
    const translation = cell.translation;
    const shouldDiscardDrafts = hasDirtyDraft.current;
    if (!translation || translation.reviewState === reviewState || !(await confirmMutationDiscard(shouldDiscardDrafts, "Changing the review state will discard your unsaved changes."))) return;
    if (shouldDiscardDrafts) discardDrafts();
    await runMutation("review", bindingKey(cell.sourceBinding), "Could not change review state", async () => {
      const overlay = await setTranslationReviewState(translation.translationUnitId, reviewState);
      applyOverlay(cell.sourceBinding, overlay);
    });
  }, [applyOverlay, confirmMutationDiscard, runMutation]);

  const handleLoadMore = useCallback(async () => {
    if (!selectedSheetName || !nextAfter || loadingMore) return;
    const generation = requestGeneration.current;
    const after = nextAfter;
    flushSync(() => setLoadingMore(true));

    try {
      const page = await pageTranslationRows(selectedSheetName, after, PAGE_SIZE);
      if (generation !== requestGeneration.current) return;
      setRows((current) => {
        const seen = new Set(current.map((row) => rowKey(row)));
        return [...current, ...page.rows.filter((row) => !seen.has(rowKey(row)))];
      });
      if (rows.length === 0 && page.rows[0]) {
        setSelectedRowCursor(cursorForRow(page.rows[0]));
        setSelectedBinding(page.rows[0].cells[0]?.sourceBinding ?? null);
      }
      setNextAfter(page.nextAfter);
    } catch (error) {
      if (generation === requestGeneration.current) showError("Could not load more rows", error);
    } finally {
      setLoadingMore(false);
    }
  }, [loadingMore, nextAfter, rows.length, selectedSheetName, showError]);

  const handleClose = useCallback(async () => {
    if (!(await requestDiscardConfirmation("Closing the project will discard your unsaved changes."))) return;
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
      showError("Could not close project", error);
    }
  }, [onClosed, requestDiscardConfirmation, showError]);

  const handleWindowClose = useCallback(async () => {
    if (!(await requestDiscardConfirmation("Closing Aeria will discard your unsaved changes."))) return;
    allowWindowClose.current = true;
    try {
      await getCurrentWindow().close();
    } catch {
      allowWindowClose.current = false;
    }
  }, [requestDiscardConfirmation]);

  const selectedUnitId = useMemo(() => {
    if (!selectedRow || !selectedBinding) return null;
    const key = bindingKey(selectedBinding);
    return selectedRow.cells.find((cell) => bindingKey(cell.sourceBinding) === key)?.translation?.translationUnitId ?? null;
  }, [selectedRow, selectedBinding]);

  const handleWorkspaceChanged = useCallback(() => {
    setWorkspaceRevision((current) => current + 1);
    if (selectedSheetName) void beginSheetLoad(selectedSheetName, false);
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

  /** Opens a string by coordinate, paging directly to it when it is not loaded. */
  const revealString = useCallback(async (sheetName: string, target: RowTarget) => {
    if (sheetName === selectedSheetName && loadedSheetName === sheetName) {
      const occurrence = allOccurrences.find((candidate) =>
        candidate.binding.rowId === target.rowId
        && candidate.binding.subrowId === target.subrowId
        && (target.columnIndex === null || candidate.binding.columnIndex === target.columnIndex));
      if (occurrence) {
        if (!visibleOccurrences.includes(occurrence)) setLensFilter(emptyOccurrenceFilter);
        await handleOccurrenceSelect(occurrence);
        return;
      }
    }
    if (!sheetsByName.has(sheetName)) {
      setEditorError({ title: "Sheet not found", tone: "warning", error: { code: "sheetNotFound", message: `${sheetName} is not in the current source.` } });
      return;
    }
    if (!(await requestDiscardConfirmation("Opening this string will discard your unsaved changes."))) return;
    if (sheetName !== selectedSheetName) setDocumentTabs((current) => reduceDocumentTabs(current, { type: "openSheet", sheetName, pin: true }));
    setLensFilter(emptyOccurrenceFilter);
    void beginSheetLoad(sheetName, true, target);
  }, [allOccurrences, beginSheetLoad, handleOccurrenceSelect, loadedSheetName, requestDiscardConfirmation, selectedSheetName, sheetsByName, visibleOccurrences]);

  const revealBinding = useCallback((binding: SourceBinding) => {
    void revealString(binding.sheetName, { rowId: binding.rowId, subrowId: binding.subrowId, columnIndex: binding.columnIndex });
  }, [revealString]);

  const openPalette = useCallback((input: string) => {
    setPalette((current) => ({ open: true, input, key: current.key + 1 }));
  }, []);

  const openSettings = useCallback((section: SettingsSection = "appearance") => {
    setSettingsSection(section);
    setSettingsOpen(true);
  }, []);

  const handleRestoreTarget = useCallback(async (targetMacro: string) => {
    if (!selectedBinding) return;
    if (!(await requestDiscardConfirmation("Restoring this text will discard your unsaved changes."))) return;
    const binding = selectedBinding;
    await runMutation("target", bindingKey(binding), "Could not restore translation", async () => {
      const overlay = await setTranslationTarget(binding, targetMacro);
      applyOverlay(binding, overlay);
    });
  }, [applyOverlay, requestDiscardConfirmation, runMutation, selectedBinding]);

  // Layout ---------------------------------------------------------------

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
      .map((region) => ({ id: region, label: region[0]!.toUpperCase() + region.slice(1) }));
  }, []);

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
        title: `${detachedPanelTitle(panel)} · Aeria`,
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
      showError("Could not open tool window", error);
    }
  }, [detachedPanel, dockLayoutState.placements, showError]);

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
  }));

  const sheetHeaderActions = <>
    <IconButton icon={hideEmptySheets ? "eyeOff" : "eye"} label={hideEmptySheets ? "Show empty sheets" : "Hide empty sheets"} pressed={hideEmptySheets} disabled={closing} onClick={() => setHideEmptySheets((current) => !current)} />
    <IconButton icon="search" label="Filter sheets" shortcut="Ctrl+F" disabled={closing} onClick={focusSheetFilter} />
    <IconButton icon="locateFixed" label="Reveal active sheet" disabled={closing || !selectedSheetName} onClick={() => setRevealSheetSignal((current) => current + 1)} />
    <IconButton icon="chevronsUp" label="Collapse all folders" disabled={closing} onClick={() => setCollapseSheetsSignal((current) => current + 1)} />
  </>;

  const renderPanelContent = (panelId: string, active: boolean): ReactNode => {
    if (panelId === "sheets") {
      return <SheetSidebar sheets={project.sheets} selectedSheetName={selectedSheetName} disabled={closing} active={active} hideEmpty={hideEmptySheets} onHideEmptyChange={setHideEmptySheets} filterOpen={sheetFilterOpen} onFilterOpenChange={setSheetFilterOpen} onOpenFilter={focusSheetFilter} quickFindSignal={quickFindSignal} revealSignal={revealSheetSignal} collapseSignal={collapseSheetsSignal} onSelect={handleSheetSelect} progress={progressBySheet} />;
    }
    const tool: WorkbenchTool = panelId === "git" ? "git" : panelId === "search" ? "search" : "ai";
    return <WorkbenchToolDock activeTool={tool} gitMode={gitMode} selectedBinding={selectedBinding} onGitModeChange={setGitMode} selectedUnitId={selectedUnitId} workspaceRevision={workspaceRevision} onWorkspaceChanged={handleWorkspaceChanged} onRestoreTarget={(target) => void handleRestoreTarget(target)} pending={{ changes: pendingChanges, refresh: refreshPendingChanges }} onRevealBinding={revealBinding} />;
  };

  const renderDock = (region: "left" | "right", panelId: string | null, open: boolean) => {
    const id = panelId ?? (region === "left" ? "sheets" : activeTool);
    const floatable = id === "ai" || id === "git" || id === "search";
    return (
      <DockPanel
        panelId={id}
        title={panelTitle(id)}
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

  const menus: readonly ApplicationMenuDefinition[] = [
    {
      id: "file",
      label: "File",
      items: [
        { kind: "command", id: "settings", label: "Settings…", shortcut: "Ctrl+,", onSelect: () => openSettings() },
        { kind: "separator", id: "file-sep-1" },
        { kind: "command", id: "close-tab", label: "Close sheet", shortcut: "Ctrl+W", ...(documentTabs.activeId ? { onSelect: () => void handleDocumentClose(documentTabs.activeId!) } : {}) },
        { kind: "command", id: "close-project", label: "Close project", onSelect: () => void handleClose() },
        { kind: "separator", id: "file-sep-2" },
        { kind: "command", id: "exit", label: "Exit", onSelect: () => void handleWindowClose() },
      ],
    },
    {
      id: "translation",
      label: "Translation",
      items: [
        { kind: "command", id: "save", label: "Save target", shortcut: "Ctrl+S", ...(selectedRow ? { onSelect: () => editorRef.current?.saveTarget(false) } : {}) },
        { kind: "command", id: "save-next", label: "Save and go to next", shortcut: "Ctrl+Enter", ...(selectedRow ? { onSelect: () => editorRef.current?.saveTarget(true) } : {}) },
        { kind: "command", id: "copy-source", label: "Copy source to target", ...(selectedRow ? { onSelect: () => editorRef.current?.copySource() } : {}) },
        { kind: "command", id: "revert", label: "Revert unsaved changes", ...(dirty ? { onSelect: () => editorRef.current?.revert() } : {}) },
      ],
    },
    {
      id: "go",
      label: "Go",
      items: [
        { kind: "command", id: "go-sheet", label: "Go to sheet…", shortcut: "Ctrl+P", onSelect: () => openPalette("") },
        { kind: "command", id: "go-row", label: "Go to row…", shortcut: "Ctrl+G", ...(selectedSheetName ? { onSelect: () => openPalette(":") } : {}) },
        { kind: "command", id: "command-palette", label: "Command palette…", shortcut: "Ctrl+Shift+P", onSelect: () => openPalette(">") },
        { kind: "separator", id: "go-sep" },
        { kind: "command", id: "go-next", label: "Next string", shortcut: "Alt+Down", onSelect: () => navigateOccurrence(1) },
        { kind: "command", id: "go-previous", label: "Previous string", shortcut: "Alt+Up", onSelect: () => navigateOccurrence(-1) },
      ],
    },
    {
      id: "view",
      label: "View",
      items: [
        { kind: "command", id: "sheets", label: "Sheets", shortcut: "Ctrl+B", checked: isPanelShown("sheets"), onSelect: () => showPanel("sheets", "left") },
        { kind: "command", id: "search", label: "Search", checked: isPanelShown("search"), onSelect: () => showPanel("search", "left") },
        { kind: "command", id: "git", label: "Git", checked: isPanelShown("git"), onSelect: () => showPanel("git", "right") },
        { kind: "command", id: "ai", label: "AI assist", checked: isPanelShown("ai"), onSelect: () => showPanel("ai", "right") },
        { kind: "command", id: "bottom", label: "Bottom panel", shortcut: "Ctrl+J", checked: bottomOpen, onSelect: () => dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" }) },
        { kind: "separator", id: "view-sep-1" },
        { kind: "command", id: "filter-sheets", label: "Filter sheets", shortcut: "Ctrl+F", onSelect: handleQuickFind },
        { kind: "command", id: "reveal-sheet", label: "Reveal active sheet", ...(selectedSheetName ? { onSelect: () => { showPanel("sheets", "left", false); setRevealSheetSignal((current) => current + 1); } } : {}) },
        { kind: "separator", id: "view-sep-2" },
        {
          kind: "submenu",
          id: "theme",
          label: "Theme",
          items: [{ kind: "radio", id: "theme-radio", label: "Theme", value: theme.id, items: themeRegistry.map((entry) => ({ value: entry.id, label: `${entry.displayName} (${entry.family ?? "Themes"})` })), onSelect: setThemeId }],
        },
      ],
    },
  ];

  const commands: PaletteCommand[] = [
    { id: "go-sheet", category: "Go", title: "Go to sheet", shortcut: "Ctrl+P", icon: "table2", run: () => openPalette("") },
    { id: "go-row", category: "Go", title: "Go to row in current sheet", shortcut: "Ctrl+G", icon: "arrowRight", enabled: selectedSheetName !== null, run: () => openPalette(":") },
    { id: "go-next", category: "Go", title: "Next string", shortcut: "Alt+Down", icon: "arrowDown", run: () => navigateOccurrence(1) },
    { id: "go-previous", category: "Go", title: "Previous string", shortcut: "Alt+Up", icon: "arrowUp", run: () => navigateOccurrence(-1) },
    { id: "save", category: "Translation", title: "Save target", shortcut: "Ctrl+S", icon: "save", enabled: selectedRow !== null, run: () => editorRef.current?.saveTarget(false) },
    { id: "save-next", category: "Translation", title: "Save and go to next string", shortcut: "Ctrl+Enter", icon: "save", enabled: selectedRow !== null, run: () => editorRef.current?.saveTarget(true) },
    { id: "copy-source", category: "Translation", title: "Copy source to target", icon: "copyPlus", enabled: selectedRow !== null, run: () => editorRef.current?.copySource() },
    { id: "revert", category: "Translation", title: "Revert unsaved changes", icon: "undo", enabled: dirty, run: () => editorRef.current?.revert() },
    { id: "filter-untranslated", category: "Strings", title: "Show only untranslated loaded strings", icon: "listFilter", run: () => setLensFilter({ status: "untranslated", query: "" }) },
    { id: "filter-review", category: "Strings", title: "Show only loaded strings that need review", icon: "listFilter", run: () => setLensFilter({ status: "needsReview", query: "" }) },
    { id: "filter-clear", category: "Strings", title: "Clear string filters", icon: "x", run: () => setLensFilter(emptyOccurrenceFilter) },
    { id: "view-sheets", category: "View", title: "Toggle Sheets", shortcut: "Ctrl+B", icon: "table2", run: () => showPanel("sheets", "left") },
    { id: "view-search", category: "View", title: "Toggle Search", icon: "search", run: () => showPanel("search", "left") },
    { id: "view-git", category: "View", title: "Toggle Git", icon: "gitBranch", run: () => showPanel("git", "right") },
    { id: "view-ai", category: "View", title: "Toggle AI assist", icon: "sparkles", run: () => showPanel("ai", "right") },
    { id: "view-bottom", category: "View", title: "Toggle bottom panel", shortcut: "Ctrl+J", icon: "panelBottom", run: () => dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" }) },
    { id: "view-filter-sheets", category: "View", title: "Filter sheets", shortcut: "Ctrl+F", icon: "search", run: handleQuickFind },
    { id: "view-reveal-sheet", category: "View", title: "Reveal active sheet", icon: "locateFixed", enabled: selectedSheetName !== null, run: () => { showPanel("sheets", "left", false); setRevealSheetSignal((current) => current + 1); } },
    { id: "git-open", category: "Git", title: "Show changes", icon: "gitBranch", run: () => showPanel("git", "right", false) },
    { id: "prefs-settings", category: "Preferences", title: "Open settings", shortcut: "Ctrl+,", icon: "settings", run: () => openSettings() },
    { id: "prefs-theme", category: "Preferences", title: "Color theme", icon: "palette", run: () => openSettings("appearance") },
    { id: "prefs-shortcuts", category: "Preferences", title: "Keyboard shortcuts", icon: "listFilter", run: () => openSettings("keyboard") },
    ...themeRegistry.map((entry): PaletteCommand => ({ id: `theme-${entry.id}`, category: "Theme", title: `${entry.displayName} (${entry.family ?? "Themes"})`, icon: "palette", run: () => setThemeId(entry.id) })),
    { id: "file-close-sheet", category: "File", title: "Close sheet", shortcut: "Ctrl+W", icon: "x", enabled: documentTabs.activeId !== null, run: () => { if (documentTabs.activeId) void handleDocumentClose(documentTabs.activeId); } },
    { id: "file-close-project", category: "File", title: "Close project", icon: "folder", run: () => void handleClose() },
  ];

  const bottomIsTool = bottomPanelId !== null && !bottomPanelIds.has(bottomPanelId);

  return (
    <main className="workbench">
      <WindowChrome
        mode="workbench"
        menus={menus}
        center={(
          <button className="command-center" type="button" onClick={() => openPalette("")} title="Go to sheet, run commands, or search (Ctrl+P)">
            <UiIcon icon="search" size="sm" />
            <span className="command-center-label"><strong>{projectName}</strong>{selectedSheetName ? <span> / {selectedSheetName}</span> : null}</span>
            <kbd>Ctrl P</kbd>
          </button>
        )}
        onClose={() => void handleWindowClose()}
        actions={<>
          <IconButton icon="panelLeft" label="Toggle left panel" shortcut="Ctrl+B" pressed={leftDockOpen} onClick={() => dispatchLayout({ type: "toggleRegion", regionId: "leftDock" })} />
          <IconButton icon="panelBottom" label="Toggle bottom panel" shortcut="Ctrl+J" pressed={bottomOpen} onClick={() => dispatchLayout({ type: "toggleRegion", regionId: "bottomPanel" })} />
          <IconButton icon="panelRight" label="Toggle right panel" pressed={rightDockOpen} onClick={() => dispatchLayout({ type: "toggleRegion", regionId: "rightDock" })} />
        </>}
      />
      {applicationWarning || editorError ? (
        <div className="notices" aria-live="polite">
          {applicationWarning ? <ErrorBanner tone="warning" title="Project opened with a Recent projects warning" error={applicationWarning} onDismiss={onDismissApplicationWarning} /> : null}
          {editorError ? <ErrorBanner title={editorError.title} error={editorError.error} onDismiss={() => setEditorError(null)} /> : null}
        </div>
      ) : null}
      <div
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
            { id: "sheets", label: "Sheets", icon: "table2", shortcut: "Ctrl+B", active: isPanelShown("sheets"), onSelect: () => showPanel("sheets", "left") },
            { id: "search", label: "Search", icon: "search", active: isPanelShown("search"), onSelect: () => showPanel("search", "left") },
          ]}
          footer={[{ id: "settings", label: "Settings", icon: "settings", shortcut: "Ctrl+,", onSelect: () => openSettings() }]}
        />
        {renderDock("left", leftPanelId, leftDockOpen)}
        {leftDockOpen && leftPanelId && detachedPanel !== leftPanelId ? <ResizeHandle axis="x" label="Resize left panel" onDelta={(delta) => dispatchLayout({ type: "resizeRegion", regionId: "leftDock", delta })} /> : null}

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
            {!selectedSheetName ? (
              <div className="document-empty empty-state">
                <strong>No sheet open</strong>
                <p>Choose a sheet in the Sheets panel to start translating.</p>
              </div>
            ) : sheetHasNoRows ? (
              <div className="document-empty empty-state">
                <strong>No translatable rows</strong>
                <p>This sheet doesn't contain any source strings that can be translated.</p>
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
                  loadingMore={loadingMore}
                  hasMore={nextAfter !== null}
                  onSelect={(occurrence) => void handleOccurrenceSelect(occurrence)}
                  onNavigate={navigateOccurrence}
                  onLoadMore={() => void handleLoadMore()}
                  changedKinds={changedKinds}
                  listStart={listStart}
                  onLoadFromStart={() => { if (selectedSheetName) void beginSheetLoad(selectedSheetName); }}
                />
                <ResizeHandle axis="y" label="Resize translation editor" onDelta={(delta) => dispatchLayout({ type: "resizeRegion", regionId: "editor", delta: -delta })} />
                <TranslationEditor
                  ref={editorRef}
                  key={selectedRow ? rowKey(selectedRow) : "empty-editor"}
                  row={selectedRow}
                  selectedBinding={selectedBinding}
                  sourceLanguage={project.sourceLanguage}
                  mutations={mutations}
                  onDirtyChange={handleDirtyChange}
                  onSelectCell={handleFieldSelect}
                  onSaveTarget={(...args) => void handleSaveTarget(...args)}
                  onSaveNote={(...args) => void handleSaveNote(...args)}
                  onReviewChange={(...args) => void handleReviewChange(...args)}
                  onNavigate={navigateFromEditor}
                  takeFocusRequest={takeFocusRequest}
                  checkpoint={selectedCheckpoint}
                />
              </div>
            )}
          </section>
          {bottomOpen && bottomPanelId && detachedPanel !== bottomPanelId ? <>
            <ResizeHandle axis="y" label="Resize bottom panel" onDelta={(delta) => dispatchLayout({ type: "resizeRegion", regionId: "bottomPanel", delta: -delta })} />
            {bottomIsTool ? (
              <DockPanel
                panelId={bottomPanelId}
                title={panelTitle(bottomPanelId)}
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

        {rightDockOpen && rightPanelId && detachedPanel !== rightPanelId ? <ResizeHandle axis="x" label="Resize right panel" onDelta={(delta) => dispatchLayout({ type: "resizeRegion", regionId: "rightDock", delta: -delta })} /> : null}
        {renderDock("right", rightPanelId, rightDockOpen)}
        <ActivityRail
          side="right"
          items={[
            { id: "git", label: "Git", icon: "gitBranch", active: isPanelShown("git"), onSelect: () => showPanel("git", "right") },
            { id: "ai", label: "AI assist", icon: "sparkles", active: isPanelShown("ai"), onSelect: () => showPanel("ai", "right") },
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
      />
      <ConfirmDialog
        open={discardRequest !== null}
        message={discardRequest?.message ?? ""}
        onKeepEditing={() => resolveDiscardConfirmation(false)}
        onDiscard={() => resolveDiscardConfirmation(true)}
      />
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} initialSection={settingsSection} />
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
    </main>
  );
}
