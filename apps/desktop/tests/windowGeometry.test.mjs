import assert from "node:assert/strict";
import test from "node:test";

import { fitToMonitor, usableGeometry } from "../src/windowGeometry.ts";

const minimum = { width: 1140, height: 710 };

test("a size saved while minimized or too small is not restored", () => {
  assert.equal(usableGeometry({ width: 160, height: 28, maximized: false }, minimum), null);
  assert.equal(usableGeometry({ width: 1400, height: 500, maximized: false }, minimum), null);
  assert.deepEqual(usableGeometry({ width: 1400, height: 900, maximized: false }, minimum), { width: 1400, height: 900, maximized: false });
  assert.deepEqual(usableGeometry({ width: 1140, height: 710, maximized: true }, minimum), { width: 1140, height: 710, maximized: true });
  assert.equal(usableGeometry({ width: "1400", height: 900, maximized: false }, minimum), null);
  assert.equal(usableGeometry(null, minimum), null);
});

test("a restored size fits the monitor", () => {
  const saved = { width: 2560, height: 1400, maximized: false };
  assert.deepEqual(fitToMonitor(saved, { width: 1920, height: 1080 }), { width: 1920, height: 1080 });
  assert.deepEqual(fitToMonitor(saved, null), { width: 2560, height: 1400 });
});
