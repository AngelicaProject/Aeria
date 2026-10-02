import { emitTo, listen } from "@tauri-apps/api/event";
import type { GitCommitDto, SourceBinding } from "./types";

/**
 * Requests a floated tool window sends to the main window, which owns the
 * editor: open a string, read changed files again, show the machine
 * translation dialog, or open a commit.
 */
const MAIN_WINDOW = "main";
const REVEAL_STRING = "workbench://reveal-string";
const WORKSPACE_CHANGED = "workbench://workspace-changed";
const OPEN_TRANSLATE = "workbench://open-translate";
const OPEN_COMMIT = "workbench://open-commit";

export const mainWindowRequests = {
  revealString: (binding: SourceBinding) => void emitTo(MAIN_WINDOW, REVEAL_STRING, binding),
  workspaceChanged: () => void emitTo(MAIN_WINDOW, WORKSPACE_CHANGED),
  openTranslate: () => void emitTo(MAIN_WINDOW, OPEN_TRANSLATE),
  openCommit: (commit: GitCommitDto) => void emitTo(MAIN_WINDOW, OPEN_COMMIT, commit),
};

type Handlers = {
  revealString: (binding: SourceBinding) => void;
  workspaceChanged: () => void;
  openTranslate: () => void;
  openCommit: (commit: GitCommitDto) => void;
};

/** Listens in the main window for the requests of tool windows; returns the cleanup. */
export function listenToToolWindows(handlers: Handlers): () => void {
  const subscriptions = [
    listen<SourceBinding>(REVEAL_STRING, (event) => handlers.revealString(event.payload)),
    listen(WORKSPACE_CHANGED, () => handlers.workspaceChanged()),
    listen(OPEN_TRANSLATE, () => handlers.openTranslate()),
    listen<GitCommitDto>(OPEN_COMMIT, (event) => handlers.openCommit(event.payload)),
  ];
  return () => {
    for (const subscription of subscriptions) void subscription.then((unlisten) => unlisten()).catch(() => undefined);
  };
}
