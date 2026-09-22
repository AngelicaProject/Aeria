export type DockRegion = "left" | "right" | "bottom";

export type DockPanelDefinition = {
  id: string;
  allowedRegions: readonly DockRegion[];
  singleton?: boolean;
  floatable?: boolean;
};

export type DockGroup = {
  id: string;
  region: DockRegion;
  panelIds: string[];
  activePanelId: string | null;
};

export type DockPanelPlacement = {
  panelId: string;
  region: DockRegion | "floating";
  groupId: string | null;
  visible: boolean;
};

export type DockLayoutState = {
  groups: DockGroup[];
  placements: DockPanelPlacement[];
};

export type DockLayoutAction =
  | { type: "move"; panelId: string; region: DockRegion }
  | { type: "reorder"; panelId: string; beforePanelId: string | null }
  | { type: "activate"; panelId: string }
  | { type: "setVisible"; panelId: string; visible: boolean }
  | { type: "float"; panelId: string }
  | { type: "restore"; panelId: string; region: DockRegion };

export const dockPanelDefinitions: readonly DockPanelDefinition[] = [
  { id: "sheets", allowedRegions: ["left", "right", "bottom"], singleton: true },
  { id: "search", allowedRegions: ["left", "right", "bottom"], singleton: true, floatable: true },
  { id: "ai", allowedRegions: ["right", "bottom"], singleton: true, floatable: true },
  { id: "git", allowedRegions: ["right", "bottom"], singleton: true, floatable: true },
  { id: "tasks", allowedRegions: ["bottom", "right"], singleton: true, floatable: true },
  { id: "gitChanges", allowedRegions: ["bottom", "right"], singleton: true, floatable: true },
  { id: "diagnostics", allowedRegions: ["bottom", "right"], singleton: true, floatable: true },
];

export const initialDockLayout: DockLayoutState = {
  groups: [
    { id: "left-main", region: "left", panelIds: ["sheets", "search"], activePanelId: "sheets" },
    { id: "right-main", region: "right", panelIds: ["ai", "git"], activePanelId: "ai" },
    { id: "bottom-main", region: "bottom", panelIds: ["tasks", "gitChanges", "diagnostics"], activePanelId: "tasks" },
  ],
  placements: dockPanelDefinitions.map((definition) => ({
    panelId: definition.id,
    region: definition.id === "sheets" || definition.id === "search" ? "left" : definition.id === "tasks" || definition.id === "gitChanges" || definition.id === "diagnostics" ? "bottom" : "right",
    groupId: definition.id === "sheets" || definition.id === "search" ? "left-main" : definition.id === "tasks" || definition.id === "gitChanges" || definition.id === "diagnostics" ? "bottom-main" : "right-main",
    visible: true,
  })),
};

function definitionFor(panelId: string): DockPanelDefinition | undefined {
  return dockPanelDefinitions.find((definition) => definition.id === panelId);
}

function removePanel(state: DockLayoutState, panelId: string): DockLayoutState {
  return {
    groups: state.groups.map((group) => ({
      ...group,
      panelIds: group.panelIds.filter((id) => id !== panelId),
      activePanelId: group.activePanelId === panelId ? group.panelIds.find((id) => id !== panelId) ?? null : group.activePanelId,
    })),
    placements: state.placements.map((placement) => placement.panelId === panelId ? { ...placement, groupId: null } : placement),
  };
}

export function reduceDockLayout(state: DockLayoutState, action: DockLayoutAction): DockLayoutState {
  const placement = state.placements.find((candidate) => candidate.panelId === action.panelId);
  const definition = definitionFor(action.panelId);
  if (!placement || !definition) return state;

  if (action.type === "move" || action.type === "restore") {
    if (!definition.allowedRegions.includes(action.region)) return state;
    const removed = removePanel(state, action.panelId);
    const group = removed.groups.find((candidate) => candidate.region === action.region) ?? {
      id: `${action.region}-main`, region: action.region, panelIds: [], activePanelId: null,
    };
    const groups = removed.groups.some((candidate) => candidate.id === group.id)
      ? removed.groups.map((candidate) => candidate.id === group.id
        ? { ...candidate, panelIds: [...candidate.panelIds, action.panelId], activePanelId: action.panelId }
        : candidate)
      : [...removed.groups, { ...group, panelIds: [action.panelId], activePanelId: action.panelId }];
    return {
      groups,
      placements: removed.placements.map((candidate) => candidate.panelId === action.panelId
        ? { ...candidate, region: action.region, groupId: group.id, visible: true }
        : candidate),
    };
  }

  if (action.type === "float") {
    if (!definition.floatable) return state;
    const removed = removePanel(state, action.panelId);
    return { ...removed, placements: removed.placements.map((candidate) => candidate.panelId === action.panelId ? { ...candidate, region: "floating", groupId: null, visible: true } : candidate) };
  }

  if (action.type === "setVisible") {
    return { ...state, placements: state.placements.map((candidate) => candidate.panelId === action.panelId ? { ...candidate, visible: action.visible } : candidate) };
  }

  if (action.type === "activate") {
    return { ...state, groups: state.groups.map((group) => group.panelIds.includes(action.panelId) ? { ...group, activePanelId: action.panelId } : group) };
  }

  const group = state.groups.find((candidate) => candidate.panelIds.includes(action.panelId));
  if (!group) return state;
  const remaining = group.panelIds.filter((id) => id !== action.panelId);
  const beforeIndex = action.beforePanelId === null ? remaining.length : remaining.findIndex((id) => id === action.beforePanelId);
  remaining.splice(beforeIndex < 0 ? remaining.length : beforeIndex, 0, action.panelId);
  return { ...state, groups: state.groups.map((candidate) => candidate.id === group.id ? { ...candidate, panelIds: remaining } : candidate) };
}

export function serializeDockLayout(state: DockLayoutState): string {
  return JSON.stringify(state);
}

export function restoreDockLayout(serialized: string): DockLayoutState {
  try {
    const candidate = JSON.parse(serialized) as DockLayoutState;
    if (!Array.isArray(candidate.groups) || !Array.isArray(candidate.placements)) return initialDockLayout;
    return candidate;
  } catch {
    return initialDockLayout;
  }
}
