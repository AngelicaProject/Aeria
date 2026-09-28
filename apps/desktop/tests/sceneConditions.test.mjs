import assert from "node:assert/strict";
import test from "node:test";

import { guardText, negate } from "../src/sceneConditions.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const t = createTranslator("ru");
const quest = (name) => ({ kind: "quest", variable: "QUEST0", row: 1, name, sheet: null });
const completed = (name, value = true) => ({
  kind: "test",
  subject: { kind: "call", function: "IsQuestCompleted", arguments: [quest(name)] },
  test: { kind: "truthy", value },
});

test("a quest variable reads as the quest's name", () => {
  assert.equal(guardText(t, completed("Yes We Cant")), "квест «Yes We Cant» завершён");
  assert.equal(guardText(t, completed("Yes We Cant", false)), "квест «Yes We Cant» не завершён");
});

test("the same quest test joined over several quests is said once", () => {
  const any = { kind: "any", guards: [completed("A"), completed("B"), completed("C")] };
  assert.equal(guardText(t, any), "завершён один из квестов: «A», «B», «C»");
  assert.equal(guardText(t, negate(any)), "не завершён ни один из квестов: «A», «B», «C»");
  assert.equal(guardText(t, { kind: "all", guards: [completed("A"), completed("B")] }), "завершены все квесты: «A», «B»");
});

test("mixed conditions are joined with or and and", () => {
  const sex = { kind: "test", subject: { kind: "call", function: "GetSex", arguments: [] }, test: { kind: "compare", comparison: "eq", value: { kind: "number", value: 1 } } };
  assert.equal(guardText(t, { kind: "any", guards: [completed("A"), sex] }), "квест «A» завершён или пол игрока = 1");
  const isTrue = { ...completed("A"), test: { kind: "compare", comparison: "ne", value: { kind: "boolean", value: true } } };
  assert.equal(guardText(t, isTrue), "квест «A» не завершён", "≠ true reads as a negation");
  const reward = { kind: "test", subject: { kind: "call", function: "QuestReward", arguments: [] }, test: { kind: "truthy", value: false } };
  assert.equal(guardText(t, reward), "награда не получена");
  const unknown = { kind: "test", subject: { kind: "call", function: "IsMount", arguments: [] }, test: { kind: "truthy", value: false } };
  assert.equal(guardText(t, unknown), "не IsMount()");
});

test("one value compared with several reads as one of them", () => {
  const job = (name, comparison = "eq") => ({ kind: "test", subject: { kind: "call", function: "GetClassJob", arguments: [] }, test: { kind: "compare", comparison, value: { kind: "field", name } } });
  assert.equal(guardText(t, { kind: "any", guards: [job("CLASS_JOB_KNIGHT"), job("CLASS_JOB_MONK")] }), "класс или профессия игрока — одно из: CLASS_JOB_KNIGHT, CLASS_JOB_MONK");
  assert.equal(guardText(t, negate({ kind: "any", guards: [job("CLASS_JOB_KNIGHT"), job("CLASS_JOB_MONK")] })), "класс или профессия игрока — ни одно из: CLASS_JOB_KNIGHT, CLASS_JOB_MONK");
  assert.equal(guardText(t, { kind: "any", guards: [job("CLASS_JOB_KNIGHT"), job("CLASS_JOB_MONK", "ne")] }), "класс или профессия игрока = CLASS_JOB_KNIGHT или класс или профессия игрока ≠ CLASS_JOB_MONK");
});

test("a group inside another reads in brackets, with one value's comparisons merged", () => {
  const weapon = (slot, name) => ({ kind: "test", subject: { kind: "call", function: "GetEquippedItem", arguments: [{ kind: "field", name: slot }] }, test: { kind: "compare", comparison: "eq", value: { kind: "field", name } } });
  const relic = { kind: "any", guards: [
    { kind: "all", guards: [weapon("MAIN", "PALADIN_MAIN"), weapon("SUB", "PALADIN_SUB")] },
    weapon("MAIN", "MONK"),
    weapon("MAIN", "BARD"),
  ] };
  const enough = { kind: "test", subject: { kind: "field", name: "COST" }, test: { kind: "compare", comparison: "le", value: { kind: "call", function: "Count", arguments: [] } } };
  assert.equal(
    guardText(t, { kind: "all", guards: [relic, enough] }),
    "((GetEquippedItem(MAIN) = PALADIN_MAIN и GetEquippedItem(SUB) = PALADIN_SUB) или GetEquippedItem(MAIN) — одно из: MONK, BARD) и COST ≤ Count()",
  );
  const unknown = (value) => ({ kind: "test", subject: { kind: "unknown" }, test: { kind: "compare", comparison: "eq", value: { kind: "number", value } } });
  assert.equal(guardText(t, { kind: "any", guards: [unknown(1), unknown(2)] }), "значение, которое вычисляет скрипт = 1 или значение, которое вычисляет скрипт = 2", "values the script computes may differ");
});
