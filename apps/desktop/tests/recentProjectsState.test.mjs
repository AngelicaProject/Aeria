import assert from "node:assert/strict";
import test from "node:test";

import {
  initialRecentProjectsState,
  reduceRecentProjectsState,
} from "../src/recentProjectsState.ts";

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
