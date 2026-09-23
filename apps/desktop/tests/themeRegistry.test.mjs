import assert from "node:assert/strict";
import test from "node:test";
import { defaultThemeId, findTheme, themeRegistry } from "../src/ui/theme/registry.ts";

const hex = /^#[0-9a-f]{6}$/i;
const required = ["crust", "mantle", "base", "surface0", "surface1", "surface2", "text", "subtext", "overlay", "accent", "accentForeground", "danger", "warning", "success"];

test("themes have unique ids and complete hex palettes", () => {
  const ids = new Set();
  for (const theme of themeRegistry) {
    assert.ok(!ids.has(theme.id), `duplicate theme id ${theme.id}`);
    ids.add(theme.id);
    for (const key of required) assert.match(theme.tokens[key], hex, `${theme.id}.${key}`);
    if (theme.tokens.macro !== undefined) assert.match(theme.tokens.macro, hex, `${theme.id}.macro`);
  }
  assert.equal(findTheme(defaultThemeId).id, defaultThemeId);
  assert.equal(findTheme("missing-theme").id, themeRegistry[0].id);
});
