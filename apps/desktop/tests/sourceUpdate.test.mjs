import assert from "node:assert/strict";
import test from "node:test";

import { en } from "../src/i18n/en.ts";
import { createTranslator } from "../src/i18n/translate.ts";
import { detachReasonLabels, migratesWorkspaceFormat, sourceUpdateFacts } from "../src/sourceUpdate.ts";

function report(overrides = {}) {
  return {
    previousContentId: "sha256:old",
    contentId: "sha256:new",
    gameVersion: "2026.09.01.0000.0000",
    previousFormatVersion: 2,
    unchanged: 0,
    encodingChanged: 0,
    sourceChanged: 0,
    detached: 0,
    newlyDetached: 0,
    reattached: 0,
    columnMapped: 0,
    rowMoved: 0,
    changedUnits: 0,
    sheetSchemaUpdates: [],
    ...overrides,
  };
}

test("source update facts list only non-empty outcomes in reading order", () => {
  const facts = sourceUpdateFacts(report({ unchanged: 10, encodingChanged: 3, sourceChanged: 2, rowMoved: 5, detached: 3, newlyDetached: 1, columnMapped: 4 }), false);
  assert.deepEqual(
    facts.map((fact) => [fact.key, fact.count, fact.tone]),
    [
      ["sourceUpdate.fact.unchanged", 10, "neutral"],
      ["sourceUpdate.fact.encodingChanged", 3, "neutral"],
      ["sourceUpdate.fact.sourceChanged", 2, "attention"],
      ["sourceUpdate.fact.rowMoved", 5, "neutral"],
      ["sourceUpdate.fact.columnMapped", 4, "neutral"],
      ["sourceUpdate.fact.newlyDetached", 1, "attention"],
      ["sourceUpdate.fact.stillDetached", 2, "neutral"],
    ],
  );
  assert.deepEqual(sourceUpdateFacts(report(), false), []);
  assert.equal(
    sourceUpdateFacts(report({ detached: 1, newlyDetached: 1 }), true)[0].key,
    "sourceUpdate.fact.wereDetached",
  );
});

test("a Workspace Format v1 project is reported as a format migration", () => {
  assert.equal(migratesWorkspaceFormat(report({ previousFormatVersion: 1 })), true);
  assert.equal(migratesWorkspaceFormat(report()), false);
});

test("every detach reason and fact has an interface message", () => {
  const t = createTranslator("ru");
  for (const key of Object.values(detachReasonLabels)) {
    assert.ok(key in en, key);
    assert.ok(t(key).length > 0);
  }
  const every = report({ unchanged: 1, encodingChanged: 1, sourceChanged: 1, rowMoved: 1, columnMapped: 1, reattached: 1, detached: 2, newlyDetached: 1 });
  for (const fact of [...sourceUpdateFacts(every, false), ...sourceUpdateFacts(every, true)]) {
    assert.ok(fact.key in en, fact.key);
  }
  assert.equal(t("sourceUpdate.fact.sourceChanged", { count: 3 }), "3 перевода требуют проверки: изменился исходный текст");
});
