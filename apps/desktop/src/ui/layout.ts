export type LayoutRegion = {
  id: string;
  panelIds: readonly string[];
  documentIds: readonly string[];
  activeTabId: string | null;
  visible: boolean;
  size: number;
  minSize: number;
  maxSize: number;
  resizable: boolean;
};

export type WorkbenchLayoutState = {
  regions: Readonly<WorkbenchRegions>;
  activeDocumentId: string | null;
};

export type WorkbenchRegions = {
  leftDock: LayoutRegion;
  /** Height of the translation editor below the string list. */
  editor: LayoutRegion;
  rightDock: LayoutRegion;
  bottomPanel: LayoutRegion;
  [regionId: string]: LayoutRegion;
};

export type WorkbenchLayoutAction =
  | { type: "toggleRegion"; regionId: string }
  | { type: "setRegionVisibility"; regionId: string; visible: boolean }
  | { type: "setActiveTab"; regionId: string; tabId: string | null }
  | { type: "setActiveDocument"; documentId: string | null }
  | { type: "resizeRegion"; regionId: string; delta: number };

function updateRegion(
  state: WorkbenchLayoutState,
  regionId: string,
  update: (region: LayoutRegion) => LayoutRegion,
): WorkbenchLayoutState {
  const region = state.regions[regionId];
  if (!region) return state;
  return {
    ...state,
    regions: { ...state.regions, [regionId]: update(region) },
  };
}

export const initialWorkbenchLayout: WorkbenchLayoutState = {
  activeDocumentId: "sheet",
  regions: {
    leftDock: {
      id: "leftDock",
      panelIds: ["sheets"],
      documentIds: [],
      activeTabId: "sheets",
      visible: true,
      size: 260,
      minSize: 190,
      maxSize: 420,
      resizable: true,
    },
    editor: {
      id: "editor",
      panelIds: [],
      documentIds: ["sheet"],
      activeTabId: "sheet",
      visible: true,
      size: 320,
      minSize: 220,
      maxSize: 680,
      resizable: true,
    },
    rightDock: {
      id: "rightDock",
      panelIds: ["ai", "git"],
      documentIds: [],
      activeTabId: "ai",
      visible: true,
      size: 340,
      minSize: 260,
      maxSize: 560,
      resizable: true,
    },
    bottomPanel: {
      id: "bottomPanel",
      panelIds: ["tasks", "gitChanges", "diagnostics"],
      documentIds: [],
      activeTabId: "tasks",
      visible: false,
      size: 160,
      minSize: 100,
      maxSize: 300,
      resizable: true,
    },
  },
};

export function reduceWorkbenchLayout(
  state: WorkbenchLayoutState,
  action: WorkbenchLayoutAction,
): WorkbenchLayoutState {
  switch (action.type) {
    case "toggleRegion":
      return updateRegion(state, action.regionId, (region) => ({ ...region, visible: !region.visible }));
    case "setRegionVisibility":
      return updateRegion(state, action.regionId, (region) => ({ ...region, visible: action.visible }));
    case "setActiveTab":
      return updateRegion(state, action.regionId, (region) => ({ ...region, activeTabId: action.tabId }));
    case "setActiveDocument":
      return { ...state, activeDocumentId: action.documentId };
    case "resizeRegion":
      return updateRegion(state, action.regionId, (region) => {
        if (!region.resizable) return region;
        return {
          ...region,
          size: Math.max(region.minSize, Math.min(region.maxSize, region.size + action.delta)),
        };
      });
  }
}
