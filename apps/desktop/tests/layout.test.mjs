import assert from "node:assert/strict";
import test from "node:test";
import { initialWorkbenchLayout, reduceWorkbenchLayout } from "../src/ui/layout.ts";

test("layout reducer resizes a named region within its bounds", () => {
  const expanded = reduceWorkbenchLayout(initialWorkbenchLayout, {
    type: "resizeRegion",
    regionId: "leftDock",
    delta: 500,
  });
  const clamped = reduceWorkbenchLayout(expanded, {
    type: "resizeRegion",
    regionId: "leftDock",
    delta: -500,
  });

  assert.equal(expanded.regions.leftDock.size, 360);
  assert.equal(clamped.regions.leftDock.size, 190);
  assert.equal(clamped.regions.translation.size, initialWorkbenchLayout.regions.translation.size);
});

test("layout reducer keeps panel and document topology separate from visibility", () => {
  const hidden = reduceWorkbenchLayout(initialWorkbenchLayout, { type: "toggleRegion", regionId: "leftDock" });
  const active = reduceWorkbenchLayout(hidden, { type: "setActiveDocument", documentId: "review" });
  const tabbed = reduceWorkbenchLayout(active, { type: "setActiveTab", regionId: "translation", tabId: "review" });

  assert.equal(hidden.regions.leftDock.visible, false);
  assert.deepEqual(hidden.regions.leftDock.panelIds, ["sheets"]);
  assert.equal(tabbed.regions.translation.activeTabId, "review");
  assert.equal(tabbed.activeDocumentId, "review");
});
