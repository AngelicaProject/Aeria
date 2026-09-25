import assert from "node:assert/strict";
import test from "node:test";

import { filterRows, inputsFromRows, rowProblems, rowsChanged, rowsFromEntries } from "../src/projectGuide.ts";

const entries = [
  { term: "Aether", translation: "Эфир", forbidden: ["Этер", "Эйтер"] },
  { term: "Crystal", translation: "Кристалл", note: "materials" },
];

test("rows round-trip entries and trim edits", () => {
  const rows = rowsFromEntries(entries);
  assert.equal(rows[0].forbidden, "Этер; Эйтер");
  assert.deepEqual(inputsFromRows([{ ...rows[1], term: " Crystal ", note: "  ", forbidden: " ; a ;" }]), [
    { term: "Crystal", translation: "Кристалл", note: null, forbidden: ["a"] },
  ]);
  assert.equal(rowsChanged(rows, entries), false);
  assert.equal(rowsChanged([{ ...rows[0], translation: "Эфир " }, rows[1]], entries), false);
  assert.equal(rowsChanged([rows[0]], entries), true);
});

test("problems follow the glossary format rules", () => {
  const rows = [
    { key: 1, term: "Aether", translation: "Эфир", note: "", forbidden: "" },
    { key: 2, term: "aether", translation: "Эфир", note: "", forbidden: "" },
    { key: 3, term: " ", translation: "x", note: "", forbidden: "" },
    { key: 4, term: "Ice", translation: "", note: "", forbidden: "" },
  ];
  assert.deepEqual([...rowProblems(rows)], [[2, "duplicateTerm"], [3, "emptyTerm"], [4, "emptyTranslation"]]);
});

test("filters match any field ignoring case", () => {
  const rows = rowsFromEntries(entries);
  assert.deepEqual(filterRows(rows, "MATER").map((row) => row.term), ["Crystal"]);
  assert.deepEqual(filterRows(rows, "эйтер").map((row) => row.term), ["Aether"]);
  assert.equal(filterRows(rows, "  ").length, 2);
});
