import assert from "node:assert/strict";
import test from "node:test";
import { insertionPick } from "../src/macroInsertions.ts";

const insertion = (form, parts) => ({ name: "x", group: "choice", form, parts, rows: [], summary: "" });

test("an insertion becomes what the editor applies", () => {
  assert.deepEqual(insertionPick(insertion("insert", ["<string $gs1>"])), { insert: "<string $gs1>" });
  assert.deepEqual(insertionPick(insertion("wrap", ["<i>", "</i>"])), { wrap: ["<i>", "</i>"] });
  assert.deepEqual(insertionPick(insertion("branches", ["<if $gn4>", "<else>", "</if>"])), { branches: ["<if $gn4>", "<else>", "</if>"] });
  assert.deepEqual(
    insertionPick(insertion("branches", ["<if ($gn71 == {row})>", "<else>", "</if>"]), 3),
    { branches: ["<if ($gn71 == 3)>", "<else>", "</if>"] },
    "a row fills its place",
  );
});
