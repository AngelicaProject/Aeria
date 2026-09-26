import assert from "node:assert/strict";
import test from "node:test";
import { scanMacros, segmentMacroText } from "../src/macroTokens.ts";

test("macro scanner finds tags, separators, and closing tags", () => {
  const text = "<kilo $n1 \",\"> and <if ($gn77 == 3)><num $day><else>x</if></color>";
  const spans = scanMacros(text);
  assert.deepEqual(spans.map((span) => text.slice(span.from, span.to)), [
    "<kilo $n1 \",\">",
    "<if ($gn77 == 3)>",
    "<num $day>",
    "<else>",
    "</if>",
    "</color>",
  ]);
  assert.deepEqual(spans[1].names.map(([from, to]) => text.slice(from, to)), ["if"]);
  assert.deepEqual(spans[5].names.map(([from, to]) => text.slice(from, to)), ["/color"]);
});

test("comparison operators inside parentheses do not end a tag", () => {
  const text = "<if ($gn72 >= 94)>{320}<else><if ($n1 < 2)>a</if></if>";
  assert.deepEqual(scanMacros(text).map((span) => text.slice(span.from, span.to)), [
    "<if ($gn72 >= 94)>",
    "<else>",
    "<if ($n1 < 2)>",
    "</if>",
    "</if>",
  ]);
});

test("macro scanner reads tags nested in quoted arguments", () => {
  const text = "<kilo $n1 \"<platform 1>\">";
  const spans = scanMacros(text);
  assert.equal(spans.length, 1);
  assert.deepEqual(spans[0].names.map(([from, to]) => text.slice(from, to)), ["kilo", "platform"]);
});

test("macro scanner leaves comparisons, escapes, and unterminated input as text", () => {
  assert.deepEqual(scanMacros("1 < 2 and a\\<b"), []);
  assert.deepEqual(scanMacros("\\<num 1>"), []);
  assert.deepEqual(scanMacros("<num 1"), []);
});

test("segments reconstruct the original text", () => {
  const text = "Deal <num $n1> damage.<br>";
  const segments = segmentMacroText(text);
  assert.equal(segments.map((segment) => segment.text).join(""), text);
  assert.deepEqual(segments.map((segment) => segment.kind), ["text", "macro", "text", "macro"]);
});
