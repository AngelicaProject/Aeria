import assert from "node:assert/strict";
import test from "node:test";

import { choose, chosenByPath, commonTerm, groupBySheet, nextRow, reconcileChosen, resultRows, sheetOpen, unchooseFiles } from "../src/searchResults.ts";

test("files of one sheet make one group", () => {
  const groups = groupBySheet([
    { path: "Achievement/0.po", sheet: "Achievement", count: 2 },
    { path: "Item/10000.po", sheet: "Item", count: 3 },
    { path: "Item/12000.po", sheet: "Item", count: 4 },
    { path: "quest/000/A.po", sheet: "", count: 1 },
  ]);
  assert.deepEqual(groups.map((group) => [group.sheet, group.files.length, group.count]), [
    ["Achievement", 1, 2],
    ["Item", 2, 7],
    ["quest/000/A.po", 1, 1],
  ]);
});

test("chosen strings take an exception for a term they all have", () => {
  const issue = (kind, term) => ({ kind, term, advice: kind === "termNotUsed", group: "", message: "", translation: null, text: null, phrases: [] });
  const hit = (...findings) => ({ path: "a.po", context: "a", findings });
  assert.equal(commonTerm([hit(issue("termNotUsed", "primal")), hit(issue("termNotUsed", "Scions"), issue("termNotUsed", "primal"))]), "primal");
  assert.equal(commonTerm([hit(issue("termNotUsed", "Scions")), hit(issue("termNotUsed", "primal"))]), null);
  assert.equal(commonTerm([hit(issue("termNotUsed", "Sage, elder"))]), null, "a term with a comma cannot be a flag");
  assert.equal(commonTerm([]), null);
});

test("chosen strings follow the result read again", () => {
  const hit = (path, context, translation) => ({ path, context, translation, fuzzy: false, findings: [] });
  const chosen = new Map([
    ["a.po|1", hit("a.po", "1", "старый")],
    ["a.po|2", hit("a.po", "2", "ушёл")],
    ["b.po|3", hit("b.po", "3", "не прочитан")],
  ]);
  const next = reconcileChosen(chosen, [hit("a.po", "1", "новый")], (path) => path === "a.po");
  assert.deepEqual([...next.keys()], ["a.po|1", "b.po|3"]);
  assert.equal(next.get("a.po|1").translation, "новый");
  const same = new Map([["a.po|1", hit("a.po", "1", "новый")]]);
  assert.equal(reconcileChosen(same, [hit("a.po", "1", "новый")], () => true), same, "nothing changed");
});

test("a whole sheet is chosen and unchosen by its files", () => {
  const string = (path, context) => ({ path, context, translation: "перевод", fuzzy: false, findings: [] });
  const sheet = [string("Item/10000.po", "1"), string("Item/12000.po", "2")];
  let chosen = choose(new Map(), [string("Addon.po", "9")], true);
  chosen = choose(chosen, sheet, true);
  chosen = choose(chosen, sheet, true);
  assert.deepEqual([...chosenByPath(chosen)], [["Addon.po", 1], ["Item/10000.po", 1], ["Item/12000.po", 1]], "a string is chosen once");
  chosen = unchooseFiles(chosen, new Set(["Item/10000.po", "Item/12000.po"]));
  assert.deepEqual([...chosen.keys()], ["Addon.po|9"]);
  assert.equal(choose(chosen, [string("Addon.po", "9")], false).size, 0);
});

test("the result list shows the strings at hand of open sheets and what is known of the rest", () => {
  const groups = groupBySheet([
    { path: "Addon.po", sheet: "Addon", count: 2 },
    { path: "Item/0.po", sheet: "Item", count: 3 },
    { path: "Quest.po", sheet: "Quest", count: 5 },
    { path: "Status.po", sheet: "Status", count: 4 },
  ]);
  const hit = (path, context) => ({ path, context });
  const known = {
    Addon: { hits: [hit("Addon.po", "a1"), hit("Addon.po", "a2")], complete: true, loading: false, cut: false },
    Item: { hits: [hit("Item/0.po", "i1")], complete: false, loading: false, cut: false },
    Quest: { hits: [], complete: false, loading: true, cut: false },
    Status: { hits: [hit("Status.po", "s1")], complete: false, loading: false, cut: true },
  };
  const rows = resultRows(groups, () => true, (group) => known[group.sheet]);
  assert.deepEqual(rows.map((row) => row.kind === "hit" ? row.hit.context : `${row.kind}:${row.group.sheet}`), [
    "sheet:Addon", "a1", "a2",
    "sheet:Item", "i1", "more:Item",
    "sheet:Quest", "loading:Quest",
    "sheet:Status", "s1", "cut:Status",
  ]);
  const closed = resultRows(groups, (group) => group.sheet === "Item", (group) => known[group.sheet]);
  assert.deepEqual(closed.map((row) => row.kind), ["sheet", "sheet", "hit", "more", "sheet", "sheet"]);
  assert.equal(closed[1].open, true);
  assert.equal(closed[0].open, false);
});

test("the keyboard moves over sheets, strings, and sheets to show, never past the ends", () => {
  const group = { key: "g", sheet: "G", files: [], count: 1 };
  const rows = [
    { kind: "sheet", key: "s", group, open: true },
    { kind: "hit", key: "h", group, hit: {} },
    { kind: "loading", key: "l", group },
    { kind: "more", key: "m", group, shown: 1 },
  ];
  assert.equal(nextRow(rows, -1, 1), 0);
  assert.equal(nextRow(rows, 1, 1), 3, "a message is skipped");
  assert.equal(nextRow(rows, 3, 1), 3, "the last row stays");
  assert.equal(nextRow(rows, 3, -1), 1);
  assert.equal(nextRow(rows, 0, -1), 0);
});

test("a sheet opened to read its strings stays open once they arrive", () => {
  const [group] = groupBySheet([{ path: "custom/002/A.po", sheet: "custom/002/A", count: 7 }]);
  const notSent = () => 0;
  assert.equal(sheetOpen(group, notSent, false, new Set()), false, "closed: none of its strings were sent");
  const toggled = new Set([group.key]);
  assert.equal(sheetOpen(group, notSent, false, toggled), true, "opened");
  // Its strings are read now; what was sent with the search is unchanged.
  assert.equal(sheetOpen(group, notSent, false, toggled), true, "still open after reading");
  assert.equal(sheetOpen(group, () => 3, false, new Set()), true, "open: strings were sent");
  assert.equal(sheetOpen(group, () => 3, true, new Set()), false, "collapse all closes it");
});
