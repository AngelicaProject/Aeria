export type SheetDocumentTab = {
  id: string;
  /** `commit` shows one commit of the project history. */
  kind: "sheet" | "commit";
  /** Empty for a commit. */
  sheetName: string;
  /** The commit id of a commit tab. */
  commitId?: string;
  label: string;
  /** Pinned tabs stay first and are not closed with the others. */
  pinned: boolean;
  /** A preview is replaced by the next sheet or commit opened. */
  preview: boolean;
  dirty: boolean;
};

export type DocumentTabsState = {
  tabs: SheetDocumentTab[];
  activeId: string | null;
};

export type DocumentTabsAction =
  /** `keep` opens the sheet in a tab of its own instead of the preview. */
  | { type: "openSheet"; sheetName: string; keep?: boolean }
  | { type: "openCommit"; commitId: string; label: string }
  | { type: "activate"; id: string }
  /** Turns a preview into a tab of its own. */
  | { type: "keep"; id: string }
  | { type: "setPinned"; id: string; pinned: boolean }
  | { type: "setDirty"; id: string; dirty: boolean }
  | { type: "close"; id: string }
  | { type: "reorder"; id: string; beforeId: string | null };

export const initialDocumentTabsState: DocumentTabsState = { tabs: [], activeId: null };

export function documentIdForSheet(sheetName: string): string {
  return `sheet:${sheetName}`;
}

export function documentIdForCommit(commitId: string): string {
  return `commit:${commitId}`;
}

/** Pinned tabs come first, each group in its own order. */
function pinnedFirst(tabs: readonly SheetDocumentTab[]): SheetDocumentTab[] {
  return [...tabs.filter((tab) => tab.pinned), ...tabs.filter((tab) => !tab.pinned)];
}

/** The tab that takes the place of a closed one in the tabs left: the next one, else the one before. */
function selectNeighbor(tabs: readonly SheetDocumentTab[], closedIndex: number): string | null {
  return tabs[closedIndex]?.id ?? tabs[closedIndex - 1]?.id ?? null;
}

export function reduceDocumentTabs(
  state: DocumentTabsState,
  action: DocumentTabsAction,
): DocumentTabsState {
  switch (action.type) {
    case "openSheet": {
      const id = documentIdForSheet(action.sheetName);
      const existing = state.tabs.find((tab) => tab.id === id);
      if (existing) {
        const tabs = action.keep && existing.preview
          ? state.tabs.map((tab) => tab.id === id ? { ...tab, preview: false } : tab)
          : state.tabs;
        return { tabs, activeId: id };
      }

      const tab: SheetDocumentTab = {
        id,
        kind: "sheet",
        sheetName: action.sheetName,
        label: action.sheetName.split("/").at(-1) ?? action.sheetName,
        pinned: false,
        preview: !action.keep,
        dirty: false,
      };
      const previewIndex = state.tabs.findIndex((candidate) => candidate.kind === "sheet" && candidate.preview && !candidate.pinned && !candidate.dirty);
      if (previewIndex >= 0 && !action.keep) {
        const tabs = [...state.tabs];
        tabs[previewIndex] = tab;
        return { tabs, activeId: id };
      }
      return { tabs: [...state.tabs, tab], activeId: id };
    }
    case "openCommit": {
      // Commit tabs behave like sheet previews: one unpinned commit tab is
      // reused, so browsing history does not pile up tabs.
      const id = documentIdForCommit(action.commitId);
      if (state.tabs.some((tab) => tab.id === id)) return { ...state, activeId: id };
      const tab: SheetDocumentTab = { id, kind: "commit", sheetName: "", commitId: action.commitId, label: action.label, pinned: false, preview: true, dirty: false };
      const previewIndex = state.tabs.findIndex((candidate) => candidate.kind === "commit" && candidate.preview && !candidate.pinned);
      if (previewIndex >= 0) {
        const tabs = [...state.tabs];
        tabs[previewIndex] = tab;
        return { tabs, activeId: id };
      }
      return { tabs: [...state.tabs, tab], activeId: id };
    }
    case "activate":
      return state.tabs.some((tab) => tab.id === action.id) ? { ...state, activeId: action.id } : state;
    case "keep":
      return {
        ...state,
        tabs: state.tabs.map((tab) => tab.id === action.id ? { ...tab, preview: false } : tab),
      };
    case "setPinned":
      return {
        ...state,
        tabs: pinnedFirst(state.tabs.map((tab) => tab.id === action.id ? { ...tab, pinned: action.pinned, preview: false } : tab)),
      };
    case "setDirty":
      return {
        ...state,
        tabs: state.tabs.map((tab) => tab.id === action.id
          ? { ...tab, dirty: action.dirty, preview: action.dirty ? false : tab.preview }
          : tab),
      };
    case "close": {
      const index = state.tabs.findIndex((tab) => tab.id === action.id);
      if (index < 0) return state;
      const tabs = state.tabs.filter((tab) => tab.id !== action.id);
      return { tabs, activeId: state.activeId === action.id ? selectNeighbor(tabs, index) : state.activeId };
    }
    case "reorder": {
      const fromIndex = state.tabs.findIndex((tab) => tab.id === action.id);
      if (fromIndex < 0) return state;
      const moving = state.tabs[fromIndex]!;
      const remaining = state.tabs.filter((tab) => tab.id !== action.id);
      const beforeIndex = action.beforeId === null ? remaining.length : remaining.findIndex((tab) => tab.id === action.beforeId);
      remaining.splice(beforeIndex < 0 ? remaining.length : beforeIndex, 0, moving);
      return { ...state, tabs: pinnedFirst(remaining) };
    }
  }
}

/**
 * Closes tabs and says what becomes of the open sheet: when the next active
 * tab is another sheet it opens (`open`); when the open sheet's tab closed
 * and no sheet tab is active, no sheet stays open (`clear`). A commit tab
 * leaves the sheet under it as it is.
 */
export function closeDocumentTabs(
  state: DocumentTabsState,
  ids: readonly string[],
  openSheet: string | null,
): { state: DocumentTabsState; open: string | null; clear: boolean } {
  const next = ids.reduce((current, id) => reduceDocumentTabs(current, { type: "close", id }), state);
  const active = next.tabs.find((tab) => tab.id === next.activeId);
  const closesSheet = openSheet !== null && ids.includes(documentIdForSheet(openSheet));
  if (active?.kind === "sheet") return { state: next, open: active.sheetName === openSheet ? null : active.sheetName, clear: false };
  return { state: next, open: null, clear: closesSheet };
}

export const openPreviewTab = (state: DocumentTabsState, sheetName: string): DocumentTabsState =>
  reduceDocumentTabs(state, { type: "openSheet", sheetName });
export const keepPreviewTab = (state: DocumentTabsState, id: string): DocumentTabsState =>
  reduceDocumentTabs(state, { type: "keep", id });
export const closeDocumentTab = (state: DocumentTabsState, id: string): DocumentTabsState =>
  reduceDocumentTabs(state, { type: "close", id });
