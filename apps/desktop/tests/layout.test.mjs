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

  assert.equal(expanded.regions.leftDock.size, 420);
  assert.equal(clamped.regions.leftDock.size, 190);
  assert.equal(clamped.regions.editor.size, initialWorkbenchLayout.regions.editor.size);
});

test("layout reducer keeps panel and document topology separate from visibility", () => {
  const hidden = reduceWorkbenchLayout(initialWorkbenchLayout, { type: "toggleRegion", regionId: "leftDock" });
  const active = reduceWorkbenchLayout(hidden, { type: "setActiveDocument", documentId: "review" });
  const tabbed = reduceWorkbenchLayout(active, { type: "setActiveTab", regionId: "editor", tabId: "review" });

  assert.equal(hidden.regions.leftDock.visible, false);
  assert.deepEqual(hidden.regions.leftDock.panelIds, ["sheets"]);
  assert.equal(tabbed.regions.editor.activeTabId, "review");
  assert.equal(tabbed.activeDocumentId, "review");
});

test("layout reducer keeps right dock and bottom panel state independent", () => {
  const resized = reduceWorkbenchLayout(initialWorkbenchLayout, { type: "resizeRegion", regionId: "rightDock", delta: 500 });
  const bottom = reduceWorkbenchLayout(resized, { type: "resizeRegion", regionId: "bottomPanel", delta: -500 });
  const shown = reduceWorkbenchLayout(bottom, { type: "setRegionVisibility", regionId: "bottomPanel", visible: true });
  const hidden = reduceWorkbenchLayout(shown, { type: "setRegionVisibility", regionId: "bottomPanel", visible: false });

  assert.equal(resized.regions.rightDock.size, 560);
  assert.equal(bottom.regions.bottomPanel.size, 100);
  assert.equal(hidden.regions.rightDock.visible, true);
  assert.equal(hidden.regions.bottomPanel.visible, false);
});
