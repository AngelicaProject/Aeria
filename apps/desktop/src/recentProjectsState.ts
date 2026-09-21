import type { CommandError, RecentProjectDto } from "./types";

export type RecentProjectsState =
  | { status: "loading" }
  | { status: "loaded"; projects: RecentProjectDto[] }
  | { status: "failed"; error: CommandError | null };

export type RecentProjectsAction =
  | { type: "loaded"; projects: RecentProjectDto[] }
  | { type: "failed"; error: CommandError }
  | { type: "dismissError" }
  | { type: "removed"; projectId: string };

export const initialRecentProjectsState: RecentProjectsState = { status: "loading" };

export function reduceRecentProjectsState(
  state: RecentProjectsState,
  action: RecentProjectsAction,
): RecentProjectsState {
  switch (action.type) {
    case "loaded":
      return { status: "loaded", projects: action.projects };
    case "failed":
      return { status: "failed", error: action.error };
    case "dismissError":
      return state.status === "failed" ? { status: "failed", error: null } : state;
    case "removed":
      return state.status === "loaded"
        ? {
            status: "loaded",
            projects: state.projects.filter((project) => project.id !== action.projectId),
          }
        : state;
  }
}
