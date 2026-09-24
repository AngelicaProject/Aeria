import assert from "node:assert/strict";
import test from "node:test";

import {
  addModels,
  parseContextWindow,
  parseSelectionKey,
  providerFromPreset,
  providerInput,
  removeModel,
  selectableEfforts,
  selectionKey,
  toggleEffort,
} from "../src/aiSettings.ts";

const model = (id, reasoningEfforts = []) => ({ id, contextWindow: null, reasoningEfforts });

const provider = {
  id: "p1",
  kind: "openCodeGo",
  name: "OpenCode Go",
  baseUrl: "https://opencode.ai/zen/go/v1",
  models: [model("glm-5.3", ["low", "high"])],
  apiKey: "stored",
};

test("a preset becomes a new provider input with copied models", () => {
  const preset = { kind: "openCodeGo", name: "OpenCode Go", baseUrl: "https://opencode.ai/zen/go/v1", models: [model("kimi-k3")] };
  const input = providerFromPreset(preset);
  assert.deepEqual(input, { id: null, kind: "openCodeGo", name: "OpenCode Go", baseUrl: "https://opencode.ai/zen/go/v1", models: [model("kimi-k3")] });
  input.models[0].reasoningEfforts.push("low");
  assert.deepEqual(preset.models[0].reasoningEfforts, []);
  assert.equal(providerFromPreset({ ...preset, baseUrl: null }, "http://localhost:11434/v1").baseUrl, "http://localhost:11434/v1");
});

test("a stored provider becomes a replacement input without its key state", () => {
  const input = providerInput(provider, { name: "Go" });
  assert.deepEqual(input, { id: "p1", kind: "openCodeGo", name: "Go", baseUrl: provider.baseUrl, models: provider.models });
  assert.equal("apiKey" in input, false);
});

test("adding models trims, skips blanks, and keeps IDs unique", () => {
  const models = addModels([model("glm-5.3")], [" kimi-k3 ", "", "glm-5.3", "kimi-k3"]);
  assert.deepEqual(models.map((entry) => entry.id), ["glm-5.3", "kimi-k3"]);
  assert.deepEqual(removeModel(models, "glm-5.3").map((entry) => entry.id), ["kimi-k3"]);
});

test("efforts toggle in canonical order", () => {
  let models = [model("glm-5.3")];
  models = toggleEffort(models, "glm-5.3", "high");
  models = toggleEffort(models, "glm-5.3", "minimal");
  assert.deepEqual(models[0].reasoningEfforts, ["minimal", "high"]);
  models = toggleEffort(models, "glm-5.3", "high");
  assert.deepEqual(models[0].reasoningEfforts, ["minimal"]);
});

test("context windows accept blank or positive integers only", () => {
  assert.equal(parseContextWindow(" "), null);
  assert.equal(parseContextWindow("200000"), 200000);
  assert.equal(parseContextWindow("0"), undefined);
  assert.equal(parseContextWindow("1.5"), undefined);
  assert.equal(parseContextWindow("-3"), undefined);
});

test("effort choices follow the selected model", () => {
  assert.deepEqual(selectableEfforts([provider], { providerId: "p1", modelId: "glm-5.3" }), ["low", "high"]);
  assert.deepEqual(selectableEfforts([provider], { providerId: "p1", modelId: "missing" }), []);
  assert.deepEqual(selectableEfforts([provider], null), []);
});

test("selection keys round-trip provider and model IDs containing separators", () => {
  const selection = { providerId: "p:1", modelId: "vendor/model:free" };
  assert.deepEqual(parseSelectionKey(selectionKey(selection)), selection);
  assert.equal(parseSelectionKey(""), null);
  assert.equal(parseSelectionKey("[1,2]"), null);
});
