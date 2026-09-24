import assert from "node:assert/strict";
import test from "node:test";

import {
  applyAgentEvent,
  contextFill,
  isToolError,
  parseReply,
  parseSpans,
  resolveModel,
  toolSubject,
  transcriptFromMessages,
} from "../src/angelica.ts";

test("stored messages become a transcript with tool results joined to calls", () => {
  const items = transcriptFromMessages([
    { role: "user", content: "Сколько листов?" },
    { role: "assistant", content: "", reasoning: "look", toolCalls: [{ id: "c1", name: "list_sheets", arguments: "{}" }] },
    { role: "tool", toolCallId: "c1", name: "list_sheets", content: "{\"error\":\"no project\"}" },
    { role: "assistant", content: "Готово" },
  ]);
  assert.deepEqual(items.map((item) => item.kind), ["user", "assistant", "tool", "assistant"]);
  assert.equal(items[1].reasoning, "look");
  assert.equal(items[2].result, "{\"error\":\"no project\"}");
  assert.equal(items[2].isError, true);
});

test("live events stream text into one reply and track tools", () => {
  let items = [{ kind: "user", key: "m0", text: "q" }];
  items = applyAgentEvent(items, { type: "reasoningDelta", text: "hm" });
  items = applyAgentEvent(items, { type: "textDelta", text: "При" });
  items = applyAgentEvent(items, { type: "textDelta", text: "вет" });
  assert.equal(items.length, 2);
  assert.equal(items[1].text, "Привет");
  assert.equal(items[1].reasoning, "hm");
  assert.equal(items[1].streaming, true);

  items = applyAgentEvent(items, { type: "responseFinished" });
  assert.equal(items[1].streaming, false);
  items = applyAgentEvent(items, { type: "toolStarted", id: "c1", name: "get_unit", arguments: "{\"sheet\":\"Item\",\"row\":5}" });
  assert.equal(items[2].result, null);
  items = applyAgentEvent(items, { type: "toolFinished", id: "c1", name: "get_unit", content: "{}", isError: false });
  assert.equal(items[2].result, "{}");
  items = applyAgentEvent(items, { type: "textDelta", text: "Next" });
  assert.equal(items.length, 4);
  assert.equal(items[3].text, "Next");
});

test("tool errors and subjects are read from JSON", () => {
  assert.equal(isToolError("{\"error\":\"x\"}"), true);
  assert.equal(isToolError("[]"), false);
  assert.equal(isToolError("not json"), false);
  assert.equal(toolSubject("get_unit", "{\"sheet\":\"Item\",\"row\":5,\"column\":1}"), "Item:5:0:1");
  assert.equal(toolSubject("list_sheets", "{\"query\":\"quest\"}"), "“quest”");
  assert.equal(toolSubject("project_overview", "oops"), "");
});

const providers = [
  { id: "p1", kind: "openCodeGo", name: "Go", baseUrl: "https://x/v1", models: [{ id: "glm", contextWindow: 1000, reasoningEfforts: ["high"] }], sessionHeader: null, headers: [], apiKey: "stored" },
  { id: "p2", kind: "custom", name: "Local", baseUrl: "http://localhost/v1", models: [{ id: "llama", contextWindow: null, reasoningEfforts: [] }], sessionHeader: null, headers: [], apiKey: "stored" },
];

test("the model comes from the conversation, then the default, then the first model", () => {
  assert.deepEqual(resolveModel(providers, [{ providerId: "p2", modelId: "llama", effort: null }, { providerId: "p1", modelId: "glm", effort: "high" }]), { providerId: "p2", modelId: "llama", effort: null });
  assert.deepEqual(resolveModel(providers, [{ providerId: "gone", modelId: "x", effort: null }, { providerId: "p1", modelId: "glm", effort: "high" }]), { providerId: "p1", modelId: "glm", effort: "high" });
  assert.deepEqual(resolveModel(providers, [{ providerId: "p2", modelId: "llama", effort: "low" }]), { providerId: "p2", modelId: "llama", effort: null });
  assert.deepEqual(resolveModel(providers, [null]), { providerId: "p1", modelId: "glm", effort: null });
  assert.equal(resolveModel([], [null]), null);
});

test("context fill is known only with a context window", () => {
  assert.equal(contextFill(500, 1000), 0.5);
  assert.equal(contextFill(5000, 1000), 1);
  assert.equal(contextFill(500, null), null);
  assert.equal(contextFill(null, 1000), null);
});

test("replies render a small Markdown subset without HTML", () => {
  const blocks = parseReply("Строка **Item:5:0:1**:\n\n- `<if(PlayerParameter(4))>` — пол\n- второй пункт\n\n```\n<num(lnum1)>\n```\nКонец");
  assert.deepEqual(blocks.map((block) => block.kind), ["paragraph", "list", "code", "paragraph"]);
  assert.deepEqual(blocks[0].spans, [{ kind: "text", text: "Строка " }, { kind: "strong", text: "Item:5:0:1" }, { kind: "text", text: ":" }]);
  assert.equal(blocks[1].items.length, 2);
  assert.deepEqual(blocks[1].items[0][0], { kind: "code", text: "<if(PlayerParameter(4))>" });
  assert.equal(blocks[2].text, "<num(lnum1)>");
  assert.deepEqual(parseSpans("<color(1)>"), [{ kind: "text", text: "<color(1)>" }]);
});
