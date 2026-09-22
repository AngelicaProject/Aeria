import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { ActivityRail } from "./ActivityRail";
import { ProjectHeader } from "./ProjectHeader";
import { SheetSidebar } from "./SheetSidebar";
import { StatusBar } from "./StatusBar";
import { CellDraft, CellMutation, TranslationEditor } from "./TranslationEditor";
import { TranslationList } from "./TranslationList";
import { WindowChrome } from "./WindowChrome";

const PAGE_SIZE = 100;

type EditorError = {
  title: string;
  error: CommandError;
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
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
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
  const [mutation, setMutation] = useState<CellMutation | null>(null);
  const [closing, setClosing] = useState(false);
  const [editorError, setEditorError] = useState<EditorError | null>(null);
  const requestGeneration = useRef(0);
  const hasDirtyDraft = useRef(false);

  const selectedRow = useMemo(
    () => selectedRowCursor ? rows.find((row) => rowKey(row) === rowKey(selectedRowCursor)) ?? null : null,
    [rows, selectedRowCursor],
  );

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

  const confirmDiscardChanges = useCallback((action: string): boolean => {
    if (!hasDirtyDraft.current) {
      return true;
    }
    return window.confirm(`You have unsaved changes. ${action} will discard them. Continue?`);
  }, []);

  const handleSheetSelect = useCallback((sheetName: string) => {
    if (sheetName === selectedSheetName || !confirmDiscardChanges("Changing sheets")) {
      return;
    }
    void beginSheetLoad(sheetName);
  }, [beginSheetLoad, confirmDiscardChanges, selectedSheetName]);

  const handleRowSelect = useCallback((row: TranslationRowDto) => {
    const cursor = cursorForRow(row);
    if (selectedRowCursor && rowKey(selectedRowCursor) === rowKey(cursor)) {
      return;
    }
    if (!confirmDiscardChanges("Changing rows")) {
      return;
    }
    flushSync(() => {
      setSelectedRowCursor(cursor);
      hasDirtyDraft.current = false;
      setEditorError(null);
    });
  }, [confirmDiscardChanges, selectedRowCursor]);

  const confirmMutationDiscard = useCallback((shouldConfirm: boolean, message: string): boolean => {
    return !shouldConfirm || window.confirm(message);
  }, []);

  const handleSaveTarget = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => {
    const key = bindingKey(cell.sourceBinding);
    if (!selectedRow || !confirmMutationDiscard(otherDirty, "Saving the target will discard other unsaved changes. Continue?")) {
      return;
    }
    flushSync(() => {
      setMutation({ kind: "target", bindingKey: key });
      setEditorError(null);
    });
    try {
      const overlay = await setTranslationTarget(cell.sourceBinding, draft.target);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not save target", error);
    } finally {
      setMutation(null);
    }
  }, [applyOverlay, confirmMutationDiscard, selectedRow, showError]);

  const handleSaveNote = useCallback(async (cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => {
    const translation = cell.translation;
    const key = bindingKey(cell.sourceBinding);
    if (!translation || !selectedRow || !confirmMutationDiscard(otherDirty, "Saving the note will discard other unsaved changes. Continue?")) {
      return;
    }
    flushSync(() => {
      setMutation({ kind: "note", bindingKey: key });
      setEditorError(null);
    });
    try {
      const overlay = await setTranslationNote(translation.translationUnitId, draft.note.length === 0 ? null : draft.note);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not save note", error);
    } finally {
      setMutation(null);
    }
  }, [applyOverlay, confirmMutationDiscard, selectedRow, showError]);

  const handleReviewChange = useCallback(async (cell: TranslationCellDto, reviewState: ReviewState) => {
    const translation = cell.translation;
    if (!translation || translation.reviewState === reviewState || !confirmMutationDiscard(hasDirtyDraft.current, "Changing review state will discard unsaved draft changes. Continue?")) {
      return;
    }
    const key = bindingKey(cell.sourceBinding);
    flushSync(() => {
      setMutation({ kind: "review", bindingKey: key });
      setEditorError(null);
    });
    try {
      const overlay = await setTranslationReviewState(translation.translationUnitId, reviewState);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not change review state", error);
    } finally {
      setMutation(null);
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
    if (!confirmDiscardChanges("Closing the project")) {
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
  }, [confirmDiscardChanges, onClosed, showError]);

  const handleCloseClick = useCallback(() => {
    void handleClose();
  }, [handleClose]);

  const handleWindowClose = useCallback(() => {
    if (!confirmDiscardChanges("Closing Aeria")) {
      return;
    }
    void getCurrentWindow().close().catch(() => undefined);
  }, [confirmDiscardChanges]);
  const handleLoadMoreClick = useCallback(() => {
    void handleLoadMore();
  }, [handleLoadMore]);
  const handleSaveTargetClick = useCallback((cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => {
    void handleSaveTarget(cell, draft, otherDirty);
  }, [handleSaveTarget]);
  const handleSaveNoteClick = useCallback((cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) => {
    void handleSaveNote(cell, draft, otherDirty);
  }, [handleSaveNote]);
  const handleReviewChangeClick = useCallback((cell: TranslationCellDto, reviewState: ReviewState) => {
    void handleReviewChange(cell, reviewState);
  }, [handleReviewChange]);

  return (
    <main className="app-shell editor-shell">
      <WindowChrome
        context={repositoryName(project.repositoryRoot)}
        detail={`${project.sourceLanguage} → ${project.targetLanguage}`}
        mode="workbench"
        onClose={handleWindowClose}
      />
      <div className="editor-notices">
        <ProjectHeader
          project={project}
          closing={closing}
          disabled={closing}
          onClose={handleCloseClick}
        />
        {applicationWarning ? (
          <ErrorBanner
            title="Project opened with a Recent projects warning"
            error={applicationWarning}
            onDismiss={onDismissApplicationWarning}
          />
        ) : null}
        {editorError ? <ErrorBanner title={editorError.title} error={editorError.error} onDismiss={() => setEditorError(null)} /> : null}
      </div>
      <div className="workbench-frame">
        <ActivityRail active="sheets" />
        <div className="workbench-content">
          <div className="editor-layout">
            <SheetSidebar
              sheets={project.sheets}
              selectedSheetName={selectedSheetName}
              disabled={closing}
              onSelect={handleSheetSelect}
            />
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
            <TranslationEditor
              key={selectedRow ? rowKey(selectedRow) : "empty-editor"}
              row={selectedRow}
              mutation={mutation}
              onDirtyChange={handleDirtyChange}
              onSaveTarget={handleSaveTargetClick}
              onSaveNote={handleSaveNoteClick}
              onReviewChange={handleReviewChangeClick}
            />
          </div>
          <StatusBar
            sheetName={selectedSheetName}
            rowCount={rows.length}
            loading={sheetLoading}
            repositoryRoot={project.repositoryRoot}
            sourceLanguage={project.sourceLanguage}
            targetLanguage={project.targetLanguage}
          />
        </div>
      </div>
    </main>
  );
}
