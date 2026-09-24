import assert from "node:assert/strict";
import test from "node:test";
import { defaultPreferences, parsePreferences } from "../src/ui/preferencesModel.ts";

test("preferences fall back to defaults for missing, malformed, or unknown values", () => {
  assert.deepEqual(parsePreferences(null), defaultPreferences);
  assert.deepEqual(parsePreferences("{not json"), defaultPreferences);
  const parsed = parsePreferences(JSON.stringify({ editorFontSize: 16, interfaceZoom: 3, listDensity: "tiny", highlightMacros: false, focusTargetOnNext: "yes" }));
  assert.equal(parsed.editorFontSize, 16);
  assert.equal(parsed.interfaceZoom, defaultPreferences.interfaceZoom);
  assert.equal(parsed.listDensity, defaultPreferences.listDensity);
  assert.equal(parsed.highlightMacros, false);
  assert.equal(parsed.focusTargetOnNext, defaultPreferences.focusTargetOnNext);
});

test("interface language accepts supported choices only", () => {
  assert.equal(parsePreferences(JSON.stringify({ language: "ru" })).language, "ru");
  assert.equal(parsePreferences(JSON.stringify({ language: "system" })).language, "system");
  assert.equal(parsePreferences(JSON.stringify({ language: "de" })).language, defaultPreferences.language);
});
