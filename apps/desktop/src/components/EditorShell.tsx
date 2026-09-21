import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { ProjectHeader } from "./ProjectHeader";
import { SheetSidebar } from "./SheetSidebar";
import { CellDraft, CellMutation, TranslationEditor } from "./TranslationEditor";
import { TranslationList } from "./TranslationList";

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

export function EditorShell({
  project,
  applicationWarning,
  onDismissApplicationWarning,
  onClosed,
}: EditorShellProps) {
  const firstSheetName = project.sheets[0]?.name ?? null;
  const [selectedSheetName, setSelectedSheetName] = useState<string | null>(firstSheetName);
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

  const beginSheetLoad = useCallback(async (sheetName: string) => {
    const generation = ++requestGeneration.current;
    setSelectedSheetName(sheetName);
    setRows([]);
    setNextAfter(null);
    setSelectedRowCursor(null);
    hasDirtyDraft.current = false;
    setSheetLoading(true);
    setEditorError(null);

    try {
      const page = await pageTranslationRows(sheetName, null, PAGE_SIZE);
      if (generation !== requestGeneration.current) {
        return;
      }
      setRows(page.rows);
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
      void beginSheetLoad(firstSheetName);
    }
  }, [beginSheetLoad, firstSheetName]);

  function confirmDiscardChanges(action: string): boolean {
    if (!hasDirtyDraft.current) {
      return true;
    }
    return window.confirm(`You have unsaved changes. ${action} will discard them. Continue?`);
  }

  function handleSheetSelect(sheetName: string) {
    if (sheetName === selectedSheetName || !confirmDiscardChanges("Changing sheets")) {
      return;
    }
    void beginSheetLoad(sheetName);
  }

  function handleRowSelect(row: TranslationRowDto) {
    const cursor = cursorForRow(row);
    if (selectedRowCursor && rowKey(selectedRowCursor) === rowKey(cursor)) {
      return;
    }
    if (!confirmDiscardChanges("Changing rows")) {
      return;
    }
    setSelectedRowCursor(cursor);
    hasDirtyDraft.current = false;
    setEditorError(null);
  }

  async function handleSaveTarget(cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) {
    const key = bindingKey(cell.sourceBinding);
    if (!selectedRow || !confirmMutationDiscard(otherDirty, "Saving the target will discard other unsaved changes. Continue?")) {
      return;
    }
    setMutation({ kind: "target", bindingKey: key });
    setEditorError(null);
    try {
      const overlay = await setTranslationTarget(cell.sourceBinding, draft.target);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not save target", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleSaveNote(cell: TranslationCellDto, draft: CellDraft, otherDirty: boolean) {
    const translation = cell.translation;
    const key = bindingKey(cell.sourceBinding);
    if (!translation || !selectedRow || !confirmMutationDiscard(otherDirty, "Saving the note will discard other unsaved changes. Continue?")) {
      return;
    }
    setMutation({ kind: "note", bindingKey: key });
    setEditorError(null);
    try {
      const overlay = await setTranslationNote(translation.translationUnitId, draft.note.length === 0 ? null : draft.note);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not save note", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleReviewChange(cell: TranslationCellDto, reviewState: ReviewState) {
    const translation = cell.translation;
    if (!translation || translation.reviewState === reviewState || !confirmMutationDiscard(hasDirtyDraft.current, "Changing review state will discard unsaved draft changes. Continue?")) {
      return;
    }
    const key = bindingKey(cell.sourceBinding);
    setMutation({ kind: "review", bindingKey: key });
    setEditorError(null);
    try {
      const overlay = await setTranslationReviewState(translation.translationUnitId, reviewState);
      applyOverlay(cell.sourceBinding, overlay);
    } catch (error) {
      showError("Could not change review state", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleLoadMore() {
    if (!selectedSheetName || !nextAfter || loadingMore) {
      return;
    }

    const generation = requestGeneration.current;
    const after = nextAfter;
    setLoadingMore(true);

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
  }

  function confirmMutationDiscard(shouldConfirm: boolean, message: string): boolean {
    return !shouldConfirm || window.confirm(message);
  }

  async function handleClose() {
    if (!confirmDiscardChanges("Closing the project")) {
      return;
    }
    setClosing(true);
    setEditorError(null);
    try {
      await closeProject();
      setClosing(false);
      onClosed();
    } catch (error) {
      setClosing(false);
      showError("Could not close project", error);
    }
  }

  return (
    <main className="app-shell editor-shell">
      <ProjectHeader
        project={project}
        closing={closing}
        disabled={closing || sheetLoading || mutation !== null}
        onClose={() => void handleClose()}
      />
      {applicationWarning ? (
        <ErrorBanner
          title="Project opened with a Recent projects warning"
          error={applicationWarning}
          onDismiss={onDismissApplicationWarning}
        />
      ) : null}
      {editorError ? <ErrorBanner title={editorError.title} error={editorError.error} onDismiss={() => setEditorError(null)} /> : null}
      <div className="editor-layout">
        <SheetSidebar
          sheets={project.sheets}
          selectedSheetName={selectedSheetName}
          disabled={sheetLoading || mutation !== null || closing}
          onSelect={handleSheetSelect}
        />
        <TranslationList
          rows={rows}
          selectedRow={selectedRowCursor}
          disabled={sheetLoading || mutation !== null || closing}
          loading={sheetLoading}
          refreshing={false}
          loadingMore={loadingMore}
          hasMore={nextAfter !== null}
          onSelect={handleRowSelect}
          onLoadMore={() => void handleLoadMore()}
        />
        <TranslationEditor
          key={selectedRow ? rowKey(selectedRow) : "empty-editor"}
          row={selectedRow}
          mutation={mutation}
          onDirtyChange={handleDirtyChange}
          onSaveTarget={(cell, draft, otherDirty) => void handleSaveTarget(cell, draft, otherDirty)}
          onSaveNote={(cell, draft, otherDirty) => void handleSaveNote(cell, draft, otherDirty)}
          onReviewChange={(cell, reviewState) => void handleReviewChange(cell, reviewState)}
        />
      </div>
    </main>
  );
}
