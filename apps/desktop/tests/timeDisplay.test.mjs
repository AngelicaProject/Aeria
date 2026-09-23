import assert from "node:assert/strict";
import test from "node:test";
import { formatRelativeTime } from "../src/timeDisplay.ts";

test("relative time uses the coarsest whole unit", () => {
  const now = Date.UTC(2026, 8, 23, 12);
  assert.equal(formatRelativeTime(now - 20_000, now, "en"), "just now");
  assert.equal(formatRelativeTime(now - 5 * 60_000, now, "en"), "5 minutes ago");
  assert.equal(formatRelativeTime(now - 26 * 3600_000, now, "en"), "yesterday");
  assert.equal(formatRelativeTime(now - 3 * 24 * 3600_000, now, "en"), "3 days ago");
});
