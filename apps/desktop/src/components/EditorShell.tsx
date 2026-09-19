import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  closeProject,
  normalizeCommandError,
  pageTranslationEntries,
  setTranslationNote,
  setTranslationReviewState,
  setTranslationTarget,
} from "../ipc";
import { bindingKey } from "../binding";
import type {
  CommandError,
  ProjectSummaryDto,
  ReviewState,
  SourceBinding,
  TranslationEntryDto,
} from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { ProjectHeader } from "./ProjectHeader";
import { SheetSidebar } from "./SheetSidebar";
import { TranslationEditor } from "./TranslationEditor";
import { TranslationList } from "./TranslationList";

const PAGE_SIZE = 100;

type Mutation = "target" | "note" | "review" | null;

type EditorError = {
  title: string;
  error: CommandError;
};

type EditorShellProps = {
  project: ProjectSummaryDto;
  onClosed: () => void;
};

function draftsForEntry(entry: TranslationEntryDto | null): { target: string; note: string } {
  return {
    target: entry?.translation?.targetMacro ?? "",
    note: entry?.translation?.translatorNote ?? "",
  };
}

export function EditorShell({ project, onClosed }: EditorShellProps) {
  const firstSheetName = project.sheets[0]?.name ?? null;
  const [selectedSheetName, setSelectedSheetName] = useState<string | null>(firstSheetName);
  const [entries, setEntries] = useState<TranslationEntryDto[]>([]);
  const [nextAfter, setNextAfter] = useState<SourceBinding | null>(null);
  const [loadedPageCount, setLoadedPageCount] = useState(0);
  const [selectedBinding, setSelectedBinding] = useState<SourceBinding | null>(null);
  const [targetDraft, setTargetDraft] = useState("");
  const [noteDraft, setNoteDraft] = useState("");
  const [sheetLoading, setSheetLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [mutation, setMutation] = useState<Mutation>(null);
  const [closing, setClosing] = useState(false);
  const [editorError, setEditorError] = useState<EditorError | null>(null);
  const requestGeneration = useRef(0);

  const selectedEntry = useMemo(
    () =>
      selectedBinding
        ? entries.find((entry) => bindingKey(entry.sourceBinding) === bindingKey(selectedBinding)) ?? null
        : null,
    [entries, selectedBinding],
  );
  const backendTarget = selectedEntry?.translation?.targetMacro ?? "";
  const backendNote = selectedEntry?.translation?.translatorNote ?? "";
  const targetDirty = selectedEntry !== null && targetDraft !== backendTarget;
  const noteDirty = selectedEntry?.translation !== null && selectedEntry !== null && noteDraft !== backendNote;
  const hasDirtyDraft = targetDirty || noteDirty;

  const showError = useCallback((title: string, error: unknown) => {
    setEditorError({ title, error: normalizeCommandError(error) });
  }, []);

  const beginSheetLoad = useCallback(async (sheetName: string) => {
    const generation = ++requestGeneration.current;
    setSelectedSheetName(sheetName);
    setEntries([]);
    setNextAfter(null);
    setLoadedPageCount(0);
    setSelectedBinding(null);
    setTargetDraft("");
    setNoteDraft("");
    setSheetLoading(true);
    setEditorError(null);

    try {
      const page = await pageTranslationEntries(sheetName, null, PAGE_SIZE);
      if (generation !== requestGeneration.current) {
        return;
      }
      setEntries(page.entries);
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
    const bindingToRestore = selectedBinding;
    let after: SourceBinding | null = null;
    let fetchedPages = 0;
    const refreshedEntries: TranslationEntryDto[] = [];

    setRefreshing(true);
    setEditorError(null);

    try {
      for (let pageIndex = 0; pageIndex < pagesToLoad; pageIndex += 1) {
        const page = await pageTranslationEntries(selectedSheetName, after, PAGE_SIZE);
        if (generation !== requestGeneration.current) {
          return false;
        }
        refreshedEntries.push(...page.entries);
        fetchedPages += 1;
        after = page.nextAfter;
        if (!after) {
          break;
        }
      }

      setEntries(refreshedEntries);
      setNextAfter(after);
      setLoadedPageCount(fetchedPages);

      const restoredEntry = bindingToRestore
        ? refreshedEntries.find((entry) => bindingKey(entry.sourceBinding) === bindingKey(bindingToRestore)) ?? null
        : null;
      if (bindingToRestore && restoredEntry) {
        const drafts = draftsForEntry(restoredEntry);
        setTargetDraft(drafts.target);
        setNoteDraft(drafts.note);
      } else if (bindingToRestore) {
        setSelectedBinding(null);
        setTargetDraft("");
        setNoteDraft("");
      }
      return true;
    } catch (error) {
      if (generation === requestGeneration.current) {
        showError("Could not refresh entries", error);
      }
      return false;
    } finally {
      if (generation === requestGeneration.current) {
        setRefreshing(false);
      }
    }
  }, [loadedPageCount, selectedBinding, selectedSheetName, showError]);

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

  function handleEntrySelect(entry: TranslationEntryDto) {
    if (selectedBinding && bindingKey(selectedBinding) === bindingKey(entry.sourceBinding)) {
      return;
    }
    if (!confirmDiscardChanges("Changing entries")) {
      return;
    }
    setSelectedBinding(entry.sourceBinding);
    const drafts = draftsForEntry(entry);
    setTargetDraft(drafts.target);
    setNoteDraft(drafts.note);
    setEditorError(null);
  }

  async function handleLoadMore() {
    if (!selectedSheetName || !nextAfter || loadingMore || refreshing) {
      return;
    }

    const generation = requestGeneration.current;
    const after = nextAfter;
    setLoadingMore(true);

    try {
      const page = await pageTranslationEntries(selectedSheetName, after, PAGE_SIZE);
      if (generation !== requestGeneration.current) {
        return;
      }
      setEntries((current) => [...current, ...page.entries]);
      setNextAfter(page.nextAfter);
      setLoadedPageCount((current) => current + 1);
    } catch (error) {
      if (generation === requestGeneration.current) {
        showError("Could not load more entries", error);
      }
    } finally {
      setLoadingMore(false);
    }
  }

  function confirmMutationDiscard(shouldConfirm: boolean, message: string): boolean {
    if (!shouldConfirm) {
      return true;
    }
    return window.confirm(message);
  }

  async function handleSaveTarget() {
    if (!selectedEntry || !confirmMutationDiscard(noteDirty, "Saving the target will discard your unsaved note changes. Continue?")) {
      return;
    }
    setMutation("target");
    setEditorError(null);
    try {
      await setTranslationTarget(selectedEntry.sourceBinding, targetDraft);
      await reloadCurrentSheet();
    } catch (error) {
      showError("Could not save target", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleSaveNote() {
    const translation = selectedEntry?.translation;
    if (!translation || !confirmMutationDiscard(targetDirty, "Saving the note will discard your unsaved target changes. Continue?")) {
      return;
    }
    setMutation("note");
    setEditorError(null);
    try {
      await setTranslationNote(translation.translationUnitId, noteDraft.length === 0 ? null : noteDraft);
      await reloadCurrentSheet();
    } catch (error) {
      showError("Could not save note", error);
    } finally {
      setMutation(null);
    }
  }

  async function handleReviewChange(reviewState: ReviewState) {
    const translation = selectedEntry?.translation;
    if (!translation || translation.reviewState === reviewState || !confirmMutationDiscard(hasDirtyDraft, "Changing review state will discard unsaved draft changes. Continue?")) {
      return;
    }
    setMutation("review");
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
      {editorError ? <ErrorBanner title={editorError.title} error={editorError.error} onDismiss={() => setEditorError(null)} /> : null}
      <div className="editor-layout">
        <SheetSidebar
          sheets={project.sheets}
          selectedSheetName={selectedSheetName}
          disabled={sheetLoading || refreshing || mutation !== null || closing}
          onSelect={handleSheetSelect}
        />
        <TranslationList
          entries={entries}
          selectedBinding={selectedBinding}
          disabled={sheetLoading || refreshing || mutation !== null || closing}
          loading={sheetLoading}
          refreshing={refreshing}
          loadingMore={loadingMore}
          hasMore={nextAfter !== null}
          onSelect={handleEntrySelect}
          onLoadMore={() => void handleLoadMore()}
        />
        <TranslationEditor
          entry={selectedEntry}
          targetDraft={targetDraft}
          noteDraft={noteDraft}
          targetDirty={targetDirty}
          noteDirty={noteDirty}
          mutation={mutation}
          onTargetChange={setTargetDraft}
          onNoteChange={setNoteDraft}
          onSaveTarget={() => void handleSaveTarget()}
          onSaveNote={() => void handleSaveNote()}
          onReviewChange={(reviewState) => void handleReviewChange(reviewState)}
        />
      </div>
    </main>
  );
}
