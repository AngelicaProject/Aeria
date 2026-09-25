import assert from "node:assert/strict";
import test from "node:test";

import { nightlyVersion, updaterFeed } from "./version.mjs";

test("nightly builds precede the next stable release", () => {
  assert.equal(nightlyVersion("0.1.0", false, "7"), "0.1.0-nightly.7");
  assert.equal(nightlyVersion("0.1.0", true, "8"), "0.1.1-nightly.8");
  assert.equal(nightlyVersion("1.9.9", true, "12"), "1.9.10-nightly.12");
});

test("the updater feed names the Windows installer and its signature", () => {
  const feed = updaterFeed({
    version: "0.1.0",
    url: "https://github.com/AngelicaProject/Aeria/releases/download/v0.1.0/Aeria_0.1.0_x64-setup.exe",
    signature: "c2lnbmF0dXJl\n",
    notes: "",
    date: new Date(Date.UTC(2026, 8, 26, 12, 30, 15, 123)),
  });
  assert.deepEqual(feed, {
    version: "0.1.0",
    notes: "",
    pub_date: "2026-09-26T12:30:15Z",
    platforms: {
      "windows-x86_64": {
        signature: "c2lnbmF0dXJl",
        url: "https://github.com/AngelicaProject/Aeria/releases/download/v0.1.0/Aeria_0.1.0_x64-setup.exe",
      },
    },
  });
});
