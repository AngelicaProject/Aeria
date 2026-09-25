import assert from "node:assert/strict";
import test from "node:test";

import {
  activitySummary,
  applyAgentEvent,
  formatElapsed,
  formatTokens,
  groupTranscript,
  reasoningTitle,
  contextFill,
  isToolError,
  jobProblems,
  jobProgress,
  parseReply,
  parseSpans,
  resolveModel,
  sortJobs,
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

test("search tools show their query", () => {
  assert.equal(toolSubject("search_source", JSON.stringify({ query: "crystal" })), "“crystal”");
  assert.equal(toolSubject("similar_translations", JSON.stringify({ text: "Fire" })), "“Fire”");
  assert.equal(toolSubject("fetch_url", JSON.stringify({ url: "https://ffxiv.consolegameswiki.com/wiki/Aether" })), "ffxiv.consolegameswiki.com/wiki/Aether");
  assert.equal(toolSubject("similar_translations", JSON.stringify({ sheet: "Item", row: 1, column: 0 })), "Item:1:0:0");
});

test("automatic messages from Aeria are shown as notices", () => {
  const items = transcriptFromMessages([
    { role: "user", content: "Переведи Action" },
    { role: "user", content: "[Aeria] Job j1 finished", automatic: true },
  ]);
  assert.deepEqual(items.map((item) => item.kind), ["user", "notice"]);
});

test("job progress counts final outcomes and problems", () => {
  const counts = { total: 10, pending: 4, running: 2, drafted: 2, rejected: 1, failed: 0, conflict: 1 };
  assert.equal(jobProgress(counts), 0.4);
  assert.equal(jobProblems(counts), 2);
  assert.equal(jobProgress({ ...counts, total: 0 }), 1);
});

test("jobs needing attention come first, newest first within a status", () => {
  const job = (id, status, createdAtUnixMs) => ({ id, status, createdAtUnixMs });
  const sorted = sortJobs([job("a", "completed", 5), job("b", "paused", 1), job("c", "running", 2), job("d", "paused", 3)]);
  assert.deepEqual(sorted.map((entry) => entry.id), ["c", "d", "b", "a"]);
});

test("reasoning and tool calls fold into activity between replies", () => {
  const items = transcriptFromMessages([
    { role: "user", content: "Переведи" },
    { role: "assistant", content: "", reasoning: "**Reading rows**", toolCalls: [{ id: "c1", name: "read_rows", arguments: "{}" }, { id: "c2", name: "get_unit", arguments: "{}" }] },
    { role: "tool", toolCallId: "c1", name: "read_rows", content: "{}" },
    { role: "tool", toolCallId: "c2", name: "get_unit", content: "{\"error\":\"x\"}" },
    { role: "assistant", content: "Готово", reasoning: "**Writing**" },
    { role: "user", content: "Ещё" },
    { role: "assistant", content: "", toolCalls: [{ id: "c3", name: "read_rows", arguments: "{}" }] },
  ]);
  const blocks = groupTranscript(items, true);
  assert.deepEqual(blocks.map((block) => block.kind), ["user", "activity", "reply", "user", "activity"]);
  assert.deepEqual(blocks[1].steps.map((step) => step.kind), ["reasoning", "tool", "tool", "reasoning"]);
  assert.deepEqual(activitySummary(blocks[1].steps), { tools: 2, failed: 1, reasoning: true });
  assert.equal(blocks[1].live, false);
  assert.equal(blocks[4].live, true);
  assert.equal(groupTranscript(items, false)[4].live, false);
});

test("status helpers format reasoning titles, time, and tokens", () => {
  assert.equal(reasoningTitle(["**Checking terms**", "", "I look.", "", "**Reviewing titles**", "More"].join(String.fromCharCode(10))), "Reviewing titles");
  assert.equal(reasoningTitle("plain"), null);
  assert.equal(formatElapsed(12_400), "12s");
  assert.equal(formatElapsed(277_000), "4m 37s");
  assert.equal(formatTokens(950), "950");
  assert.equal(formatTokens(5_230), "5.2k");
  assert.equal(formatTokens(33_184), "33k");
  assert.equal(formatTokens(1_250_000), "1.3M");
});
