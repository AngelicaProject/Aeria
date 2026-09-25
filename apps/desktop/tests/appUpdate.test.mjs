import assert from "node:assert/strict";
import test from "node:test";

import { awaitsNewerStable, downloadShare, installBlockers, shouldOfferUpdate } from "../src/appUpdate.ts";

function status(overrides = {}) {
  return {
    currentVersion: "0.1.0",
    channel: "stable",
    canInstall: true,
    checking: false,
    lastCheckedAt: null,
    available: null,
    download: null,
    installing: false,
    error: null,
    runningActivities: [],
    ...overrides,
  };
}

const offered = { version: "0.1.1", channel: "stable", notes: null, publishedAt: null, releaseUrl: "https://example.invalid" };

test("an unsaved draft and running work hold back installation", () => {
  assert.deepEqual(installBlockers(status(), false), []);
  assert.deepEqual(installBlockers(status({ runningActivities: ["sync", "export"] }), true), ["unsavedDraft", "sync", "export"]);
});

test("download share is unknown until the size is announced", () => {
  assert.equal(downloadShare(null), null);
  assert.equal(downloadShare({ received: 10, total: null, ready: false }), null);
  assert.equal(downloadShare({ received: 25, total: 100, ready: false }), 0.25);
  assert.equal(downloadShare({ received: 0, total: null, ready: true }), 1);
});

test("a postponed version is not offered again until a newer one appears", () => {
  assert.equal(shouldOfferUpdate(null, null), false);
  assert.equal(shouldOfferUpdate(status(), null), false);
  assert.equal(shouldOfferUpdate(status({ available: offered }), null), true);
  assert.equal(shouldOfferUpdate(status({ available: offered }), "0.1.1"), false);
  assert.equal(shouldOfferUpdate(status({ available: { ...offered, version: "0.1.2" } }), "0.1.1"), true);
});

test("a nightly build on the stable channel waits for a newer stable release", () => {
  assert.equal(awaitsNewerStable(status({ currentVersion: "0.1.1-nightly.4" })), true);
  assert.equal(awaitsNewerStable(status({ currentVersion: "0.1.1-nightly.4", channel: "nightly" })), false);
  assert.equal(awaitsNewerStable(status({ currentVersion: "0.1.1-nightly.4", available: offered })), false);
  assert.equal(awaitsNewerStable(status()), false);
});
