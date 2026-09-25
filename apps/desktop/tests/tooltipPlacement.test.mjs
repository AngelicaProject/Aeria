import assert from "node:assert/strict";
import test from "node:test";

import { placeTooltip } from "../src/ui/tooltipPlacement.ts";

const viewport = { width: 800, height: 600 };

test("tooltips sit centered below their anchor", () => {
  assert.deepEqual(placeTooltip({ left: 100, top: 100, width: 40, height: 20 }, { width: 60, height: 24 }, viewport), { left: 90, top: 126, side: "bottom" });
});

test("tooltips flip above near the bottom and stay inside the viewport", () => {
  assert.deepEqual(placeTooltip({ left: 780, top: 580, width: 16, height: 16 }, { width: 120, height: 24 }, viewport), { left: 672, top: 550, side: "top" });
  assert.equal(placeTooltip({ left: -10, top: 10, width: 10, height: 10 }, { width: 100, height: 20 }, viewport).left, 8);
});
