import assert from "node:assert/strict";
import { mock, test } from "node:test";

import { throttled } from "../src/throttle.ts";

test("a stream of triggers runs at most once an interval, and the last one always", () => {
  mock.timers.enable({ apis: ["setTimeout", "Date"] });
  try {
    let runs = 0;
    const throttle = throttled(() => { runs += 1; }, 3000);
    throttle.trigger();
    mock.timers.tick(0);
    assert.equal(runs, 1);
    // Triggers within the interval wait for it, together.
    throttle.trigger();
    mock.timers.tick(1500);
    throttle.trigger();
    assert.equal(runs, 1);
    mock.timers.tick(1500);
    assert.equal(runs, 2);
    // After a quiet interval a trigger runs at once.
    mock.timers.tick(5000);
    throttle.trigger();
    mock.timers.tick(0);
    assert.equal(runs, 3);
    throttle.trigger();
    throttle.cancel();
    mock.timers.tick(5000);
    assert.equal(runs, 3);
  } finally {
    mock.timers.reset();
  }
});
