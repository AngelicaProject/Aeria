import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type CSSProperties } from "react";
import { flushSync } from "react-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  closeProject,
  normalizeCommandError,
  pageTranslationRows,
  setTranslationNote,
  setTranslationReviewState,
  setTranslationTarget,
} from "../ipc";
import { bindingKey, rowKey } from "../binding";
import type {
  CommandError,
  ProjectSummaryDto,
  ReviewState,
  SourceBinding,
  TranslationCellDto,
  TranslationOverlayDto,
  TranslationRowCursorDto,
  TranslationRowDto,
} from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { ConfirmDialog } from "./ConfirmDialog";
import { ActivityRail } from "./ActivityRail";
import { BottomPanel } from "./BottomPanel";
import { DocumentTabs, type DocumentTab } from "./DocumentTabs";
import { DockPanel } from "./DockPanel";
import { ResizeHandle } from "./ResizeHandle";
import { SheetSidebar } from "./SheetSidebar";
import { StatusBar } from "./StatusBar";
import { CellDraft, CellMutation, TranslationEditor } from "./TranslationEditor";
import { TranslationList } from "./TranslationList";
import { WindowChrome } from "./WindowChrome";
import { WorkbenchToolDock, type GitPresentationMode, type WorkbenchTool } from "./WorkbenchToolDock";
import { displayPathName } from "../pathDisplay";
import { initialWorkbenchLayout, reduceWorkbenchLayout } from "../ui/layout";
import type { TranslationOccurrenceView } from "../translationOccurrences";
import { documentIdForSheet, initialDocumentTabsState, reduceDocumentTabs, type DocumentTabsState } from "../documentTabs";
import { initialDockLayout, reduceDockLayout, type DockRegion } from "../dockLayout";

const PAGE_SIZE = 100;

type EditorError = {
  title: string;
  error: CommandError;
};

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

type DetachedPanel = "search" | "ai" | "git" | "tasks" | "gitChanges" | "diagnostics";

function cursorForRow(row: TranslationRowDto): TranslationRowCursorDto {
  return { sheetName: row.sheetName, rowId: row.rowId, subrowId: row.subrowId };
}

function repositoryName(path: string): string {
  return displayPathName(path);
}

