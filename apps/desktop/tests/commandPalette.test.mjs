import assert from "node:assert/strict";
import test from "node:test";
import { parsePaletteQuery, parseRowTarget } from "../src/commandPalette.ts";

test("palette prefixes select a mode", () => {
  assert.deepEqual(parsePaletteQuery("Addon"), { mode: "sheets", term: "Addon" });
  assert.deepEqual(parsePaletteQuery(">  close"), { mode: "commands", term: "close" });
  assert.deepEqual(parsePaletteQuery(":12"), { mode: "goto", term: "12" });
  assert.deepEqual(parsePaletteQuery("#hello"), { mode: "strings", term: "hello" });
});

test("row targets accept row, subrow, and column", () => {
  assert.deepEqual(parseRowTarget("42"), { rowId: 42, subrowId: 0, columnIndex: null });
  assert.deepEqual(parseRowTarget("42:3"), { rowId: 42, subrowId: 3, columnIndex: null });
  assert.deepEqual(parseRowTarget("42:3:7"), { rowId: 42, subrowId: 3, columnIndex: 7 });
  assert.equal(parseRowTarget("abc"), null);
  assert.equal(parseRowTarget("1:70000"), null);
});
