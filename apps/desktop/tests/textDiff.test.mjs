import assert from "node:assert/strict";
import test from "node:test";
import { diffWords } from "../src/textDiff.ts";

test("word diff marks replaced words and keeps macros atomic", () => {
  assert.deepEqual(diffWords("Deal <num $n1> damage", "Deal <num $n1> heavy damage"), [
    { kind: "same", text: "Deal <num $n1> " },
    { kind: "added", text: "heavy " },
    { kind: "same", text: "damage" },
  ]);
  assert.deepEqual(diffWords("Отменить", "Отмена"), [
    { kind: "removed", text: "Отменить" },
    { kind: "added", text: "Отмена" },
  ]);
  assert.deepEqual(diffWords("", "new"), [{ kind: "added", text: "new" }]);
  assert.deepEqual(diffWords("same", "same"), [{ kind: "same", text: "same" }]);
});

test("diff parts reconstruct both sides", () => {
  const before = "The quick <color #FF0000FF>brown</color> fox";
  const after = "A quick <color #FF00FF00>brown</color> dog";
  const parts = diffWords(before, after);
  assert.equal(parts.filter((part) => part.kind !== "added").map((part) => part.text).join(""), before);
  assert.equal(parts.filter((part) => part.kind !== "removed").map((part) => part.text).join(""), after);
});
