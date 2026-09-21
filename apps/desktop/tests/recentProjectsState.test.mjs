import assert from "node:assert/strict";
import test from "node:test";

import {
  initialRecentProjectsState,
  reduceRecentProjectsState,
} from "../src/recentProjectsState.ts";
import {
  launcherErrorTitle,
  sourcePackageListenerError,
} from "../src/launcherErrorState.ts";

const project = {
  id: "local-project",
  repositoryRoot: "C:\\Projects\\aeria",
  sourcePackagePath: "C:\\Sources\\source.hsp",
  sourcePackageId: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
  sourceLanguage: "en",
  targetLanguage: "fr",
  gameVersion: "test",
  lastOpenedAtUnixMs: 1,
  availability: "ready",
};

test("dismissed recent-project load failure remains a stable unavailable state", () => {
  const failed = reduceRecentProjectsState(initialRecentProjectsState, {
    type: "failed",
    error: { code: "projectRegistryRead", message: "registry unavailable" },
  });
  const dismissed = reduceRecentProjectsState(failed, { type: "dismissError" });

  assert.deepEqual(dismissed, { status: "failed", error: null });
  assert.notEqual(dismissed.status, "loading");
});

test("successful recent-project removal updates the loaded list locally", () => {
  const loaded = reduceRecentProjectsState(initialRecentProjectsState, {
    type: "loaded",
    projects: [project],
  });

  assert.deepEqual(
    reduceRecentProjectsState(loaded, { type: "removed", projectId: project.id }),
    { status: "loaded", projects: [] },
  );
});

test("recent-project open errors use a recent-project-specific title", () => {
  assert.equal(launcherErrorTitle("recentOpen"), "Could not reopen recent project");
  assert.equal(launcherErrorTitle("create"), "Could not create project");
});

test("source-package listener failures use a typed user-facing error", () => {
  const error = sourcePackageListenerError();

  assert.equal(error.code, "sourcePackageListener");
  assert.match(error.message, /progress updates/i);
});
