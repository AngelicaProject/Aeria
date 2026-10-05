import assert from "node:assert/strict";
import test from "node:test";
import { formatKind, guideMacros, macroState } from "../src/stringGuide.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const ru = createTranslator("ru");
const lookup = { colorOf: () => null, conditionOf: () => null };

/** Tags as `macro_view` reports them, with the rule the structure policy gives each. */
function tag(text, spelling, rule, part = "inline", name = spelling.slice(1).split(/[ >]/)[0]) {
  const from = text.indexOf(spelling);
  return { from, to: from + spelling.length, name, part, family: null, args: [], color: null, condition: null, rule };
}

test("the guide groups the source's parts by what a translation may do, strictest first", () => {
  const text = "<i>Take</i> <sheet Item $n1 0><br>to <string $gs1>.<br>";
  const tags = [
    tag(text, "<i>", "formatting", "open"),
    tag(text, "</i>", "formatting", "close", "i"),
    tag(text, "<sheet Item $n1 0>", "keep"),
    { ...tag(text, "<br>", "free"), from: text.indexOf("<br>"), to: text.indexOf("<br>") + 4 },
    tag(text, "<string $gs1>", "keep"),
    { ...tag(text, "<br>", "free"), from: text.lastIndexOf("<br>"), to: text.lastIndexOf("<br>") + 4 },
  ];
  const macros = guideMacros(text, tags, lookup, ru);
  assert.deepEqual(macros.map((macro) => [macro.rule, macro.label, macro.count]), [
    ["keep", "Item · $n1", 1],
    ["keep", "$gs1", 1],
    ["formatting", "курсив", 1],
    ["free", "↵", 2],
  ]);
  assert.deepEqual(macros[0].data, ["<sheet Item $n1 0>"]);
  assert.deepEqual(macros[2].pick, { wrap: ["<i>", "</i>"] });
  assert.deepEqual(macros[1].pick, { insert: "<string $gs1>" });
});

test("a condition is one part, without its separators and end", () => {
  const text = "<if $gn4>She<else>He</if> waits.";
  const tags = [
    tag(text, "<if $gn4>", "condition", "open", "if"),
    tag(text, "<else>", "condition", "separator", "if"),
    tag(text, "</if>", "condition", "close", "if"),
  ];
  const macros = guideMacros(text, tags, lookup, ru);
  assert.equal(macros.length, 1);
  assert.equal(macros[0].rule, "condition");
  assert.deepEqual(macros[0].pick, { insert: "<if $gn4>She<else>He</if>" });
});

test("game data a translation lacks is marked missing", () => {
  const keep = { rule: "keep", tone: "value", label: "Item · $n1", pick: { insert: "" }, data: ["<sheet Item $n1 0>"], count: 1 };
  assert.equal(macroState(keep, ["<sheet Item $n1 0>"]), "missing");
  assert.equal(macroState(keep, []), "present");
  assert.equal(macroState({ ...keep, rule: "free", data: [] }, []), null);
  assert.equal(formatKind("<ui-color 504><ui-edge-color 505>"), "color");
  assert.equal(formatKind("<b>"), "bold");
});
