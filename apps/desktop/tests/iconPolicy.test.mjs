import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const sourceRoot = fileURLToPath(new URL("../src", import.meta.url));
const sharedIconLayer = join(sourceRoot, "ui", "primitives", "UiIcon.tsx");
const legacyGlyphNode = />\s*(?:\+|×|✕|✖|➕|➖|⋯|…|←|→|↑|↓|⌄|⌃|⌵|▸|▾|▶|▼|◀|▲|•|●|○|✓|✔|✗|❌|❎|⚙|⚠|ℹ|[\p{Extended_Pictographic}\uFE0F\u200D])\s*</u;

function collectRendererSources(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return collectRendererSources(path);
    return /\.(?:tsx|css)$/i.test(entry.name) ? [path] : [];
  });
}

test("renderer UI icons use the shared icon layer", () => {
  const sources = collectRendererSources(sourceRoot);
  assert.ok(sources.includes(sharedIconLayer), "shared UiIcon implementation must exist");

  for (const path of sources) {
    const source = readFileSync(path, "utf8");
    const label = relative(sourceRoot, path).split(sep).join("/");

    if (path !== sharedIconLayer) {
      assert.doesNotMatch(source, /<svg\b/i, `${label} must use UiIcon instead of inline SVG`);
    }
    assert.doesNotMatch(source, legacyGlyphNode, `${label} must not use a standalone glyph as an icon`);

    if (path.endsWith(".css")) {
      assert.doesNotMatch(source, /(?:^|[;{])\s*content\s*:\s*(?:"[^"]+"|'[^']+')/im, `${label} must not use CSS text as an icon`);
      assert.doesNotMatch(source, /data:image\/svg\+xml/i, `${label} must not embed SVG icons in CSS`);
    }
  }
});

test("renderer dropdowns use the shared Select instead of native selects", () => {
  for (const path of collectRendererSources(sourceRoot).filter((file) => file.endsWith(".tsx"))) {
    const label = relative(sourceRoot, path).split(sep).join("/");
    assert.doesNotMatch(readFileSync(path, "utf8"), /<select[\s>]/, `${label} must use ui/primitives/Select instead of <select>`);
  }
});
