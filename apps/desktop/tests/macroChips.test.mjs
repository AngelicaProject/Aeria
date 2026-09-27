import assert from "node:assert/strict";
import test from "node:test";
import { chipSpecs } from "../src/macroChips.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const ru = createTranslator("ru");
const colors = (tag) => (tag === "<ui-color 504>" ? "#00cc22ff" : null);
const view = (text) => chipSpecs(text, colors, ru).map((spec) => {
  const source = text.slice(spec.from, spec.to);
  if (spec.kind === "chip") return `[${spec.label}]`;
  if (spec.kind === "marker") return `${spec.side === "open" ? "⟨" : "⟩"}${spec.color ?? ""}:${source}`;
  return `style(${source}|${spec.color ?? ""}${spec.italic ? " i" : ""}${spec.bold ? " b" : ""})`;
});

test("a color and its outline read as one styled span between two markers", () => {
  assert.deepEqual(view("a<br><ui-color 504><ui-edge-color 505>Duration:</ui-edge-color></ui-color> 10s"), [
    "[↵]",
    "⟨#00cc22ff:<ui-color 504><ui-edge-color 505>",
    "style(Duration:|#00cc22ff)",
    "⟩:</ui-edge-color></ui-color>",
  ]);
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

test("conditions, values, icons, and unknown codes are chips", () => {
  const specs = chipSpecs("<if ($gn68 == 19)>{220}<else>{150}</if> <sheet Item $n1 0> <icon2 10> <code:35 1>", colors, ru);
  assert.deepEqual(specs.map((spec) => [spec.label, spec.tone, spec.icon]), [
    ["если ($gn68 == 19)", "condition", undefined],
    ["иначе", "condition", undefined],
    ["конец", "condition", undefined],
    ["Item · $n1", "value", undefined],
    ["10", "icon", 10],
    ["<code:35 1>", "unknown", undefined],
  ]);
});
