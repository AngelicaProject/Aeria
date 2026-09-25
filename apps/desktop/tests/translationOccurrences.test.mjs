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

test("filters loaded occurrences by review state and text", async () => {
  const { filterOccurrences, adjacentOccurrence } = await import("../src/translationOccurrences.ts");
  const occurrences = flattenTranslationRows([
    {
      sheetName: "Adventure",
      rowId: 7,
      subrowId: 0,
      context: [],
      cells: [
        { sourceBinding: binding(0), sourceMacro: "Hello <num(1)>", formattingOnly: false, translation: { translationUnitId: "tu1", targetMacro: "Bonjour", reviewState: "needsReview", translatorNote: null } },
        { sourceBinding: binding(3), sourceMacro: "World", formattingOnly: false, translation: null },
        { sourceBinding: binding(4), sourceMacro: "...", formattingOnly: true, translation: null },
      ],
    },
  ]);

  assert.deepEqual(filterOccurrences(occurrences, { status: "untranslated", kind: "all", query: "" }).map((item) => item.sourceMacro), ["World", "..."]);
  assert.deepEqual(filterOccurrences(occurrences, { status: "all", kind: "formatting", query: "" }).map((item) => item.sourceMacro), ["..."]);
  assert.deepEqual(filterOccurrences(occurrences, { status: "untranslated", kind: "text", query: "" }).map((item) => item.sourceMacro), ["World"]);
  assert.deepEqual(filterOccurrences(occurrences, { status: "needsReview", kind: "all", query: "" }).map((item) => item.sourceMacro), ["Hello <num(1)>"]);
  assert.deepEqual(filterOccurrences(occurrences, { status: "all", kind: "all", query: "bonj" }).map((item) => item.sourceMacro), ["Hello <num(1)>"]);
  assert.equal(filterOccurrences(occurrences, { status: "reviewed", kind: "all", query: "" }).length, 0);

  assert.equal(adjacentOccurrence(occurrences, binding(0), 1)?.sourceMacro, "World");
  assert.equal(adjacentOccurrence(occurrences, binding(3), 1)?.sourceMacro, "...");
  assert.equal(adjacentOccurrence(occurrences, binding(4), 1), null);
  assert.equal(adjacentOccurrence(occurrences, null, -1)?.sourceMacro, "...");
  const withoutWorld = occurrences.filter((occurrence) => occurrence.sourceMacro !== "World");
  assert.equal(adjacentOccurrence(withoutWorld, binding(3), 1)?.sourceMacro, "...");
  assert.equal(adjacentOccurrence(withoutWorld, binding(3), -1)?.sourceMacro, "Hello <num(1)>");
  assert.equal(adjacentOccurrence(withoutWorld.slice(0, 1), binding(3), 1), null);
});

test("cursorBefore starts a page at the requested row", async () => {
  const { cursorBefore } = await import("../src/translationOccurrences.ts");
  assert.deepEqual(cursorBefore("Addon", 12, 3), { sheetName: "Addon", rowId: 12, subrowId: 2 });
  assert.deepEqual(cursorBefore("Addon", 12, 0), { sheetName: "Addon", rowId: 11, subrowId: 65535 });
  assert.equal(cursorBefore("Addon", 0, 0), null);
});
