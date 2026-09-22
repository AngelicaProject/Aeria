import assert from "node:assert/strict";
import test from "node:test";
import {
  closeDocumentTab,
  documentIdForSheet,
  initialDocumentTabsState,
  openPreviewTab,
  pinPreviewTab,
  reduceDocumentTabs,
} from "../src/documentTabs.ts";

test("document tabs replace an unedited preview and preserve pinned tabs", () => {
  let state = openPreviewTab(initialDocumentTabsState, "A");
  state = openPreviewTab(state, "B");
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["B"]);
  state = pinPreviewTab(state, documentIdForSheet("B"));
  state = openPreviewTab(state, "C");
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["B", "C"]);
  assert.equal(state.tabs[0]?.pinned, true);
});

test("editing pins a preview and close selects the nearest neighbor", () => {
  let state = reduceDocumentTabs(initialDocumentTabsState, { type: "openSheet", sheetName: "A" });
  state = reduceDocumentTabs(state, { type: "setDirty", id: documentIdForSheet("A"), dirty: true });
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "B", pin: true });
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "C", pin: true });
  state = closeDocumentTab(state, documentIdForSheet("B"));
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["A", "C"]);
  assert.equal(state.activeId, documentIdForSheet("C"));
});

test("document tabs reorder without duplicating a document", () => {
  let state = initialDocumentTabsState;
  for (const sheetName of ["A", "B", "C"]) state = reduceDocumentTabs(state, { type: "openSheet", sheetName, pin: true });
  state = reduceDocumentTabs(state, { type: "reorder", id: documentIdForSheet("C"), beforeId: documentIdForSheet("A") });
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["C", "A", "B"]);
  assert.equal(new Set(state.tabs.map((tab) => tab.id)).size, 3);
});
