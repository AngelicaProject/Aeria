import { useCallback, useEffect, useMemo, useReducer, useRef, useState, type CSSProperties } from "react";
import { flushSync } from "react-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
  TranslationCellDto,
  TranslationOverlayDto,
  TranslationRowCursorDto,
  TranslationRowDto,
} from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { ConfirmDialog } from "./ConfirmDialog";
import { ActivityRail } from "./ActivityRail";
import { DocumentTabs } from "./DocumentTabs";
import { DockPanel } from "./DockPanel";
import { ResizeHandle } from "./ResizeHandle";
import { SheetSidebar } from "./SheetSidebar";
import { StatusBar } from "./StatusBar";
import { CellDraft, CellMutation, TranslationEditor } from "./TranslationEditor";
import { TranslationList } from "./TranslationList";
import { WindowChrome } from "./WindowChrome";
import { displayPathName } from "../pathDisplay";
import { initialWorkbenchLayout, reduceWorkbenchLayout } from "../ui/layout";

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
  const [sheetLoading, setSheetLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [mutations, setMutations] = useState<CellMutation[]>([]);
  const [closing, setClosing] = useState(false);
  const [editorError, setEditorError] = useState<EditorError | null>(null);
  const [discardRequest, setDiscardRequest] = useState<DiscardRequest | null>(null);
  const [layout, dispatchLayout] = useReducer(reduceWorkbenchLayout, initialWorkbenchLayout);
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
  }, []);

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
      setSelectedRowCursor(null);
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

  const handleSheetSelect = useCallback(async (sheetName: string) => {
    if (sheetName === selectedSheetName || !(await requestDiscardConfirmation("Changing sheets will discard your unsaved changes."))) {
      return;
    }
    void beginSheetLoad(sheetName);
  }, [beginSheetLoad, requestDiscardConfirmation, selectedSheetName]);

  const handleRowSelect = useCallback(async (row: TranslationRowDto) => {
    const cursor = cursorForRow(row);
    if (selectedRowCursor && rowKey(selectedRowCursor) === rowKey(cursor)) {
      return;
    }
    if (!(await requestDiscardConfirmation("Changing rows will discard your unsaved changes."))) {
      return;
    }
    flushSync(() => {
      setSelectedRowCursor(cursor);
      hasDirtyDraft.current = false;
      setEditorError(null);
    });
  }, [requestDiscardConfirmation, selectedRowCursor]);

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
      setNextAfter(page.nextAfter);
    } catch (error) {
      if (generation === requestGeneration.current) {
        showError("Could not load more rows", error);
      }
    } finally {
      setLoadingMore(false);
    }
  }, [loadingMore, nextAfter, selectedSheetName, showError]);

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

  return (
    <main className="app-shell editor-shell">
      <WindowChrome
        context={repositoryName(project.repositoryRoot)}
        projectName={repositoryName(project.repositoryRoot)}
        mode="workbench"
        onToggleDock={() => dispatchLayout({ type: "toggleRegion", regionId: "leftDock" })}
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
        className={leftDockOpen ? "workbench-frame" : "workbench-frame dock-closed"}
        style={{ "--left-dock-width": `${layout.regions.leftDock.size}px` } as CSSProperties}
      >
        <ActivityRail side="left" items={[{ id: "sheets", label: "Sheets", icon: "folder", active: leftDockOpen, onSelect: () => dispatchLayout({ type: "toggleRegion", regionId: "leftDock" }) }]} />
        {leftDockOpen ? (
          <>
            <DockPanel title="Sheets" meta={project.sheets.length.toLocaleString()} className="sheets-dock">
              <SheetSidebar sheets={project.sheets} selectedSheetName={selectedSheetName} disabled={closing} onSelect={handleSheetSelect} />
            </DockPanel>
            <ResizeHandle axis="x" label="Resize sheets panel" onDelta={resizeLeftDock} />
          </>
        ) : null}
        <div className="workbench-content">
          <DocumentTabs label={selectedSheetName ?? "Sheet"} detail={loadedSheetName ? `${rows.length.toLocaleString()} loaded` : "loading"} />
          {sheetHasNoRows ? (
            <div className="empty-document" role="status">
              <strong>No translatable rows</strong>
              <p>This sheet does not contain any source String cells that can be translated.</p>
            </div>
          ) : (
            <div className="sheet-document" style={{ "--translation-width": `${layout.regions.translation.size}px` } as CSSProperties}>
              <TranslationList
                rows={rows}
                selectedRow={selectedRowCursor}
                selectedSheetName={selectedSheetName}
                loadedSheetName={loadedSheetName}
                disabled={closing}
                loading={sheetLoading}
                refreshing={false}
                loadingMore={loadingMore}
                hasMore={nextAfter !== null}
                onSelect={handleRowSelect}
                onLoadMore={handleLoadMoreClick}
              />
              <ResizeHandle axis="x" label="Resize translation editor" onDelta={resizeTranslation} />
              <TranslationEditor
                key={selectedRow ? rowKey(selectedRow) : "empty-editor"}
                row={selectedRow}
                mutations={mutations}
                onDirtyChange={handleDirtyChange}
                onSaveTarget={handleSaveTargetClick}
                onSaveNote={handleSaveNoteClick}
                onReviewChange={handleReviewChangeClick}
              />
            </div>
          )}
          <StatusBar
            sheetName={selectedSheetName}
            rowCount={rows.length}
            loading={sheetLoading}
            repositoryRoot={project.repositoryRoot}
            sourceLanguage={project.sourceLanguage}
          />
        </div>
      </div>
      <ConfirmDialog
        open={discardRequest !== null}
        message={discardRequest?.message ?? ""}
        onKeepEditing={() => resolveDiscardConfirmation(false)}
        onDiscard={() => resolveDiscardConfirmation(true)}
      />
    </main>
  );
}
