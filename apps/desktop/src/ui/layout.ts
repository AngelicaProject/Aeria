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
  translation: LayoutRegion;
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
      size: 238,
      minSize: 190,
      maxSize: 360,
      resizable: true,
    },
    translation: {
      id: "translation",
      panelIds: [],
      documentIds: ["sheet"],
      activeTabId: "sheet",
      visible: true,
      size: 390,
      minSize: 300,
      maxSize: 620,
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
