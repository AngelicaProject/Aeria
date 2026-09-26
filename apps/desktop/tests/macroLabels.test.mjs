import assert from "node:assert/strict";
import test from "node:test";
import { choiceLabels, describeTag, parameterLabel, tagAt, valueLabel } from "../src/macroLabels.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const en = createTranslator("en");
const ru = createTranslator("ru");

test("parameters and values read as words", () => {
  assert.equal(parameterLabel({ prefix: "n", index: 1 }, en), "number 1");
  assert.equal(parameterLabel({ prefix: "gs", index: 2 }, ru), "глобальный текст 2");
  const style = { color: null, edge: null, italic: false, bold: false };
  assert.equal(valueLabel({ kind: "value", valueKind: "number", source: "num", parameter: { prefix: "n", index: 2 }, label: "$n2", style }, en), "number 2");
  assert.equal(valueLabel({ kind: "value", valueKind: "time", source: "num2", parameter: null, label: "$min", style }, ru), "минуты");
  assert.equal(valueLabel({ kind: "value", valueKind: "playerName", source: "player-name", parameter: { prefix: "n", index: 1 }, label: "$n1", style }, en), "player name (number 1)");
  assert.equal(valueLabel({ kind: "value", valueKind: "gameData", source: "sheet", parameter: { prefix: "n", index: 1 }, label: "Item $n1 0", style }, en), "Item · number 1");
  assert.equal(valueLabel({ kind: "value", valueKind: "gameData", source: "platform", parameter: null, label: "41", style }, en), "41");
});

test("choices label every branch", () => {
  assert.deepEqual(choiceLabels({ type: "if", condition: "($n1 == 1)" }, 2, en), ["when ($n1 == 1)", "otherwise"]);
  assert.deepEqual(choiceLabels({ type: "switch", value: "$weekday" }, 2, en), ["when $weekday is 1", "when $weekday is 2"]);
  assert.deepEqual(choiceLabels({ type: "gender" }, 2, ru), ["мужской персонаж", "женский персонаж"]);
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
    "row: $n1 — number 1",
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
