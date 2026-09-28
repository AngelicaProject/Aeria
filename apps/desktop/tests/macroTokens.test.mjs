import assert from "node:assert/strict";
import test from "node:test";
import { scanMacros, segmentMacroText, speakerMarkers } from "../src/macroTokens.ts";

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

test("a speaker name at the start of a line is syntax", () => {
  const text = "(-???-)Well, this is a surprise.";
  assert.deepEqual(speakerMarkers(text), { open: { from: 0, to: 2 }, close: { from: 5, to: 7 } });
  assert.deepEqual(segmentMacroText(text), [
    { kind: "macro", text: "(-" },
    { kind: "text", text: "???" },
    { kind: "macro", text: "-)" },
    { kind: "text", text: "Well, this is a surprise." },
  ]);
  const titled = "(-<i>An Introduction</i>-)Chapter one.";
  assert.equal(titled.slice(speakerMarkers(titled).close.from), "-)Chapter one.", "the name may hold tags");
  assert.equal(speakerMarkers("Damage (-<num $n2>%)"), null, "only at the start");
  assert.equal(speakerMarkers("(-no end"), null);
  assert.equal(speakerMarkers("(-)"), null);
});

test("an idiom reads as one segment that keeps its macro text", () => {
  const idioms = new Map([["<split \" \" 1><string $gs1></split>", { name: "player-first-name", summary: "the first name of the player character" }]]);
  const text = "You! <split \" \" 1><string $gs1></split> or somethin'?";
  assert.deepEqual(segmentMacroText(text, idioms), [
    { kind: "text", text: "You! " },
    { kind: "idiom", text: "<split \" \" 1><string $gs1></split>", name: "player-first-name", summary: "the first name of the player character" },
    { kind: "text", text: " or somethin'?" },
  ]);
  assert.equal(segmentMacroText(text).filter((segment) => segment.kind === "macro").length, 3, "without idioms the tags stay tags");
});
