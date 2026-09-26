import assert from "node:assert/strict";
import test from "node:test";

import { MAX_IMAGE_BYTES, canSendAsIs, fitImageSize, transferredImages } from "../src/imageAttachments.ts";

test("images shrink to the side and pixel limits and are never enlarged", () => {
  assert.deepEqual(fitImageSize(800, 600), { width: 800, height: 600 });
  assert.deepEqual(fitImageSize(1920, 1080), { width: 1920, height: 1080 });
  assert.deepEqual(fitImageSize(3840, 2160), { width: 1920, height: 1080 });
  // A tall capture is limited by its long side first.
  assert.deepEqual(fitImageSize(400, 8000), { width: 102, height: 2048 });
});

test("only PNG and JPEG files within the limits are sent as they are", () => {
  assert.equal(canSendAsIs("image/png", 200_000, 1280, 720), true);
  assert.equal(canSendAsIs("image/jpeg", 200_000, 1280, 720), true);
  assert.equal(canSendAsIs("image/webp", 200_000, 1280, 720), false);
  assert.equal(canSendAsIs("image/png", MAX_IMAGE_BYTES + 1, 1280, 720), false);
  assert.equal(canSendAsIs("image/png", 200_000, 2560, 1440), false);
});

function transfer(files, text = "") {
  return {
    types: text ? ["text/plain", "Files"] : ["Files"],
    getData: (format) => format === "text/plain" ? text : "",
    files,
  };
}

test("pasted images attach unless the clipboard also carries text", () => {
  const image = { type: "image/png", name: "shot.png" };
  const other = { type: "application/pdf", name: "doc.pdf" };
  assert.deepEqual(transferredImages(transfer([image, other])), [image]);
  assert.deepEqual(transferredImages(transfer([image], "A1\tB1")), []);
  assert.deepEqual(transferredImages(transfer([image], "  ")), [image]);
  assert.deepEqual(transferredImages(transfer([])), []);
});
