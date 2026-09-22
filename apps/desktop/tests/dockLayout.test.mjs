import assert from "node:assert/strict";
import test from "node:test";
import {
  initialDockLayout,
  reduceDockLayout,
  restoreDockLayout,
  serializeDockLayout,
} from "../src/dockLayout.ts";

test("dock layout moves a singleton panel between regions", () => {
  const moved = reduceDockLayout(initialDockLayout, { type: "move", panelId: "sheets", region: "bottom" });
  assert.equal(moved.placements.find((placement) => placement.panelId === "sheets")?.region, "bottom");
  assert.equal(moved.groups.find((group) => group.region === "left")?.panelIds.includes("sheets"), false);
  assert.equal(moved.groups.find((group) => group.region === "bottom")?.panelIds.includes("sheets"), true);
});

test("dock layout supports floating and restoring a panel", () => {
  const floating = reduceDockLayout(initialDockLayout, { type: "float", panelId: "ai" });
  assert.equal(floating.placements.find((placement) => placement.panelId === "ai")?.region, "floating");
  const restored = reduceDockLayout(floating, { type: "restore", panelId: "ai", region: "right" });
  assert.equal(restored.placements.find((placement) => placement.panelId === "ai")?.region, "right");
});

test("dock layout serializes and rejects malformed state", () => {
  assert.deepEqual(restoreDockLayout(serializeDockLayout(initialDockLayout)), initialDockLayout);
  assert.deepEqual(restoreDockLayout("not json").groups, initialDockLayout.groups);
});
