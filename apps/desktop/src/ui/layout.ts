export type DockLayoutState = {
  leftDockWidth: number;
  translationWidth: number;
};

export type DockLayoutAction =
  | { type: "resizeLeftDock"; delta: number }
  | { type: "resizeTranslation"; delta: number };

export const initialDockLayout: DockLayoutState = {
  leftDockWidth: 238,
  translationWidth: 390,
};

export function reduceDockLayout(state: DockLayoutState, action: DockLayoutAction): DockLayoutState {
  if (action.type === "resizeLeftDock") {
    return { ...state, leftDockWidth: Math.max(190, Math.min(360, state.leftDockWidth + action.delta)) };
  }
  return { ...state, translationWidth: Math.max(300, Math.min(620, state.translationWidth - action.delta)) };
}
