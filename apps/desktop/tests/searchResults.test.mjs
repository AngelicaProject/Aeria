import assert from "node:assert/strict";
import test from "node:test";

import { choose, chosenByPath, commonTerm, groupBySheet, reconcileChosen, unchooseFiles } from "../src/searchResults.ts";

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
  const issue = (kind, term) => ({ kind, term, advice: kind === "termNotUsed", group: "", message: "", translation: null, variant: null, text: null, phrases: [] });
  const hit = (...findings) => ({ path: "a.po", context: "a", findings });
  assert.equal(commonTerm([hit(issue("forbiddenTerm", "primal")), hit(issue("termNotUsed", "Scions"), issue("termNotUsed", "primal"))]), "primal");
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
