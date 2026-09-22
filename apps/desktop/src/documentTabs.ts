export type SheetDocumentTab = {
  id: string;
  sheetName: string;
  label: string;
  pinned: boolean;
  preview: boolean;
  dirty: boolean;
};

export type DocumentTabsState = {
  tabs: SheetDocumentTab[];
  activeId: string | null;
};

export type DocumentTabsAction =
  | { type: "openSheet"; sheetName: string; pin?: boolean }
  | { type: "activate"; id: string }
  | { type: "pin"; id: string }
  | { type: "setDirty"; id: string; dirty: boolean }
  | { type: "close"; id: string }
  | { type: "reorder"; id: string; beforeId: string | null };

export const initialDocumentTabsState: DocumentTabsState = { tabs: [], activeId: null };

export function documentIdForSheet(sheetName: string): string {
  return `sheet:${sheetName}`;
}

function selectNeighbor(tabs: readonly SheetDocumentTab[], closedIndex: number): string | null {
  return tabs[closedIndex + 1]?.id ?? tabs[closedIndex - 1]?.id ?? null;
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
        const tabs = action.pin && !existing.pinned
          ? state.tabs.map((tab) => tab.id === id ? { ...tab, pinned: true, preview: false } : tab)
          : state.tabs;
        return { tabs, activeId: id };
      }

      const tab: SheetDocumentTab = {
        id,
        sheetName: action.sheetName,
        label: action.sheetName.split("/").at(-1) ?? action.sheetName,
        pinned: Boolean(action.pin),
        preview: !action.pin,
        dirty: false,
      };
      const previewIndex = state.tabs.findIndex((candidate) => candidate.preview && !candidate.pinned && !candidate.dirty);
      if (previewIndex >= 0 && !action.pin) {
        const tabs = [...state.tabs];
        tabs[previewIndex] = tab;
        return { tabs, activeId: id };
      }
      return { tabs: [...state.tabs, tab], activeId: id };
    }
    case "activate":
      return state.tabs.some((tab) => tab.id === action.id) ? { ...state, activeId: action.id } : state;
    case "pin":
      return {
        ...state,
        tabs: state.tabs.map((tab) => tab.id === action.id ? { ...tab, pinned: true, preview: false } : tab),
      };
    case "setDirty":
      return {
        ...state,
        tabs: state.tabs.map((tab) => tab.id === action.id
          ? { ...tab, dirty: action.dirty, pinned: action.dirty ? true : tab.pinned, preview: action.dirty ? false : tab.preview }
          : tab),
      };
    case "close": {
      const index = state.tabs.findIndex((tab) => tab.id === action.id);
      if (index < 0) return state;
      const tabs = state.tabs.filter((tab) => tab.id !== action.id);
      return { tabs, activeId: state.activeId === action.id ? selectNeighbor(tabs, Math.min(index, tabs.length - 1)) : state.activeId };
    }
    case "reorder": {
      const fromIndex = state.tabs.findIndex((tab) => tab.id === action.id);
      if (fromIndex < 0) return state;
      const moving = state.tabs[fromIndex]!;
      const remaining = state.tabs.filter((tab) => tab.id !== action.id);
      const beforeIndex = action.beforeId === null ? remaining.length : remaining.findIndex((tab) => tab.id === action.beforeId);
      remaining.splice(beforeIndex < 0 ? remaining.length : beforeIndex, 0, moving);
      return { ...state, tabs: remaining };
    }
  }
}

export const openPreviewTab = (state: DocumentTabsState, sheetName: string): DocumentTabsState =>
  reduceDocumentTabs(state, { type: "openSheet", sheetName });
export const pinPreviewTab = (state: DocumentTabsState, id: string): DocumentTabsState =>
  reduceDocumentTabs(state, { type: "pin", id });
export const closeDocumentTab = (state: DocumentTabsState, id: string): DocumentTabsState =>
  reduceDocumentTabs(state, { type: "close", id });
