import assert from "node:assert/strict";
import test from "node:test";
import { mergeVariables, variableCode, variableHint, variableLabel } from "../src/previewVariables.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const en = createTranslator("en");
const ru = createTranslator("ru");

const variable = (fields) => ({
  kind: "number",
  parameter: null,
  global: null,
  sheet: null,
  default: 1,
  value: 1,
  valueName: null,
  options: [],
  ...fields,
});

const classJob = variable({ key: "gn68", parameter: { prefix: "gn", index: 68 }, global: "class-job", sheet: "ClassJob", default: 19, value: 19 });
const row = variable({ key: "n1", parameter: { prefix: "n", index: 1 }, sheet: "Item" });

test("known globals read as their meaning with the code in the hint", () => {
  assert.equal(variableLabel(classJob, ru), "Класс");
  assert.equal(variableCode(classJob), "$gn68");
  assert.equal(variableHint(classJob, en), "The player’s current class or job, a row of the ClassJob sheet ($gn68)");
});

test("other variables read as their code, time values and characters as words", () => {
  assert.equal(variableLabel(row, en), "$n1");
  assert.equal(variableHint(row, en), "Number parameter: a number the game passes to this string when it shows it ($n1). Its value is a row of the Item sheet");
  assert.equal(variableLabel(variable({ key: "hour", kind: "time" }), ru), "час");
  assert.equal(variableCode(variable({ key: "hour", kind: "time" })), "$hour");
  const gender = variable({ key: "gender", kind: "character", default: 0, value: 0 });
  assert.equal(variableLabel(gender, ru), "Персонаж");
  assert.equal(variableCode(gender), null);
});

test("the variables of both panes are listed once, in order of first use", () => {
  const level = variable({ key: "gn72", parameter: { prefix: "gn", index: 72 }, global: "level" });
  assert.deepEqual(mergeVariables([classJob, level], null, [level, row]).map((entry) => entry.key), ["gn68", "gn72", "n1"]);
});
