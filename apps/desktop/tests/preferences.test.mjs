import assert from "node:assert/strict";
import test from "node:test";
import { defaultPreferences, parsePreferences } from "../src/ui/preferencesModel.ts";

test("preferences fall back to defaults for missing, malformed, or unknown values", () => {
  assert.deepEqual(parsePreferences(null), defaultPreferences);
  assert.deepEqual(parsePreferences("{not json"), defaultPreferences);
  const parsed = parsePreferences(JSON.stringify({ editorFontSize: 16, interfaceZoom: 3, listDensity: "tiny", highlightMacros: false, focusTargetOnNext: "yes", sidePaneTab: "chat" }));
  assert.equal(parsed.editorFontSize, 16);
  assert.equal(parsed.interfaceZoom, defaultPreferences.interfaceZoom);
  assert.equal(parsed.listDensity, defaultPreferences.listDensity);
  assert.equal(parsed.highlightMacros, false);
  assert.equal(parsed.focusTargetOnNext, defaultPreferences.focusTargetOnNext);
  assert.equal(parsed.sidePaneTab, defaultPreferences.sidePaneTab);
  assert.equal(parsePreferences(JSON.stringify({ sidePaneTab: "languages" })).sidePaneTab, "languages");
  assert.equal(parsePreferences(JSON.stringify({ dialogueView: "film" })).dialogueView, defaultPreferences.dialogueView);
  assert.equal(parsePreferences(JSON.stringify({ dialogueView: "strings" })).dialogueView, "strings");
});

test("interface language accepts supported choices only", () => {
  assert.equal(parsePreferences(JSON.stringify({ language: "ru" })).language, "ru");
  assert.equal(parsePreferences(JSON.stringify({ language: "system" })).language, "system");
  assert.equal(parsePreferences(JSON.stringify({ language: "de" })).language, defaultPreferences.language);
});
