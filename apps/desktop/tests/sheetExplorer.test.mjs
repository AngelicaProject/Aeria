import assert from "node:assert/strict";
import test from "node:test";
import {
  buildSheetTree,
  expandSheetAncestors,
  findSheetMatches,
  visibleSheetTreeEntries,
} from "../src/sheetExplorer.ts";

const sheets = [
  { name: "root10", effectiveLanguage: "en", rowCount: 10 },
  { name: "quest/028/StmBdz720_02823", effectiveLanguage: "en", rowCount: 23 },
  { name: "quest/028/StmBdz719_02822", effectiveLanguage: "en", rowCount: 22 },
  { name: "quest/030/Side_03001", effectiveLanguage: "en", rowCount: 1 },
];

test("sheet explorer groups presentation paths without changing canonical names", () => {
  const tree = buildSheetTree(sheets);
  const quest = tree.children.find((entry) => entry.kind === "folder" && entry.path === "quest");
  assert.equal(quest?.kind, "folder");
  assert.equal(quest?.descendantCount, 3);
  assert.deepEqual(visibleSheetTreeEntries(tree, new Set()).map((entry) => entry.name), ["quest", "root10"]);
  assert.deepEqual(visibleSheetTreeEntries(tree, new Set(["quest", "quest/028"])).map((entry) => entry.name), [
    "quest", "028", "StmBdz719_02822", "StmBdz720_02823", "030", "root10",
  ]);
});

test("sheet explorer uses natural order and quick find returns basename and breadcrumb", () => {
  const tree = buildSheetTree(sheets);
  const quest = tree.children.find((entry) => entry.kind === "folder" && entry.path === "quest");
  assert.equal(quest?.kind, "folder");
  assert.equal(quest?.children[0]?.name, "028");
  const expanded = expandSheetAncestors(new Set(), "quest/028/StmBdz720_02823");
  assert.deepEqual([...expanded], ["quest", "quest/028"]);
  assert.deepEqual(findSheetMatches(sheets, "720").map((match) => [match.basename, match.breadcrumb, match.sheet.name]), [["StmBdz720_02823", "quest › 028", "quest/028/StmBdz720_02823"]]);
});
