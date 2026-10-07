import assert from "node:assert/strict";
import test from "node:test";

import { filterRows, folderPath, folderTree, inputsFromRows, moveFolder, moveFolders, openCandidates, parentFolder, rowFromCandidate, rowProblems, rowsChanged, rowsFromEntries, splitForms } from "../src/projectGuide.ts";

const entries = [
  { term: "Aether", translation: "Эфир", forms: ["aethers"], folder: "Lore/Magic" },
  { term: "Crystal", translation: "Кристалл", note: "materials" },
];

test("rows round-trip entries and trim edits", () => {
  const rows = rowsFromEntries(entries);
  assert.deepEqual(rows[0].forms, ["aethers"]);
  assert.equal(rows[0].folder, "Lore/Magic");
  assert.deepEqual(inputsFromRows([{ ...rows[1], term: " Crystal ", note: "  ", forms: [" crystals ", " "], folder: " Items / Ore/" }]), [
    { term: "Crystal", translation: "Кристалл", forms: ["crystals"], note: null, folder: "Items/Ore", matchCase: false },
  ]);
  assert.equal(rowsFromEntries([{ term: "the Maelstrom", translation: "Мальстрём", matchCase: true }])[0].matchCase, true);
  assert.equal(rowsChanged([{ ...rows[0], matchCase: true }, rows[1]], entries), true);
  assert.equal(rowsChanged([{ ...rows[0], folder: "Lore" }, rows[1]], entries), true);
  assert.equal(rowsChanged(rows, entries), false);
  assert.equal(rowsChanged([{ ...rows[0], translation: "Эфир " }, rows[1]], entries), false);
  assert.equal(rowsChanged([rows[0]], entries), true);
});

test("forms typed together are split at semicolons", () => {
  assert.deepEqual(splitForms(" linkshells; ; LS "), ["linkshells", "LS"]);
  assert.deepEqual(splitForms("  "), []);
});

test("problems follow the glossary format rules", () => {
  const row = (key, term, translation, forms = []) => ({ key, term, translation, forms, note: "", folder: "" });
  const rows = [
    row(1, "Aether", "Эфир", ["aethers"]),
    row(2, "aether", "Эфир"),
    row(3, " ", "x"),
    row(4, "Ice", ""),
    row(5, "Aethers", "Эфиры"),
    row(6, "linkshell", "линкшелл", ["Linkshell"]),
    row(7, "Crystal", "Кристалл", ["AETHERS"]),
  ];
  assert.deepEqual([...rowProblems(rows)], [[2, "duplicateTerm"], [3, "emptyTerm"], [4, "emptyTranslation"], [5, "duplicateTerm"], [6, "duplicateTerm"], [7, "duplicateTerm"]]);
});

test("filters match any field and form ignoring case, within a folder", () => {
  const rows = rowsFromEntries(entries);
  assert.deepEqual(filterRows(rows, "MATER").map((row) => row.term), ["Crystal"]);
  assert.deepEqual(filterRows(rows, "AETHERS").map((row) => row.term), ["Aether"]);
  assert.equal(filterRows(rows, "  ").length, 2);
  assert.deepEqual(filterRows(rows, "", "Lore").map((row) => row.term), ["Aether"]);
  assert.deepEqual(filterRows(rows, "", "Lore/Magic").map((row) => row.term), ["Aether"]);
  assert.deepEqual(filterRows(rows, "", "Lo").map((row) => row.term), []);
});

test("folders form a tree with every parent and counts", () => {
  const rows = rowsFromEntries([
    { term: "a", translation: "a", folder: "Lore/Magic" },
    { term: "b", translation: "b", folder: "Lore" },
    { term: "c", translation: "c", folder: "Bestiary" },
    { term: "d", translation: "d" },
  ]);
  assert.deepEqual(folderTree(rows, ["Lore/Places", " Zones /"]), [
    { path: "Bestiary", name: "Bestiary", depth: 0, count: 1 },
    { path: "Lore", name: "Lore", depth: 0, count: 2 },
    { path: "Lore/Magic", name: "Magic", depth: 1, count: 1 },
    { path: "Lore/Places", name: "Places", depth: 1, count: 0 },
    { path: "Zones", name: "Zones", depth: 0, count: 0 },
  ]);
});

test("a folder moves with its subfolders and terms", () => {
  const rows = rowsFromEntries([
    { term: "a", translation: "a", folder: "Lore/Magic" },
    { term: "b", translation: "b", folder: "Lore" },
    { term: "c", translation: "c", folder: "Lorekeepers" },
  ]);
  assert.deepEqual(moveFolder(rows, "Lore", "World/Lore").map((row) => row.folder), ["World/Lore/Magic", "World/Lore", "Lorekeepers"]);
  // Removing a folder moves what it holds to its parent.
  assert.deepEqual(moveFolder(rows, "Lore/Magic", parentFolder("Lore/Magic")).map((row) => row.folder), ["Lore", "Lore", "Lorekeepers"]);
  assert.deepEqual(moveFolder(rows, "Lore", "").map((row) => row.folder), ["Magic", "", "Lorekeepers"]);
  assert.deepEqual(moveFolders(["Lore/Places", "Lore", "Zones"], "Lore", ""), ["Places", "Zones"]);
  assert.equal(folderPath(" a / /b/ "), "a/b");
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

test("a candidate becomes a term with the chosen rendering", () => {
  const row = rowFromCandidate(kojin, 1, 7, "Peoples");
  assert.deepEqual(row, { key: 7, term: "Kojin", translation: "кодзин", forms: [], note: "", folder: "Peoples", matchCase: true });
});

test("candidates already in the glossary or skipped are not offered", () => {
  const grace = { ...kojin, phrase: "Grace" };
  const twelveswood = { ...kojin, phrase: "Twelveswood" };
  const elezen = { ...kojin, phrase: "Elezen" };
  const rows = rowsFromEntries([{ term: "the Kojin", translation: "кодзин" }, { term: "Elf", translation: "эльф", forms: ["the Elezen"] }]);
  assert.deepEqual(openCandidates([kojin, grace, twelveswood, elezen], rows, new Set(["Grace"])).map((candidate) => candidate.phrase), ["Twelveswood"]);
});