export function EditorShell({
  project,
  applicationWarning,
  onDismissApplicationWarning,
  onClosed,
}: EditorShellProps) {
  const firstSheetName = project.sheets[0]?.name ?? null;
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
  const [leftTool, setLeftTool] = useState<"sheets" | "search">("sheets");
  const [quickFindSignal, setQuickFindSignal] = useState(0);
  const [detachedPanel, setDetachedPanel] = useState<DetachedPanel | null>(null);
  const [dockLayoutState, setDockLayoutState] = useState(initialDockLayout);
  const [activeTool, setActiveTool] = useState<WorkbenchTool>("ai");
  const [gitMode, setGitMode] = useState<GitPresentationMode>("collaboration");
  const leftDockOpen = layout.regions.leftDock.visible;
  const requestGeneration = useRef(0);
  const hasDirtyDraft = useRef(false);
  const allowWindowClose = useRef(false);
  const discardRequestRef = useRef<DiscardRequest | null>(null);
  const mutationKeys = useRef(new Set<string>());

  const selectedRow = useMemo(
    () => selectedRowCursor ? rows.find((row) => rowKey(row) === rowKey(selectedRowCursor)) ?? null : null,
    [rows, selectedRowCursor],
  );
  const sheetHasNoRows = selectedSheetName !== null && loadedSheetName === selectedSheetName && !sheetLoading && rows.length === 0 && nextAfter === null;

  const showError = useCallback((title: string, error: unknown) => {
    setEditorError({ title, error: normalizeCommandError(error) });
  }, []);

  const handleDirtyChange = useCallback((dirty: boolean) => {
    hasDirtyDraft.current = dirty;
    setDirty(dirty);
    const activeDocumentId = documentTabs.activeId;
    if (activeDocumentId) {
      setDocumentTabs((current) => reduceDocumentTabs(current, { type: "setDirty", id: activeDocumentId, dirty }));
    }
  }, [documentTabs.activeId]);

  const applyOverlay = useCallback((sourceBinding: TranslationCellDto["sourceBinding"], translation: TranslationOverlayDto) => {
    const targetKey = bindingKey(sourceBinding);
    setRows((current) => current.map((row) => {
      let changed = false;
      const cells = row.cells.map((cell) => {
        if (bindingKey(cell.sourceBinding) !== targetKey) {
          return cell;
        }
        changed = true;
        return { ...cell, translation };
      });
      return changed ? { ...row, cells } : row;
    }));
  }, []);

  const beginSheetLoad = useCallback(async (sheetName: string, immediate = true) => {
    const generation = ++requestGeneration.current;
    const resetForSheet = () => {
      setSelectedSheetName(sheetName);
      setNextAfter(null);
      setDirty(false);
      hasDirtyDraft.current = false;
      setSheetLoading(true);
      setEditorError(null);
    };
    if (immediate) {
      flushSync(resetForSheet);
    } else {
      resetForSheet();
    }

    try {
      const page = await pageTranslationRows(sheetName, null, PAGE_SIZE);
      if (generation !== requestGeneration.current) {
        return;
      }
      setRows(page.rows);
      setLoadedSheetName(sheetName);
      setNextAfter(page.nextAfter);
      const firstRow = page.rows[0];
      setSelectedRowCursor(firstRow ? cursorForRow(firstRow) : null);
      setSelectedBinding(firstRow?.cells[0]?.sourceBinding ?? null);
    } catch (error) {
      if (generation === requestGeneration.current) {
        showError("Could not load sheet", error);
      }
    } finally {
      if (generation === requestGeneration.current) {
        setSheetLoading(false);
      }
    }
  }, [showError]);

  useEffect(() => {
    if (firstSheetName) {
      void beginSheetLoad(firstSheetName, false);
    }
  }, [beginSheetLoad, firstSheetName]);

  const requestDiscardConfirmation = useCallback((message: string): Promise<boolean> => {
    if (!hasDirtyDraft.current) {
      return Promise.resolve(true);
    }
    if (discardRequestRef.current) {
      return Promise.resolve(false);
    }
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
        if (!hasDirtyDraft.current) {
          return;
        }
        event.preventDefault();
        void (async () => {
          if (!(await requestDiscardConfirmation("Closing Aeria will discard your unsaved changes."))) {
            return;
          }
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
    setLeftTool("sheets");
    void beginSheetLoad(sheetName);
  }, [beginSheetLoad, documentTabs.activeId, requestDiscardConfirmation, selectedSheetName]);

  const handleDocumentSelect = useCallback(async (documentId: string) => {
    const document = documentTabs.tabs.find((tab) => tab.id === documentId);
    if (!document || document.id === documentTabs.activeId) return;
    if (!(await requestDiscardConfirmation("Changing documents will discard your unsaved changes."))) return;
    setDocumentTabs((current) => reduceDocumentTabs(current, { type: "activate", id: documentId }));
    void beginSheetLoad(document.sheetName);
  }, [beginSheetLoad, documentTabs.activeId, documentTabs.tabs, requestDiscardConfirmation]);

  const handleDocumentClose = useCallback(async (documentId: string) => {
    const document = documentTabs.tabs.find((tab) => tab.id === documentId);
    if (!document) return;
    if (document.id === documentTabs.activeId && !(await requestDiscardConfirmation("Closing this document will discard your unsaved changes."))) return;
    const next = reduceDocumentTabs(documentTabs, { type: "close", id: documentId });
    setDocumentTabs(next);
    if (next.activeId && next.activeId !== documentTabs.activeId) {
      const nextDocument = next.tabs.find((tab) => tab.id === next.activeId);
      if (nextDocument) void beginSheetLoad(nextDocument.sheetName);
    }
  }, [beginSheetLoad, documentTabs, requestDiscardConfirmation]);

  useEffect(() => {
    function handleWorkbenchShortcut(event: KeyboardEvent) {
      if (event.ctrlKey && event.key.toLocaleLowerCase() === "w" && documentTabs.activeId) {
        event.preventDefault();
        void handleDocumentClose(documentTabs.activeId);
      }
    }
    window.addEventListener("keydown", handleWorkbenchShortcut);
    return () => window.removeEventListener("keydown", handleWorkbenchShortcut);
  }, [documentTabs.activeId, handleDocumentClose]);

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
    if (!sameRow && !(await requestDiscardConfirmation("Changing rows will discard your unsaved changes."))) return;

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

  const handleFieldSelect = useCallback((binding: SourceBinding) => {
    setSelectedBinding(binding);
  }, []);

  const confirmMutationDiscard = useCallback((shouldConfirm: boolean, message: string): Promise<boolean> => {
    return shouldConfirm ? requestDiscardConfirmation(message) : Promise.resolve(true);
  }, [requestDiscardConfirmation]);

  const handleSaveTarget = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => {
    const key = bindingKey(cell.sourceBinding);
    if (!selectedRow || !(await confirmMutationDiscard(otherDirty, "Saving the target will discard other unsaved changes."))) {
      return;
    }
    if (otherDirty) discardOtherDrafts();
    if (mutationKeys.current.has(key)) return;
    flushSync(() => {
      mutationKeys.current.add(key);
      setMutations((current) => [...current, { kind: "target", bindingKey: key }]);
      setEditorError(null);
    });
    try {
      const overlay = await setTranslationTarget(cell.sourceBinding, draft.target);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not save target", error);
    } finally {
      mutationKeys.current.delete(key);
      setMutations((current) => current.filter((mutation) => mutation.bindingKey !== key));
    }
  }, [applyOverlay, confirmMutationDiscard, selectedRow, showError]);

  const handleSaveNote = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => {
    const translation = cell.translation;
    const key = bindingKey(cell.sourceBinding);
    if (!translation || !selectedRow || !(await confirmMutationDiscard(otherDirty, "Saving the note will discard other unsaved changes."))) {
      return;
    }
    if (otherDirty) discardOtherDrafts();
    if (mutationKeys.current.has(key)) return;
    flushSync(() => {
      mutationKeys.current.add(key);
      setMutations((current) => [...current, { kind: "note", bindingKey: key }]);
      setEditorError(null);
    });
    try {
      const overlay = await setTranslationNote(translation.translationUnitId, draft.note.length === 0 ? null : draft.note);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not save note", error);
    } finally {
      mutationKeys.current.delete(key);
      setMutations((current) => current.filter((mutation) => mutation.bindingKey !== key));
    }
  }, [applyOverlay, confirmMutationDiscard, selectedRow, showError]);

  const handleReviewChange = useCallback(async (cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => {
    const translation = cell.translation;
    const shouldDiscardDrafts = hasDirtyDraft.current;
    if (!translation || translation.reviewState === reviewState || !(await confirmMutationDiscard(shouldDiscardDrafts, "Changing review state will discard your unsaved changes."))) {
      return;
    }
    if (shouldDiscardDrafts) discardDrafts();
    const key = bindingKey(cell.sourceBinding);
    if (mutationKeys.current.has(key)) return;
    flushSync(() => {
      mutationKeys.current.add(key);
      setMutations((current) => [...current, { kind: "review", bindingKey: key }]);
      setEditorError(null);
    });
    try {
      const overlay = await setTranslationReviewState(translation.translationUnitId, reviewState);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not change review state", error);
    } finally {
      mutationKeys.current.delete(key);
      setMutations((current) => current.filter((mutation) => mutation.bindingKey !== key));
    }
  }, [applyOverlay, confirmMutationDiscard, showError]);

  const handleLoadMore = useCallback(async () => {
    if (!selectedSheetName || !nextAfter || loadingMore) {
      return;
    }

    const generation = requestGeneration.current;
    const after = nextAfter;
    flushSync(() => setLoadingMore(true));

    try {
      const page = await pageTranslationRows(selectedSheetName, after, PAGE_SIZE);
      if (generation !== requestGeneration.current) {
        return;
      }
      setRows((current) => {
        const appended = [...current];
        for (const row of page.rows) {
          if (!appended.some((existing) => rowKey(existing) === rowKey(row))) {
            appended.push(row);
          }
        }
        return appended;
      });
      if (rows.length === 0 && page.rows[0]) {
        setSelectedRowCursor(cursorForRow(page.rows[0]));
        setSelectedBinding(page.rows[0].cells[0]?.sourceBinding ?? null);
      }
      setNextAfter(page.nextAfter);
    } catch (error) {
      if (generation === requestGeneration.current) {
        showError("Could not load more rows", error);
      }
    } finally {
      setLoadingMore(false);
    }
  }, [loadingMore, nextAfter, rows.length, selectedSheetName, showError]);

  const handleClose = useCallback(async () => {
    if (!(await requestDiscardConfirmation("Closing the project will discard your unsaved changes."))) {
      return;
    }
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
    if (!(await requestDiscardConfirmation("Closing Aeria will discard your unsaved changes."))) {
      return;
    }
    allowWindowClose.current = true;
    try {
      await getCurrentWindow().close();
    } catch {
      allowWindowClose.current = false;
    }
  }, [requestDiscardConfirmation]);
  const handleLoadMoreClick = useCallback(() => {
    void handleLoadMore();
  }, [handleLoadMore]);
  const resizeLeftDock = useCallback((delta: number) => {
    dispatchLayout({ type: "resizeRegion", regionId: "leftDock", delta });
  }, []);
  const resizeTranslation = useCallback((delta: number) => {
    dispatchLayout({ type: "resizeRegion", regionId: "translation", delta: -delta });
  }, []);
  const handleSaveTargetClick = useCallback((cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => {
    void handleSaveTarget(cell, draft, otherDirty, discardOtherDrafts);
  }, [handleSaveTarget]);
  const handleSaveNoteClick = useCallback((cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean, discardOtherDrafts: () => void) => {
    void handleSaveNote(cell, draft, otherDirty, discardOtherDrafts);
  }, [handleSaveNote]);
  const handleReviewChangeClick = useCallback((cell: TranslationCellDto, reviewState: ReviewState, discardDrafts: () => void) => {
    void handleReviewChange(cell, reviewState, discardDrafts);
  }, [handleReviewChange]);
  const resizeRightDock = useCallback((delta: number) => {
    dispatchLayout({ type: "resizeRegion", regionId: "rightDock", delta: -delta });
  }, []);
  const resizeBottomPanel = useCallback((delta: number) => {
    dispatchLayout({ type: "resizeRegion", regionId: "bottomPanel", delta: -delta });
  }, []);
  const handleToolSelect = useCallback((tool: WorkbenchTool) => {
    if (tool === "search") {
      setDockLayoutState((current) => reduceDockLayout(current, { type: "move", panelId: "search", region: "left" }));
      if (leftTool === "search" && leftDockOpen) {
        dispatchLayout({ type: "setRegionVisibility", regionId: "leftDock", visible: false });
      } else {
        setLeftTool("search");
        dispatchLayout({ type: "setRegionVisibility", regionId: "leftDock", visible: true });
      }
      return;
    }
    if (activeTool === tool && layout.regions.rightDock.visible) {
      dispatchLayout({ type: "setRegionVisibility", regionId: "rightDock", visible: false });
      return;
    }
    setDockLayoutState((current) => reduceDockLayout(current, { type: "move", panelId: tool, region: "right" }));
    setActiveTool(tool);
    dispatchLayout({ type: "setRegionVisibility", regionId: "rightDock", visible: true });
    dispatchLayout({ type: "setActiveTab", regionId: "rightDock", tabId: tool });
  }, [activeTool, layout.regions.rightDock.visible, leftDockOpen, leftTool]);
  const handlePanelMove = useCallback((panelId: string, region: string) => {
    if (region !== "left" && region !== "right" && region !== "bottom") return;
    setDockLayoutState((current) => reduceDockLayout(current, { type: "move", panelId, region: region as DockRegion }));
    if (panelId === "sheets" || panelId === "search") setLeftTool(panelId);
    if (panelId === "ai" || panelId === "git") setActiveTool(panelId);
    if (region === "bottom") dispatchLayout({ type: "setRegionVisibility", regionId: "bottomPanel", visible: true });
  }, []);
  const handlePanelDrop = useCallback((region: DockRegion, panelId: string) => {
    handlePanelMove(panelId, region);
  }, [handlePanelMove]);
  const regionPanelId = useCallback((region: DockRegion): string | null => dockLayoutState.groups.find((group) => group.region === region)?.activePanelId ?? null, [dockLayoutState.groups]);
  const leftPanelId = regionPanelId("left");
  const rightPanelId = regionPanelId("right");
  const bottomPanelId = regionPanelId("bottom");
  const panelMoveTargets = useCallback((panelId: string, currentRegion: DockRegion): Array<{ id: string; label: string }> => {
    const definitions = dockLayoutState.placements.find((placement) => placement.panelId === panelId);
    if (!definitions) return [];
    return (["left", "right", "bottom"] as const).filter((region) => region !== currentRegion && (panelId === "tasks" || panelId === "gitChanges" || panelId === "diagnostics" ? region !== "left" : panelId === "ai" || panelId === "git" ? region !== "left" : true)).map((region) => ({ id: region, label: region[0]!.toUpperCase() + region.slice(1) }));
  }, [dockLayoutState.placements]);
  const handleQuickFind = useCallback(() => {
    setLeftTool("sheets");
    dispatchLayout({ type: "setRegionVisibility", regionId: "leftDock", visible: true });
    setQuickFindSignal((current) => current + 1);
  }, []);
  const detachPanel = useCallback(async (panel: DetachedPanel) => {
    if (detachedPanel === panel) return;
    try {
      const placement = dockLayoutState.placements.find((candidate) => candidate.panelId === panel);
      const originRegion = placement?.region === "left" || placement?.region === "right" || placement?.region === "bottom" ? placement.region : null;
      const label = `aeria-tool-${panel}`;
      const existing = await WebviewWindow.getByLabel(label);
      const detached = existing ?? new WebviewWindow(label, {
        title: `${panel === "ai" ? "AI" : panel === "git" ? "Git" : panel === "search" ? "Search" : "Tasks"} · Aeria`,
        url: `/?detached=${panel}`,
        width: panel === "tasks" || panel === "gitChanges" || panel === "diagnostics" ? 720 : 380,
        height: panel === "tasks" || panel === "gitChanges" || panel === "diagnostics" ? 280 : 620,
        decorations: false,
        resizable: true,
      });
      if (existing) {
        await detached.show();
        await detached.setFocus();
      }
      void detached.onCloseRequested(() => {
        setDetachedPanel((current) => current === panel ? null : current);
        if (originRegion) {
          setDockLayoutState((current) => reduceDockLayout(current, { type: "restore", panelId: panel, region: originRegion }));
        }
        if (originRegion === "bottom") {
          dispatchLayout({ type: "setRegionVisibility", regionId: "bottomPanel", visible: true });
        }
      });
      if (originRegion) {
        setDockLayoutState((current) => reduceDockLayout(current, { type: "float", panelId: panel }));
      }
      setDetachedPanel(panel);
      if (panel === "tasks" || panel === "gitChanges" || panel === "diagnostics") dispatchLayout({ type: "setRegionVisibility", regionId: "bottomPanel", visible: false });
    } catch (error) {
      showError("Could not detach tool", error);
    }
  }, [detachedPanel, dockLayoutState.placements, showError]);
  const documents: DocumentTab[] = documentTabs.tabs.map((document) => {
    const detail = document.id === documentTabs.activeId && loadedSheetName === document.sheetName ? `${rows.length.toLocaleString()} rows loaded` : null;
    return {
      id: document.id,
      label: document.label,
      ...(detail ? { detail } : {}),
      closable: true,
      pinned: document.pinned,
      preview: document.preview,
      dirty: document.dirty,
    };
  });

  return (
    <main className="app-shell editor-shell">
      <WindowChrome
        context={repositoryName(project.repositoryRoot)}
        projectName={repositoryName(project.repositoryRoot)}
        mode="workbench"
        onToggleDock={() => dispatchLayout({ type: "toggleRegion", regionId: "leftDock" })}
        onSelectTool={handleToolSelect}
        onQuickFind={handleQuickFind}
        onToggleBottom={() => dispatchLayout({ type: "setRegionVisibility", regionId: "bottomPanel", visible: !layout.regions.bottomPanel.visible })}
        onCloseProject={() => void handleClose()}
        onClose={() => void handleWindowClose()}
      />
      <div className="editor-notices" aria-live="polite">
        {applicationWarning ? (
          <ErrorBanner
            title="Project opened with a Recent projects warning"
            error={applicationWarning}
            onDismiss={onDismissApplicationWarning}
          />
        ) : null}
        {editorError ? <ErrorBanner title={editorError.title} error={editorError.error} onDismiss={() => setEditorError(null)} /> : null}
      </div>
      <div
        className="workbench-frame"
        style={{
          "--left-dock-width": leftDockOpen ? `${layout.regions.leftDock.size}px` : "0px",
          "--right-dock-width": layout.regions.rightDock.visible ? `${layout.regions.rightDock.size}px` : "0px",
          "--bottom-panel-height": layout.regions.bottomPanel.visible ? `${layout.regions.bottomPanel.size}px` : "0px",
        } as CSSProperties}
      >
        <ActivityRail side="left" items={[{ id: "sheets", label: "Sheets", icon: "folder", active: leftDockOpen && leftPanelId === "sheets", onSelect: () => { handlePanelMove("sheets", "left"); dispatchLayout({ type: "setRegionVisibility", regionId: "leftDock", visible: true }); } }, { id: "search", label: "Search", icon: "search", active: leftDockOpen && leftPanelId === "search", onSelect: () => handleToolSelect("search") }]} />
        <DockPanel panelId={leftPanelId ?? "sheets"} title={leftPanelId === "sheets" ? "Sheets" : "Search"} meta={leftPanelId === "sheets" ? project.sheets.length.toLocaleString() : "unavailable"} moveTargets={leftPanelId ? panelMoveTargets(leftPanelId, "left") : []} onMove={(region) => leftPanelId && handlePanelMove(leftPanelId, region)} canFloat={leftPanelId === "search"} onFloat={() => void detachPanel("search")} onDropPanel={(panelId) => handlePanelDrop("left", panelId)} className={leftDockOpen && detachedPanel !== leftPanelId ? "sheets-dock" : "sheets-dock is-hidden-dock"}>
          {leftPanelId === "sheets" ? <SheetSidebar sheets={project.sheets} selectedSheetName={selectedSheetName} disabled={closing} active={leftDockOpen && leftPanelId === "sheets"} quickFindSignal={quickFindSignal} onSelect={handleSheetSelect} /> : <WorkbenchToolDock activeTool="search" gitMode={gitMode} selectedBinding={selectedBinding} onGitModeChange={setGitMode} />}
        </DockPanel>
        <ResizeHandle axis="x" label="Resize sheets panel" onDelta={resizeLeftDock} />
        <div className="workbench-content">
          <div className="workbench-main-content">
            <DocumentTabs documents={documents} activeDocumentId={documentTabs.activeId} onSelect={(documentId) => void handleDocumentSelect(documentId)} onClose={(documentId) => void handleDocumentClose(documentId)} onPin={handleDocumentPin} onReorder={(documentId, beforeDocumentId) => setDocumentTabs((current) => reduceDocumentTabs(current, { type: "reorder", id: documentId, beforeId: beforeDocumentId }))} />
            {sheetHasNoRows ? (
              <div className="empty-document" role="status"><strong>No translatable rows</strong><p>This sheet does not contain any source String cells that can be translated.</p></div>
            ) : (
              <div className="sheet-document" style={{ "--translation-width": `${layout.regions.translation.size}px` } as CSSProperties}>
                <TranslationList rows={rows} selectedBinding={selectedBinding} selectedSheetName={selectedSheetName} loadedSheetName={loadedSheetName} disabled={closing} loading={sheetLoading} refreshing={false} loadingMore={loadingMore} hasMore={nextAfter !== null} onSelect={handleOccurrenceSelect} onLoadMore={handleLoadMoreClick} />
                <ResizeHandle axis="x" label="Resize translation editor" onDelta={resizeTranslation} />
                <TranslationEditor key={selectedRow ? rowKey(selectedRow) : "empty-editor"} row={selectedRow} selectedBinding={selectedBinding} mutations={mutations} onDirtyChange={handleDirtyChange} onSelectCell={handleFieldSelect} onSaveTarget={handleSaveTargetClick} onSaveNote={handleSaveNoteClick} onReviewChange={handleReviewChangeClick} />
              </div>
            )}
          </div>
          {layout.regions.bottomPanel.visible ? <><ResizeHandle axis="y" label="Resize bottom panel" onDelta={resizeBottomPanel} />{bottomPanelId === "tasks" || bottomPanelId === "gitChanges" || bottomPanelId === "diagnostics" ? <BottomPanel panelId={bottomPanelId} activeTab={bottomPanelId} onTabChange={(tab) => { dispatchLayout({ type: "setActiveTab", regionId: "bottomPanel", tabId: tab }); setDockLayoutState((current) => reduceDockLayout(current, { type: "activate", panelId: tab })); }} onCollapse={() => dispatchLayout({ type: "setRegionVisibility", regionId: "bottomPanel", visible: false })} onDetach={() => void detachPanel(bottomPanelId)} onDropPanel={(panelId) => handlePanelDrop("bottom", panelId)} /> : <div className="bottom-panel bottom-dock-content" onDragOver={(event) => event.preventDefault()} onDrop={(event) => { event.preventDefault(); const panelId = event.dataTransfer.getData("text/aeria-panel"); if (panelId) handlePanelDrop("bottom", panelId); }}>{bottomPanelId === "sheets" ? <SheetSidebar sheets={project.sheets} selectedSheetName={selectedSheetName} disabled={closing} active quickFindSignal={quickFindSignal} onSelect={handleSheetSelect} /> : <WorkbenchToolDock activeTool={bottomPanelId === "git" ? "git" : bottomPanelId === "search" ? "search" : "ai"} gitMode={gitMode} selectedBinding={selectedBinding} onGitModeChange={setGitMode} />}</div>}</> : null}
        </div>
        <ResizeHandle axis="x" label="Resize auxiliary dock" onDelta={resizeRightDock} />
          <DockPanel panelId={rightPanelId ?? activeTool} title={rightPanelId === "ai" ? "AI" : rightPanelId === "git" ? "Git" : rightPanelId === "sheets" ? "Sheets" : "Search"} meta={rightPanelId === "sheets" ? project.sheets.length.toLocaleString() : "unavailable"} moveTargets={rightPanelId ? panelMoveTargets(rightPanelId, "right") : []} onMove={(region) => rightPanelId && handlePanelMove(rightPanelId, region)} canFloat={rightPanelId === "ai" || rightPanelId === "git" || rightPanelId === "search"} onFloat={() => void detachPanel(rightPanelId === "search" ? "search" : rightPanelId === "git" ? "git" : "ai")} onDropPanel={(panelId) => handlePanelDrop("right", panelId)} className={layout.regions.rightDock.visible && detachedPanel !== rightPanelId ? "right-tool-dock" : "right-tool-dock is-hidden-dock"}>
            {rightPanelId === "sheets" ? <SheetSidebar sheets={project.sheets} selectedSheetName={selectedSheetName} disabled={closing} active={layout.regions.rightDock.visible} quickFindSignal={quickFindSignal} onSelect={handleSheetSelect} /> : <WorkbenchToolDock activeTool={rightPanelId === "search" ? "search" : rightPanelId === "git" ? "git" : "ai"} gitMode={gitMode} selectedBinding={selectedBinding} onGitModeChange={setGitMode} />}
          </DockPanel>
        <ActivityRail side="right" items={[{ id: "ai", label: "AI", icon: "sparkles", active: layout.regions.rightDock.visible && activeTool === "ai", onSelect: () => handleToolSelect("ai") }, { id: "git", label: "Git", icon: "branch", active: layout.regions.rightDock.visible && activeTool === "git", onSelect: () => handleToolSelect("git") }]} />
      </div>
      <StatusBar sheetName={selectedSheetName} rowCount={rows.length} loading={sheetLoading} repositoryRoot={project.repositoryRoot} sourceLanguage={project.sourceLanguage} sourceSnapshotId={project.sourceSnapshotId} selectedBinding={selectedBinding} dirty={dirty} />
      <ConfirmDialog
        open={discardRequest !== null}
        message={discardRequest?.message ?? ""}
        onKeepEditing={() => resolveDiscardConfirmation(false)}
        onDiscard={() => resolveDiscardConfirmation(true)}
      />
    </main>
  );
}
