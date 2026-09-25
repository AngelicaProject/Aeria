import assert from "node:assert/strict";
import test from "node:test";
import { layoutGraph } from "../src/commitGraph.ts";

test("a linear history stays in one lane", () => {
  const rows = layoutGraph([
    { id: "c", parents: ["b"] },
    { id: "b", parents: ["a"] },
    { id: "a", parents: [] },
  ]);
  assert.deepEqual(rows.map((row) => row.column), [0, 0, 0]);
  assert.deepEqual(rows.map((row) => row.outgoing), [[0], [0], []]);
  assert.deepEqual(rows.map((row) => row.incoming), [[], [0], [0]]);
});

test("a merge opens a lane for the second parent and closes it at the fork", () => {
  // m merges f (feature) into c (main); both come from a.
  const rows = layoutGraph([
    { id: "m", parents: ["c", "f"] },
    { id: "f", parents: ["a"] },
    { id: "c", parents: ["a"] },
    { id: "a", parents: [] },
  ]);
  assert.deepEqual(rows[0], { column: 0, through: [], incoming: [], outgoing: [0, 1], width: 2 });
  assert.equal(rows[1].column, 1);
  assert.deepEqual(rows[1].through, [0]);
  // f's parent a gets its own lane; c then joins the same a.
  assert.equal(rows[2].column, 0);
  assert.deepEqual(rows[2].through, [1]);
  assert.deepEqual(rows[2].outgoing, [1]);
  assert.equal(rows[3].column, 1);
  assert.deepEqual(rows[3].incoming, [1]);
});

test("a page that ends mid-history keeps its open lanes", () => {
  const rows = layoutGraph([{ id: "b", parents: ["a"] }]);
  assert.deepEqual(rows[0].outgoing, [0]);
});
