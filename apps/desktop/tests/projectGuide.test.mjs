import assert from "node:assert/strict";
import test from "node:test";

import { filterRows, inputsFromRows, openCandidates, rowFromCandidate, rowProblems, rowsChanged, rowsFromEntries } from "../src/projectGuide.ts";

const entries = [
  { term: "Aether", translation: "Эфир", forbidden: ["Этер", "Эйтер"] },
  { term: "Crystal", translation: "Кристалл", note: "materials" },
];

test("rows round-trip entries and trim edits", () => {
  const rows = rowsFromEntries(entries);
  assert.equal(rows[0].forbidden, "Этер; Эйтер");
  assert.deepEqual(inputsFromRows([{ ...rows[1], term: " Crystal ", note: "  ", forbidden: " ; a ;" }]), [
    { term: "Crystal", translation: "Кристалл", note: null, forbidden: ["a"], settled: false, matchCase: false },
  ]);
  assert.equal(rowsFromEntries([{ term: "the Maelstrom", translation: "Мальстрём", matchCase: true }])[0].matchCase, true);
  assert.equal(rowsChanged([{ ...rows[0], matchCase: true }, rows[1]], entries), true);
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

const kojin = {
  phrase: "Kojin",
  strings: 291,
  translated: 285,
  renderings: [
    { words: ["кудзин"], strings: 86, examples: [] },
    { words: ["кодзин"], strings: 84, examples: [] },
    { words: ["койджинов"], strings: 43, examples: [] },
  ],
  spellings: true,
  sheets: [{ sheet: "quest", count: 171 }],
};

test("a candidate becomes a term with the chosen rendering and the others forbidden", () => {
  const row = rowFromCandidate(kojin, 1, 7);
  assert.deepEqual(row, { key: 7, term: "Kojin", translation: "кодзин", note: "", forbidden: "кудзин; койджинов", settled: true, matchCase: true });
});

test("candidates already in the glossary or skipped are not offered", () => {
  const grace = { ...kojin, phrase: "Grace" };
  const twelveswood = { ...kojin, phrase: "Twelveswood" };
  const rows = rowsFromEntries([{ term: "the Kojin", translation: "кодзин" }]);
  assert.deepEqual(openCandidates([kojin, grace, twelveswood], rows, new Set(["Grace"])).map((candidate) => candidate.phrase), ["Twelveswood"]);
});
