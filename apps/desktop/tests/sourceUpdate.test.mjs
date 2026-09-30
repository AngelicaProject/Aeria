import assert from "node:assert/strict";
import test from "node:test";

import { sourceUpdateFacts } from "../src/sourceUpdate.ts";

function report(overrides = {}) {
  return {
    previousGameVersion: "2026.08.01.0000.0000",
    gameVersion: "2026.09.01.0000.0000",
    files: 0,
    fuzzy: 0,
    obsolete: 0,
    commit: null,
    ...overrides,
  };
}

test("update facts list only non-empty outcomes, translations to check first", () => {
  const facts = sourceUpdateFacts(report({ files: 12, fuzzy: 3, obsolete: 1 }));
  assert.deepEqual(
    facts.map((fact) => [fact.key, fact.count, fact.tone]),
    [
      ["sourceUpdate.fact.fuzzy", 3, "attention"],
      ["sourceUpdate.fact.obsolete", 1, "attention"],
      ["sourceUpdate.fact.files", 12, "neutral"],
    ],
  );
  assert.deepEqual(sourceUpdateFacts(report()), []);
});
