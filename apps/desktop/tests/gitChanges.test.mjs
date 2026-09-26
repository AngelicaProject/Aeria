import assert from "node:assert/strict";
import test from "node:test";

import { COLLAPSE_THRESHOLD, flattenChanges, groupChanges, kindCounts, matchesChange } from "../src/gitChanges.ts";

const change = (sheetName, rowId, kind, text, columnIndex = 0) => {
  const version = { sourceBinding: { sheetName, rowId, subrowId: 0, columnIndex }, targetMacro: text };
  return { translationUnitId: `${sheetName}:${rowId}:${columnIndex}`, kind, before: kind === "added" ? null : version, after: kind === "removed" ? null : version };
};

const changes = [
  change("Item", 12, "added", "Зелье силы"),
  change("Addon", 3, "modified", "Изменить порядок"),
  change("Item", 2, "removed", "Старый текст"),
  change("Addon", 3, "added", "Подкоманды", 1),
];

test("changes group by sheet, sorted by row, and note mixed columns", () => {
  const groups = groupChanges(changes, "?");
  assert.deepEqual(groups.map((group) => group.sheetName), ["Addon", "Item"]);
  assert.deepEqual(groups[1].changes.map((entry) => entry.after?.sourceBinding.rowId ?? entry.before?.sourceBinding.rowId), [2, 12]);
  assert.deepEqual(groups.map((group) => group.multiColumn), [true, false]);
  assert.deepEqual(kindCounts(changes), { added: 2, modified: 1, removed: 1 });
});

test("the filter matches kind and sheet, coordinates, or text", () => {
  const all = { query: "", kind: "all" };
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "зелье" }), true);
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "12:0" }), true);
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "12" }), true);
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "2" }), false, "a row number matches exactly");
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "12:1" }), false);
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "item" }), true);
  assert.equal(matchesChange(changes[0], "Item", { ...all, query: "порядок" }), false);
  assert.equal(matchesChange(changes[2], "Item", { ...all, query: "старый" }), true, "removed text is searchable");
  assert.equal(matchesChange(changes[0], "Item", { ...all, kind: "modified" }), false);
});

test("closed sheets hide their changes unless a filter is active", () => {
  const groups = groupChanges(changes, "?");
  const closed = (name) => name === "Item";
  const items = flattenChanges(groups, { query: "", kind: "all" }, closed);
  assert.deepEqual(items.map((item) => item.type === "group" ? `${item.group.sheetName}${item.closed ? "+" : "-"}` : item.change.kind), ["Addon-", "modified", "added", "Item+"]);

  const filtered = flattenChanges(groups, { query: "", kind: "added" }, closed);
  assert.deepEqual(filtered.map((item) => item.type === "group" ? `${item.group.sheetName}:${item.shown}` : item.change.kind), ["Addon:1", "added", "Item:1", "added"]);
  assert.deepEqual(flattenChanges(groups, { query: "нет такого", kind: "all" }, closed), []);
  assert.ok(COLLAPSE_THRESHOLD > 0);
});
