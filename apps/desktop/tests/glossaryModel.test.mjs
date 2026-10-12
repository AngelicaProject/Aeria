import assert from "node:assert/strict";
import test from "node:test";

import {
  emptyGlossaryFilter,
  entryItems,
  facetCounts,
  filterItems,
  inputsFromRows,
  rowProblems,
  rowsChanged,
  rowsFromEntries,
  sheetItems,
  sortItems,
  splitForms,
  unappliedOf,
  usageKey,
} from "../src/glossaryModel.ts";

const entries = [
  { term: "Aether", translation: "Эфир", forms: ["aethers"], category: "понятие" },
  { term: "Crystal", translation: "Кристалл", note: "materials", category: "предмет" },
];

const sheetTerm = (context, term, translation, category, sheet, variants = [[translation, 1]]) => ({
  sheet,
  context,
  term,
  translation,
  category,
  forms: [],
  strings: variants.reduce((sum, [, strings]) => sum + strings, 0),
  variants: variants.map(([text, strings]) => ({ translation: text, strings })),
});

test("rows round-trip entries and trim edits", () => {
  const rows = rowsFromEntries(entries);
  assert.deepEqual(rows[0].forms, ["aethers"]);
  assert.deepEqual(inputsFromRows([{ ...rows[1], term: " Crystal ", note: "  ", forms: [" crystals ", " "], category: " предмет " }]), [
    { term: "Crystal", translation: "Кристалл", forms: ["crystals"], note: null, matchCase: false, category: "предмет" },
  ]);
  assert.equal(rowsFromEntries([{ term: "the Maelstrom", translation: "Мальстрём", matchCase: true, category: "организация" }])[0].matchCase, true);
  assert.equal(rowsChanged([{ ...rows[0], matchCase: true }, rows[1]], entries), true);
  assert.equal(rowsChanged(rows, entries), false);
  assert.equal(rowsChanged([{ ...rows[0], translation: "Эфир " }, rows[1]], entries), false);
  assert.equal(rowsChanged([rows[0]], entries), true);
});

test("forms typed together are split at semicolons", () => {
  assert.deepEqual(splitForms(" linkshells; ; LS "), ["linkshells", "LS"]);
  assert.deepEqual(splitForms("  "), []);
});

test("problems follow the glossary format rules", () => {
  const row = (key, term, translation, forms = [], category = "понятие") => ({ key, term, translation, forms, note: "", matchCase: false, category });
  const rows = [
    row(1, "Aether", "Эфир", ["aethers"]),
    row(2, "aether", "Эфир"),
    row(3, " ", "x"),
    row(4, "Ice", ""),
    row(5, "Aethers", "Эфиры"),
    row(6, "linkshell", "линкшелл", ["Linkshell"]),
    row(7, "Crystal", "Кристалл", ["AETHERS"]),
    row(8, "Gil", "гил", [], " "),
    row(9, "aether", "эфир", [], "существо"),
  ];
  assert.deepEqual([...rowProblems(rows)], [[2, "duplicateTerm"], [3, "emptyTerm"], [4, "emptyTranslation"], [5, "duplicateTerm"], [6, "duplicateTerm"], [7, "duplicateTerm"], [8, "emptyCategory"]]);
});

test("the list holds the project's terms, then the sheets', with what each entry replaces", () => {
  const rows = rowsFromEntries([
    { term: "The Tangle", translation: "Сплетение", category: "место" },
    { term: "Gil", translation: "гил", category: "валюта" },
    { term: "Ul'dah", translation: "Уль'да", category: "место" },
  ]);
  const tangle = sheetTerm("PlaceName:413:0:0", "The Tangle", "Колтун", "место", "PlaceName");
  const uldah = sheetTerm("PlaceName:40:0:0", "Ul'dah", "Уль'да", "место", "PlaceName");
  const mist = sheetTerm("PlaceName:1:0:0", "Mist", "Туман", "место", "PlaceName", [["Туман", 3], ["Мгла", 1]]);
  const items = [
    ...entryItems(rows, rowProblems(rows), [{ entry: 0, term: tangle }, { entry: 2, term: uldah }], ["Ul'dah"]),
    ...sheetItems([mist, sheetTerm("BNpcName:1:0:0", "mist", "туман", "существо", "BNpcName")]),
  ];
  assert.deepEqual(items.map((item) => item.kind), ["entry", "entry", "entry", "sheet", "sheet"]);
  // Each meaning of a name has what the strings do not apply of its own.
  const unapplied = new Map([[usageKey("Mist", "место"), 4], [usageKey("gil", "валюта"), 1]]);
  assert.equal(unappliedOf(items[3], unapplied), 4);
  assert.equal(unappliedOf(items[4], unapplied), 0);
  assert.equal(unappliedOf(items[0], unapplied), 0);
  const counts = facetCounts(items, unapplied);
  assert.equal(counts.views.all, 5);
  assert.equal(counts.views.entries, 3);
  assert.equal(counts.views.sheets, 2);
  assert.equal(counts.views.unapplied, 2);
  assert.equal(counts.views.redundant, 1);
  assert.deepEqual(counts.categories.map((category) => [category.key, category.count]), [["место", 3], ["валюта", 1], ["существо", 1]]);
  assert.equal(counts.sheets.get("PlaceName"), 1);

  const shown = (filter) => filterItems(items, { ...emptyGlossaryFilter, ...filter }).map((item) => item.id);
  assert.deepEqual(shown({ category: "место", view: "entries" }), ["entry:0", "entry:2"]);
  // A sheet shows its terms and the entries that replace its terms.
  assert.deepEqual(shown({ sheet: "PlaceName" }), ["entry:0", "entry:2", "sheet:PlaceName:1:0:0"]);
  // A query finds forms, translations, and the translations of the sheets' strings.
  assert.deepEqual(shown({ query: "МГЛА" }), ["sheet:PlaceName:1:0:0"]);
  assert.deepEqual(shown({ query: "колтун" }), ["entry:0"]);
  assert.deepEqual(filterItems(items, { ...emptyGlossaryFilter, view: "unapplied" }, unapplied).map((item) => item.id), ["entry:1", "sheet:PlaceName:1:0:0"]);
});

test("a column orders the list, equals keeping their order", () => {
  const rows = rowsFromEntries([
    { term: "b", translation: "в", category: "x" },
    { term: "A", translation: "а", category: "x" },
    { term: "c", translation: "б", category: "x" },
  ]);
  const items = entryItems(rows, new Map(), [], []);
  const order = (sort, usage = null) => sortItems(items, sort, usage).map((item) => item.row.term);
  assert.deepEqual(order({ key: "term", descending: false }), ["A", "b", "c"]);
  assert.deepEqual(order({ key: "translation", descending: true }), ["b", "c", "A"]);
  assert.deepEqual(order({ key: "order", descending: false }), ["b", "A", "c"]);
  const usage = new Map([[usageKey("c", "x"), 7], [usageKey("A", "X"), 7], [usageKey("b", "x"), 1]]);
  assert.deepEqual(order({ key: "uses", descending: true }, usage), ["A", "c", "b"]);
});
