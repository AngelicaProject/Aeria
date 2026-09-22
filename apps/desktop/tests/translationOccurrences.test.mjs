import assert from "node:assert/strict";
import test from "node:test";

import { flattenTranslationRows, occurrenceKey } from "../src/translationOccurrences.ts";

const binding = (columnIndex) => ({ sheetName: "Adventure", rowId: 2162688, subrowId: 0, columnIndex });

test("flattening preserves row paging while exposing one occurrence per cell", () => {
  const occurrences = flattenTranslationRows([
    {
      sheetName: "Adventure",
      rowId: 2162688,
      subrowId: 0,
      context: [],
      cells: [
        { sourceBinding: binding(0), sourceMacro: "source A", translation: { translationUnitId: "tu1", targetMacro: "target A", reviewState: "reviewed", translatorNote: null } },
        { sourceBinding: binding(3), sourceMacro: "source B", translation: null },
      ],
    },
  ]);

  assert.equal(occurrences.length, 2);
  assert.deepEqual(occurrences.map(({ binding: value }) => value.columnIndex), [0, 3]);
  assert.equal(occurrences[0].fieldCountInRow, 2);
  assert.equal(occurrences[0].firstInRow, true);
  assert.equal(occurrences[0].lastInRow, false);
  assert.equal(occurrences[1].firstInRow, false);
  assert.equal(occurrences[1].lastInRow, true);
  assert.equal(occurrences[1].targetMacro, null);
  assert.equal(occurrences[1].reviewState, null);
});

test("occurrence identity includes the non-contiguous column index", () => {
  const first = { binding: binding(0) };
  const second = { binding: binding(3) };

  assert.notEqual(occurrenceKey(first), occurrenceKey(second));
  assert.match(occurrenceKey(second), /3$/);
});
