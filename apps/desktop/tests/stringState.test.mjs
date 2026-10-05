import assert from "node:assert/strict";
import test from "node:test";
import { stringState } from "../src/stringState.ts";
import { filterOccurrences, emptyOccurrenceFilter } from "../src/translationOccurrences.ts";

const overlay = (fields) => ({ targetMacro: "Перевод", fuzzy: false, translatorNote: null, previousSource: null, ...fields });

test("a reviewed translation has its own state, and a changed source comes first", () => {
  assert.equal(stringState(null), null);
  assert.equal(stringState(overlay({ targetMacro: "" })), null);
  assert.equal(stringState(overlay({})), "translated");
  assert.equal(stringState(overlay({ reviewed: true })), "reviewed");
  assert.equal(stringState(overlay({ reviewed: true, fuzzy: true })), "fuzzy");
  assert.equal(stringState(overlay({ reviewStale: true })), "translated", "a stale mark protects nothing");
});

test("the list filters reviewed strings apart from other translations", () => {
  const occurrence = (state, row) => ({ binding: { sheetName: "S", rowId: row, subrowId: 0, columnIndex: 0 }, state, sourceMacro: "x", targetMacro: "y", formattingOnly: false });
  const all = [occurrence("translated", 1), occurrence("reviewed", 2), occurrence(null, 3)];
  const rows = (status) => filterOccurrences(all, { ...emptyOccurrenceFilter, status }).map((item) => item.binding.rowId);
  assert.deepEqual(rows("reviewed"), [2]);
  assert.deepEqual(rows("translated"), [1]);
  assert.deepEqual(rows("untranslated"), [3]);
});
