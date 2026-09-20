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

function draftsForRow(row: TranslationRowDto | null): Record<string, CellDraft> {
  if (!row) {
    return {};
  }
  return Object.fromEntries(
    row.cells.map((cell) => [
      bindingKey(cell.sourceBinding),
      {
        target: cell.translation?.targetMacro ?? "",
        note: cell.translation?.translatorNote ?? "",
      },
    ]),
  );
}

function draftForCell(cell: TranslationCellDto, drafts: Record<string, CellDraft>): CellDraft {
  return drafts[bindingKey(cell.sourceBinding)] ?? {
    target: cell.translation?.targetMacro ?? "",
    note: cell.translation?.translatorNote ?? "",
  };
}

function cellIsDirty(cell: TranslationCellDto, draft: CellDraft): boolean {
  return draft.target !== (cell.translation?.targetMacro ?? "") ||
    (cell.translation !== null && draft.note !== (cell.translation.translatorNote ?? ""));
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
  const [loadedPageCount, setLoadedPageCount] = useState(0);
  const [selectedRowCursor, setSelectedRowCursor] = useState<TranslationRowCursorDto | null>(null);
  const [drafts, setDrafts] = useState<Record<string, CellDraft>>({});
  const [sheetLoading, setSheetLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [mutation, setMutation] = useState<CellMutation | null>(null);
  const [closing, setClosing] = useState(false);
  const [editorError, setEditorError] = useState<EditorError | null>(null);
  const requestGeneration = useRef(0);

  const selectedRow = useMemo(
    () => selectedRowCursor ? rows.find((row) => rowKey(row) === rowKey(selectedRowCursor)) ?? null : null,
    [rows, selectedRowCursor],
  );

  const hasDirtyDraft = selectedRow?.cells.some((cell) => cellIsDirty(cell, draftForCell(cell, drafts))) ?? false;

  const showError = useCallback((title: string, error: unknown) => {
    setEditorError({ title, error: normalizeCommandError(error) });
  }, []);

  const beginSheetLoad = useCallback(async (sheetName: string) => {
    const generation = ++requestGeneration.current;
    setSelectedSheetName(sheetName);
    setRows([]);
    setNextAfter(null);
    setLoadedPageCount(0);
    setSelectedRowCursor(null);
    setDrafts({});
    setSheetLoading(true);
    setEditorError(null);

    try {
      const page = await pageTranslationRows(sheetName, null, PAGE_SIZE);
      if (generation !== requestGeneration.current) {
        return;
      }
      setRows(page.rows);
      setNextAfter(page.nextAfter);
      setLoadedPageCount(1);
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

  const reloadCurrentSheet = useCallback(async (): Promise<boolean> => {
    if (!selectedSheetName) {
      return true;
    }

    const generation = ++requestGeneration.current;
    const pagesToLoad = Math.max(loadedPageCount, 1);
    const rowToRestore = selectedRowCursor;
    let after: TranslationRowCursorDto | null = null;
    let fetchedPages = 0;
    const refreshedRows: TranslationRowDto[] = [];

    setRefreshing(true);
    setEditorError(null);

    try {
      for (let pageIndex = 0; pageIndex < pagesToLoad; pageIndex += 1) {
        const page = await pageTranslationRows(selectedSheetName, after, PAGE_SIZE);
        if (generation !== requestGeneration.current) {
          return false;
        }
        for (const row of page.rows) {
          if (!refreshedRows.some((existing) => rowKey(existing) === rowKey(row))) {
            refreshedRows.push(row);
          }
        }
        fetchedPages += 1;
        after = page.nextAfter;
        if (!after) {
          break;
        }
      }

      setRows(refreshedRows);
      setNextAfter(after);
      setLoadedPageCount(fetchedPages);

      const restoredRow = rowToRestore
        ? refreshedRows.find((row) => rowKey(row) === rowKey(rowToRestore)) ?? null
        : null;
      if (restoredRow) {
        setSelectedRowCursor(cursorForRow(restoredRow));
        setDrafts(draftsForRow(restoredRow));
      } else if (rowToRestore) {
        setSelectedRowCursor(null);
        setDrafts({});
      }
      return true;
    } catch (error) {
      if (generation === requestGeneration.current) {
        showError("Could not refresh rows", error);
      }
      return false;
    } finally {
      if (generation === requestGeneration.current) {
        setRefreshing(false);
      }
    }
  }, [loadedPageCount, selectedRowCursor, selectedSheetName, showError]);

  function confirmDiscardChanges(action: string): boolean {
    if (!hasDirtyDraft) {
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
    setDrafts(draftsForRow(row));
    setEditorError(null);
  }

  function updateDraft(cell: TranslationCellDto, field: keyof CellDraft, value: string) {
    const key = bindingKey(cell.sourceBinding);
    const current = draftForCell(cell, drafts);
    setDrafts((existing) => ({ ...existing, [key]: { ...current, ...existing[key], [field]: value } }));
  }

  function hasOtherDirtyDraft(cell: TranslationCellDto, includeCellNote: boolean): boolean {
    if (!selectedRow) {
      return false;
    }
    const targetKey = bindingKey(cell.sourceBinding);
    return selectedRow.cells.some((candidate) => {
      const key = bindingKey(candidate.sourceBinding);
      const draft = draftForCell(candidate, drafts);
      if (key !== targetKey) {
        return cellIsDirty(candidate, draft);
      }
      if (includeCellNote) {
        return candidate.translation !== null && draft.note !== (candidate.translation.translatorNote ?? "");
      }
      return draft.target !== (candidate.translation?.targetMacro ?? "");
    });
  }

  async function handleSaveTarget(cell: TranslationCellDto) {
    const key = bindingKey(cell.sourceBinding);
    const draft = draftForCell(cell, drafts);
    if (!selectedRow || !confirmMutationDiscard(hasOtherDirtyDraft(cell, true), "Saving the target will discard other unsaved changes. Continue?")) {
      return;
    }
    setMutation({ kind: "target", bindingKey: key });
    setEditorError(null);
    try {
      await setTranslationTarget(cell.sourceBinding, draft.target);
      await reloadCurrentSheet();
    } catch (error) {
      showError("Could not save target", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleSaveNote(cell: TranslationCellDto) {
    const translation = cell.translation;
    const key = bindingKey(cell.sourceBinding);
    const draft = draftForCell(cell, drafts);
    if (!translation || !selectedRow || !confirmMutationDiscard(hasOtherDirtyDraft(cell, false), "Saving the note will discard other unsaved changes. Continue?")) {
      return;
    }
    setMutation({ kind: "note", bindingKey: key });
    setEditorError(null);
    try {
      await setTranslationNote(translation.translationUnitId, draft.note.length === 0 ? null : draft.note);
      await reloadCurrentSheet();
    } catch (error) {
      showError("Could not save note", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleReviewChange(cell: TranslationCellDto, reviewState: ReviewState) {
    const translation = cell.translation;
    if (!translation || translation.reviewState === reviewState || !confirmMutationDiscard(hasDirtyDraft, "Changing review state will discard unsaved draft changes. Continue?")) {
      return;
    }
    const key = bindingKey(cell.sourceBinding);
    setMutation({ kind: "review", bindingKey: key });
    setEditorError(null);
    try {
      await setTranslationReviewState(translation.translationUnitId, reviewState);
      await reloadCurrentSheet();
    } catch (error) {
      showError("Could not change review state", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleLoadMore() {
    if (!selectedSheetName || !nextAfter || loadingMore || refreshing) {
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
      setLoadedPageCount((current) => current + 1);
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
        disabled={closing || sheetLoading || refreshing || mutation !== null}
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
          disabled={sheetLoading || refreshing || mutation !== null || closing}
          onSelect={handleSheetSelect}
        />
        <TranslationList
          rows={rows}
          selectedRow={selectedRowCursor}
          disabled={sheetLoading || refreshing || mutation !== null || closing}
          loading={sheetLoading}
          refreshing={refreshing}
          loadingMore={loadingMore}
          hasMore={nextAfter !== null}
          onSelect={handleRowSelect}
          onLoadMore={() => void handleLoadMore()}
        />
        <TranslationEditor
          row={selectedRow}
          drafts={drafts}
          mutation={mutation}
          onTargetChange={(cell, value) => updateDraft(cell, "target", value)}
          onNoteChange={(cell, value) => updateDraft(cell, "note", value)}
          onSaveTarget={(cell) => void handleSaveTarget(cell)}
          onSaveNote={(cell) => void handleSaveNote(cell)}
          onReviewChange={(cell, reviewState) => void handleReviewChange(cell, reviewState)}
        />
      </div>
    </main>
  );
}
