import assert from "node:assert/strict";
import test from "node:test";
import {
  closeDocumentTab,
  documentIdForSheet,
  initialDocumentTabsState,
  openPreviewTab,
  pinPreviewTab,
  documentIdForCommit,
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

test("commit tabs reuse one preview, sit beside sheets, and never replace a sheet preview", () => {
  let state = openPreviewTab(initialDocumentTabsState, "A");
  state = reduceDocumentTabs(state, { type: "openCommit", commitId: "c1", label: "c1" });
  state = reduceDocumentTabs(state, { type: "openCommit", commitId: "c2", label: "c2" });
  assert.deepEqual(state.tabs.map((tab) => tab.id), [documentIdForSheet("A"), documentIdForCommit("c2")]);
  assert.equal(state.activeId, documentIdForCommit("c2"));
  state = pinPreviewTab(state, documentIdForCommit("c2"));
  state = reduceDocumentTabs(state, { type: "openCommit", commitId: "c3", label: "c3" });
  assert.equal(state.tabs.length, 3);
  state = openPreviewTab(state, "B");
  assert.deepEqual(state.tabs.map((tab) => tab.kind), ["sheet", "commit", "commit"]);
  state = closeDocumentTab(state, documentIdForCommit("c3"));
  assert.ok(state.activeId);
});
