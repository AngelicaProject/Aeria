import assert from "node:assert/strict";
import test from "node:test";
import { scanMacros, segmentMacroText } from "../src/macroTokens.ts";

test("macro scanner finds flat, nested, and closing macros", () => {
  const text = "<kilo(lnum1,\\,)> and <if([gnum77==3],<num(t_day)>,x)></color>";
  const spans = scanMacros(text);
  assert.deepEqual(spans.map((span) => text.slice(span.from, span.to)), [
    "<kilo(lnum1,\\,)>",
    "<if([gnum77==3],<num(t_day)>,x)>",
    "</color>",
  ]);
  assert.deepEqual(spans[1].names.map(([from, to]) => text.slice(from, to)), ["if", "num"]);
  assert.deepEqual(spans[2].names.map(([from, to]) => text.slice(from, to)), ["/color"]);
});

test("macro scanner leaves comparisons, escapes, and unterminated input as text", () => {
  assert.deepEqual(scanMacros("1 < 2 and a<b"), []);
  assert.deepEqual(scanMacros("\\<num(1)>"), []);
  assert.deepEqual(scanMacros("<num(1"), []);
});

test("segments reconstruct the original text", () => {
  const text = "Deal <num(lnum1)> damage.<br>";
  const segments = segmentMacroText(text);
  assert.equal(segments.map((segment) => segment.text).join(""), text);
  assert.deepEqual(segments.map((segment) => segment.kind), ["text", "macro", "text", "macro"]);
});
