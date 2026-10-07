import assert from "node:assert/strict";
import test from "node:test";
import {
  closeDocumentTab,
  closeDocumentTabs,
  documentIdForSheet,
  initialDocumentTabsState,
  openPreviewTab,
  keepPreviewTab,
  documentIdForCommit,
  reduceDocumentTabs,
} from "../src/documentTabs.ts";

test("document tabs replace an unedited preview and preserve kept tabs", () => {
  let state = openPreviewTab(initialDocumentTabsState, "A");
  state = openPreviewTab(state, "B");
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["B"]);
  state = keepPreviewTab(state, documentIdForSheet("B"));
  state = openPreviewTab(state, "C");
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["B", "C"]);
  assert.equal(state.tabs[0]?.preview, false);
});

test("editing pins a preview and close selects the nearest neighbor", () => {
  let state = reduceDocumentTabs(initialDocumentTabsState, { type: "openSheet", sheetName: "A" });
  state = reduceDocumentTabs(state, { type: "setDirty", id: documentIdForSheet("A"), dirty: true });
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "B", keep: true });
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "C", keep: true });
  state = closeDocumentTab(state, documentIdForSheet("B"));
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["A", "C"]);
  assert.equal(state.activeId, documentIdForSheet("C"));
});

test("document tabs reorder without duplicating a document", () => {
  let state = initialDocumentTabsState;
  for (const sheetName of ["A", "B", "C"]) state = reduceDocumentTabs(state, { type: "openSheet", sheetName, keep: true });
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
  state = keepPreviewTab(state, documentIdForCommit("c2"));
  state = reduceDocumentTabs(state, { type: "openCommit", commitId: "c3", label: "c3" });
  assert.equal(state.tabs.length, 3);
  state = openPreviewTab(state, "B");
  assert.deepEqual(state.tabs.map((tab) => tab.kind), ["sheet", "commit", "commit"]);
  state = closeDocumentTab(state, documentIdForCommit("c3"));
  assert.ok(state.activeId);
});

test("closing tabs opens the next sheet or leaves none open", () => {
  let state = reduceDocumentTabs(initialDocumentTabsState, { type: "openSheet", sheetName: "A", keep: true });
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "B", keep: true });
  const a = documentIdForSheet("A");
  const b = documentIdForSheet("B");

  // The open sheet's tab closes and the other sheet opens.
  assert.deepEqual(closeDocumentTabs(state, [b], "B"), { state: { tabs: [state.tabs[0]], activeId: a }, open: "A", clear: false });
  // Every tab closes: nothing is left on screen.
  const all = closeDocumentTabs(state, [a, b], "B");
  assert.equal(all.state.activeId, null);
  assert.equal(all.open, null);
  assert.equal(all.clear, true);
  // Another tab closes: the open sheet stays as it is.
  assert.deepEqual(closeDocumentTabs(state, [a], "B").open, null);
  assert.equal(closeDocumentTabs(state, [a], "B").clear, false);

  // A commit tab left active covers no sheet once the sheet's tab is gone.
  const withCommit = reduceDocumentTabs(state, { type: "openCommit", commitId: "c1", label: "c1" });
  const behind = closeDocumentTabs(withCommit, [a, b], "B");
  assert.equal(behind.state.activeId, documentIdForCommit("c1"));
  assert.equal(behind.clear, true);
  // Closing the commit tab returns to the sheet under it without reopening it.
  const back = closeDocumentTabs(withCommit, [documentIdForCommit("c1")], "B");
  assert.equal(back.state.activeId, b);
  assert.equal(back.open, null);
  assert.equal(back.clear, false);
});

test("pinned tabs stay first and keep their place among themselves", () => {
  let state = initialDocumentTabsState;
  for (const sheetName of ["A", "B", "C"]) state = reduceDocumentTabs(state, { type: "openSheet", sheetName, keep: true });
  state = reduceDocumentTabs(state, { type: "setPinned", id: documentIdForSheet("C"), pinned: true });
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["C", "A", "B"]);
  state = reduceDocumentTabs(state, { type: "setPinned", id: documentIdForSheet("B"), pinned: true });
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["C", "B", "A"]);
  // A tab dragged in front of a pinned one stays after them.
  state = reduceDocumentTabs(state, { type: "reorder", id: documentIdForSheet("A"), beforeId: documentIdForSheet("C") });
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["C", "B", "A"]);
  state = reduceDocumentTabs(state, { type: "setPinned", id: documentIdForSheet("C"), pinned: false });
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["B", "C", "A"]);
  // A pinned preview is no longer replaced.
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "D" });
  state = reduceDocumentTabs(state, { type: "setPinned", id: documentIdForSheet("D"), pinned: true });
  state = reduceDocumentTabs(state, { type: "openSheet", sheetName: "E" });
  assert.deepEqual(state.tabs.map((tab) => tab.sheetName), ["B", "D", "C", "A", "E"]);
});
