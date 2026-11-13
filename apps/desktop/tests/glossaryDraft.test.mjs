import assert from "node:assert/strict";
import test from "node:test";
import { createGlossaryDraft } from "../src/glossaryDraft.ts";
import { entryItems, facetCounts, originsForRows, rowProblems, rowsChanged, rowsFromEntries, usageKey } from "../src/glossaryModel.ts";

const entries = [
  { term: "Mist", translation: "Туман", category: "место", forms: ["MIST"] },
  { term: "Mist", translation: "Мгла", category: "существо", note: "Имя", matchCase: true },
  { term: "The Studium", translation: "Академия", category: "место" },
];
const ids = ["mist-place", "mist-creature", "studium"];
const sources = [
  { entry: 0, term: { sheet: "PlaceName", context: "PlaceName:1:0:0", term: "Mist", translation: "Туман", category: "место", forms: [], strings: 1, variants: [] } },
];

test("draft caches preserve the full model through editing, merging, reverting and reload", () => {
  const draft = createGlossaryDraft();
  const saved = rowsFromEntries(entries, [], ids);
  const outdated = new Map([[usageKey("Mist", "место"), 3]]);
  function check(rows, origins = sources, merges = []) {
    const problems = draft.problems(rows);
    assert.deepEqual(problems, rowProblems(rows));
    const expectedOrigins = originsForRows(rows, ids, origins, merges);
    const actualOrigins = draft.origins(rows, ids, origins, merges);
    assert.deepEqual(actualOrigins, expectedOrigins);
    const items = draft.items(rows, problems, actualOrigins);
    assert.deepEqual(items, entryItems(rows, problems, expectedOrigins, [], rows.map((r) => r.key)));
    assert.deepEqual(draft.counts(items, outdated), facetCounts(items, outdated));
    assert.equal(draft.changed(rows, saved), rowsChanged(rows, entries, ids));
  }
  check(saved);
  for (const patch of [
    { note: "Новое примечание" }, { translation: "Туманная область" }, { translation: " " },
    { forms: ["The Studium"] }, { category: "существо" }, { term: "The Studium" }, { matchCase: true },
  ]) check(saved.map((r, i) => i === 0 ? { ...r, ...patch } : r));
  check(saved.map((r, i) => i === 0 ? { ...r, term: " Mist ", translation: " Туман ", note: " " } : r));
  check(saved.toReversed());
  check(saved.slice(1));
  check([...saved, { ...saved[0], id: undefined, key: 9 }]);
  check(saved.filter((r) => r.id !== ids[1]), sources, [{ from: ids[1], into: ids[0] }]);
  check(saved, []);
  check(saved);
  check(rowsFromEntries(entries, saved, ids));
});

test("incremental name validation matches complete validation across changing collisions and row order", () => {
  const draft = createGlossaryDraft();
  let rows = Array.from({ length: 200 }, (_, key) => ({ key, term: `Name ${key}`, translation: "Перевод", category: "место", forms: [], note: "", matchCase: false }));
  let seed = 73;
  const random = (limit) => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed % limit; };
  const patches = [
    () => ({ term: `Name ${random(200)}` }),
    () => ({ forms: [`Name ${random(200)}`, "Alias", "ALIAS"] }),
    () => ({ forms: ["Alias", "Alias"] }),
    () => ({ category: random(2) ? " место " : "существо" }),
    () => ({ translation: random(2) ? "" : "Перевод" }),
    () => ({ term: "", note: "Примечание" }),
  ];
  for (let step = 0; step < 150; step++) {
    const at = random(rows.length);
    const patch = patches[random(patches.length)]();
    rows = rows.map((r, i) => i === at ? { ...r, ...patch } : r);
    if (step % 20 === 0) rows = rows.toReversed();
    assert.deepEqual(draft.problems(rows), rowProblems(rows), `edit ${step}`);
  }
  assert.deepEqual(draft.problems([]), new Map());
  assert.deepEqual(draft.problems(rows), rowProblems(rows));
});

test("typing in a 40000-term glossary does not rebuild or normalize unchanged terms", () => {
  let untouchedReads = 0;
  const rows = Array.from({ length: 40000 }, (_, key) => ({
    key, id: `term-${key}`, term: `Name ${key}`, translation: "Перевод", category: "место", note: "", matchCase: false,
    get forms() { if (key !== 200) untouchedReads++; return []; },
  }));
  const draft = createGlossaryDraft();
  const termIds = rows.map((r) => r.id);
  const merges = [];
  const origins = draft.origins(rows, termIds, sources, merges);
  const problems = draft.problems(rows);
  const before = draft.items(rows, problems, origins);
  const counts = draft.counts(before, null);
  const changed = rows.map((r, i) => i === 200 ? { ...r, translation: "Новый перевод", note: "Новое примечание" } : r);
  untouchedReads = 0;
  assert.equal(draft.changed(changed, rows), true);
  assert.equal(draft.problems(changed), problems);
  assert.equal(draft.origins(changed, termIds, sources, merges), origins);
  const after = draft.items(changed, problems, origins);
  assert.equal(draft.counts(after, null), counts);
  assert.equal(after.filter((item, at) => item === before[at]).length, rows.length - 1);
  assert.equal(untouchedReads, 0);
  assert.equal(draft.changed(rows, rows), false);
});

test("saved responses reuse unchanged draft rows and source cards while refreshing changed sources", () => {
  const draft = createGlossaryDraft();
  const rows = rowsFromEntries(entries, [], ids);
  const merges = [];
  const before = draft.items(rows, draft.problems(rows), draft.origins(rows, ids, sources, merges));
  const counts = draft.counts(before, null);
  const reloaded = rowsFromEntries(structuredClone(entries), rows, [...ids]);
  assert.ok(reloaded.every((row, at) => row === rows[at]));
  const next = draft.items(reloaded, draft.problems(reloaded), draft.origins(reloaded, [...ids], structuredClone(sources), []));
  assert.ok(next.every((item, at) => item === before[at]));
  assert.equal(draft.counts(next, null), counts);
  const changedSources = structuredClone(sources);
  changedSources[0].term.translation = "Новый источник";
  const changed = draft.items(reloaded, draft.problems(reloaded), draft.origins(reloaded, ids, changedSources, []));
  assert.notEqual(changed[0], before[0]);
  assert.equal(changed[1], before[1]);
  assert.equal(changed[0].replaced[0].translation, "Новый источник");
  assert.ok(changed[0].search.includes("новый источник"));
});
