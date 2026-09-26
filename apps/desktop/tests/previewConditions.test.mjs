import assert from "node:assert/strict";
import test from "node:test";
import { chosenBranch, conditionVariables, effectiveValues } from "../src/previewConditions.ts";

const style = { color: null, edge: null, italic: false, bold: false };
const text = (value) => ({ kind: "text", text: value, style });
const param = (prefix, index) => ({ type: "parameter", prefix, index });
const int = (value) => ({ type: "int", value });
const compare = (operator, left, right) => ({ type: "compare", operator, left, right });
const choice = (test, then, otherwise) => ({
  kind: "choice",
  choice: { type: "if", condition: "…", test },
  branches: [then, otherwise],
});

// <if ($gn68 == 20)><if ($gn72 >= 94)>{320}<else>{280}</if><else>{240}</if>
const potency = [choice(
  compare("==", param("gn", 68), int(20)),
  [choice(compare(">=", param("gn", 72), int(94)), [text("320")], [text("280")])],
  [text("240")],
)];

function shown(pieces, values) {
  let out = "";
  for (const piece of pieces) {
    if (piece.kind === "text") out += piece.text;
    if (piece.kind === "choice") {
      const index = chosenBranch(piece.choice, piece.branches.length, values);
      if (index !== null && index >= 0) out += shown(piece.branches[index], values);
    }
  }
  return out;
}

test("variables are collected from nested conditions with values that show the first branches", () => {
  const variables = conditionVariables(potency);
  assert.deepEqual(variables.map((variable) => [variable.key, variable.fallback]), [["gn68", 20], ["gn72", 94]]);
  assert.equal(shown(potency, effectiveValues(variables, {})), "320");
});

test("nested conditions follow the values as in the game", () => {
  const variables = conditionVariables(potency);
  assert.equal(shown(potency, effectiveValues(variables, { gn72: 90 })), "280");
  assert.equal(shown(potency, effectiveValues(variables, { gn68: 19, gn72: 100 })), "240");
});

test("comparisons and switches evaluate every operator", () => {
  const values = { n1: 5 };
  for (const [operator, expected] of [["==", 1], ["!=", 0], ["<", 1], ["<=", 1], [">", 0], [">=", 0]]) {
    assert.equal(chosenBranch({ type: "if", condition: "", test: compare(operator, param("n", 1), int(4)) }, 2, values), expected, operator);
  }
  assert.equal(chosenBranch({ type: "if", condition: "", test: { type: "value", operand: param("n", 1) } }, 2, { n1: 0 }), 1);
  assert.equal(chosenBranch({ type: "if", condition: "", test: { type: "other" } }, 2, values), null);
  assert.equal(chosenBranch({ type: "switch", value: "$n1", selector: param("n", 1) }, 7, values), 4);
  assert.equal(chosenBranch({ type: "switch", value: "$n1", selector: param("n", 1) }, 3, values), -1);
  assert.equal(chosenBranch({ type: "gender" }, 2, { gender: 1 }), 1);
  assert.equal(chosenBranch({ type: "myself" }, 2, { self: 1 }), 0);
});

test("a right-hand variable gets a value that satisfies the mirrored comparison", () => {
  const variables = conditionVariables([choice(compare("<", int(10), param("n", 2)), [text("a")], [text("b")])]);
  assert.deepEqual(variables.map((variable) => [variable.key, variable.fallback]), [["n2", 11]]);
});
