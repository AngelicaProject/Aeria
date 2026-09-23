import assert from "node:assert/strict";
import test from "node:test";
import { fuzzyFilter, fuzzyMatch } from "../src/fuzzy.ts";

test("fuzzy match requires ordered characters", () => {
  assert.ok(fuzzyMatch("adn", "Addon"));
  assert.equal(fuzzyMatch("nda", "Addon"), null);
  assert.deepEqual(fuzzyMatch("", "Addon"), { score: 0, indices: [] });
});

test("fuzzy ranking prefers prefixes and word starts", () => {
  const names = ["AddonTransient", "Addon", "quest/000/ClsAdd", "BNpcName"];
  assert.deepEqual(fuzzyFilter("addon", names, (name) => name).map((result) => result.item), ["Addon", "AddonTransient"]);
  assert.equal(fuzzyFilter("bnn", names, (name) => name)[0].item, "BNpcName");
  assert.deepEqual(fuzzyMatch("at", "AddonTransient").indices, [0, 5]);
});
