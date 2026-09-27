import assert from "node:assert/strict";
import test from "node:test";
import { chipSpecs, conditionLabel } from "../src/macroChips.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const ru = createTranslator("ru");
const conditions = {
  "<if ($gn68 == 21)>": { left: { kind: "parameter", code: "$gn68", meaning: "class-job" }, operator: "==", right: { kind: "int", value: 21, name: "monk" } },
  "<if ($gn72 >= 94)>": { left: { kind: "parameter", code: "$gn72", meaning: "level" }, operator: ">=", right: { kind: "int", value: 94, name: null } },
};
const lookup = {
  colorOf: (tag) => (tag === "<ui-color 504>" ? "#00cc22ff" : null),
  conditionOf: (tag) => conditions[tag] ?? null,
};
const view = (text) => chipSpecs(text, lookup, ru).map((spec) => {
  if (spec.kind === "break") return `⏎${"·".repeat(spec.indent)}`;
  const source = text.slice(spec.from, spec.to);
  if (spec.kind === "chip") return `[${spec.label}]`;
  if (spec.kind === "marker") return `${spec.side === "open" ? "⟨" : "⟩"}${spec.color ?? ""}:${source}`;
  return `style(${source}|${spec.color ?? ""}${spec.italic ? " i" : ""}${spec.bold ? " b" : ""})`;
});

test("a color and its outline read as one styled span between two markers", () => {
  assert.deepEqual(view("a<br><ui-color 504><ui-edge-color 505>Duration:</ui-edge-color></ui-color> 10s"), [
    "[↵]",
    "⏎",
    "⟨#00cc22ff:<ui-color 504><ui-edge-color 505>",
    "style(Duration:|#00cc22ff)",
    "⟩:</ui-edge-color></ui-color>",
  ]);
});

test("a line break at the end still starts a line for the cursor", () => {
  assert.deepEqual(view("a<br>"), ["[↵]", "⏎"]);
  assert.deepEqual(chipSpecs("a<br>", lookup, ru).find((spec) => spec.kind === "break"), { kind: "break", at: 5, indent: 0, assoc: 1 });
});

test("nested formatting keeps the innermost color and adds italics", () => {
  assert.deepEqual(view("<ui-color 504>a<i>b</i></ui-color>"), [
    "⟨#00cc22ff:<ui-color 504>",
    "style(a|#00cc22ff)",
    "⟨:<i>",
    "style(b|#00cc22ff i)",
    "⟩:</i></ui-color>",
  ]);
});

test("conditions read in words and value branches read as their values", () => {
  assert.equal(conditionLabel(conditions["<if ($gn68 == 21)>"], ru), "класс = monk");
  assert.equal(conditionLabel({ left: { kind: "parameter", code: "$gn4", meaning: "player-female" }, operator: null, right: null }, ru), "игрок — женщина");
  assert.deepEqual(view("a <if-gender $n1>his<else>her</if-gender> {5}"), ["[если мужской]", "[иначе]", "[конец]", "[5]"]);
});

test("a condition holding another starts lines, and simple ones stay inline", () => {
  const text = "Сила <if ($gn68 == 21)><if ($gn72 >= 94)>{240}<else>{200}</if><else>{150}</if>.";
  assert.deepEqual(view(text), [
    "⏎",
    "[если класс = monk]",
    "[если уровень ≥ 94]",
    "[240]",
    "[иначе]",
    "[200]",
    "[конец]",
    "⏎",
    "[иначе]",
    "[150]",
    "⏎",
    "[конец]",
    "⏎",
  ]);
  // Three levels: the middle condition starts lines one level in.
  const deep = "<if ($gn68 == 21)>a<if ($gn72 >= 94)>b<if ($gn72 >= 50)>c</if></if></if>";
  assert.deepEqual(view(deep).filter((part) => part.startsWith("⏎")), ["⏎·", "⏎·", "⏎"]);
});

test("picking a chip adds its tag, a condition its block, and a marker its pair", () => {
  const text = "<if ($gn72 >= 94)>a<else>b</if> <ui-color 504><ui-edge-color 505>x</ui-edge-color></ui-color> <sheet Item $n1 0>";
  const specs = chipSpecs(text, lookup, ru);
  const chips = specs.filter((spec) => spec.kind === "chip");
  assert.equal(chips[0].insert, "<if ($gn72 >= 94)>a<else>b</if>");
  assert.equal(chips.at(-1).insert, "<sheet Item $n1 0>");
  const markers = specs.filter((spec) => spec.kind === "marker");
  assert.deepEqual(markers[0].wrap, ["<ui-color 504><ui-edge-color 505>", "</ui-edge-color></ui-color>"]);
  assert.deepEqual(markers[1].wrap, markers[0].wrap);
  assert.deepEqual(chipSpecs("<i>a", lookup, ru).find((spec) => spec.kind === "marker").wrap, ["<i>", "</i>"]);
});
