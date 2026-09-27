import assert from "node:assert/strict";
import test from "node:test";
import { describeTag, parameterLabel, tagAt } from "../src/macroLabels.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const en = createTranslator("en");
const ru = createTranslator("ru");

test("parameters read as words", () => {
  assert.equal(parameterLabel({ prefix: "n", index: 1 }, en), "number parameter 1 of the string");
  assert.equal(parameterLabel({ prefix: "gn", index: 68 }, ru), "глобальная переменная игры 68 (число)");
});

test("tags describe themselves and their arguments", () => {
  const sheet = {
    from: 0,
    to: 18,
    name: "sheet",
    part: "inline",
    family: "gameData",
    args: [
      { name: "sheet", role: "sheet", value: "Item", parameter: null },
      { name: "row", role: "row", value: "$n1", parameter: { prefix: "n", index: 1 } },
    ],
  };
  const described = describeTag(sheet, en);
  assert.equal(described.title, "<sheet>");
  assert.deepEqual(described.lines, [
    "A value from a game data sheet, such as a name.",
    "sheet: Item",
    "row: $n1 — number parameter 1 of the string",
    "Game data",
  ]);
  assert.equal(describeTag({ ...sheet, name: "i", part: "close", family: "formatting", args: [] }, ru).title, "</i>");
  assert.equal(describeTag({ ...sheet, name: "code:35", part: "generic", family: null, args: [] }, en).lines[0], "A macro code Aeria does not know. It is kept exactly.");
});

test("the innermost tag under a position wins", () => {
  const outer = { from: 0, to: 30, name: "kilo", part: "inline", family: "runtimeValue", args: [] };
  const inner = { from: 10, to: 22, name: "platform", part: "inline", family: "gameData", args: [] };
  assert.equal(tagAt([outer, inner], 12)?.name, "platform");
  assert.equal(tagAt([outer, inner], 3)?.name, "kilo");
  assert.equal(tagAt([outer, inner], 40), null);
});
