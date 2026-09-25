import type { AgentEvent, AiModelSelection, AiProviderDto, AiUsage, ChatMessage, JobCounts, JobSummary } from "./types";

/** Angelica's fixed name. It is never localized. */
export const ANGELICA = "Angelica";

/** One rendered entry of a conversation. */
export type TranscriptItem =
  | { kind: "user"; key: string; text: string }
  /** An update Aeria sent to Angelica, such as a finished job. */
  | { kind: "notice"; key: string; text: string }
  | { kind: "assistant"; key: string; text: string; reasoning: string; streaming: boolean }
  | { kind: "tool"; key: string; id: string; name: string; arguments: string; result: string | null; isError: boolean };

/** Returns whether a stored tool result is an error object. */
export function isToolError(content: string): boolean {
  try {
    const value: unknown = JSON.parse(content);
    return typeof value === "object" && value !== null && !Array.isArray(value) && typeof (value as Record<string, unknown>).error === "string";
  } catch {
    return false;
  }
}

/** Builds the transcript of stored messages; tool results join their calls. */
export function transcriptFromMessages(messages: readonly ChatMessage[]): TranscriptItem[] {
  const items: TranscriptItem[] = [];
  const tools = new Map<string, Extract<TranscriptItem, { kind: "tool" }>>();
  messages.forEach((message, index) => {
    if (message.role === "user") {
      items.push({ kind: message.automatic ? "notice" : "user", key: `m${index}`, text: message.content });
    } else if (message.role === "assistant") {
      if (message.content || message.reasoning) {
        items.push({ kind: "assistant", key: `m${index}`, text: message.content, reasoning: message.reasoning ?? "", streaming: false });
      }
      for (const call of message.toolCalls ?? []) {
        const item: Extract<TranscriptItem, { kind: "tool" }> = { kind: "tool", key: `t${index}-${call.id}`, id: call.id, name: call.name, arguments: call.arguments, result: null, isError: false };
        tools.set(call.id, item);
        items.push(item);
      }
    } else {
      const item = tools.get(message.toolCallId);
      if (item) {
        item.result = message.content;
        item.isError = isToolError(message.content);
      }
    }
  });
  return items;
}

/** Applies one live event to the transcript of the running turn. */
export function applyAgentEvent(items: readonly TranscriptItem[], event: AgentEvent): TranscriptItem[] {
  const last = items[items.length - 1];
  const streaming = last?.kind === "assistant" && last.streaming ? last : null;
  switch (event.type) {
    case "textDelta":
    case "reasoningDelta": {
      const base = streaming ?? { kind: "assistant" as const, key: `live${items.length}`, text: "", reasoning: "", streaming: true };
      const next = event.type === "textDelta" ? { ...base, text: base.text + event.text } : { ...base, reasoning: base.reasoning + event.text };
      return streaming ? [...items.slice(0, -1), next] : [...items, next];
    }
    case "responseFinished":
      return streaming ? [...items.slice(0, -1), { ...streaming, streaming: false }] : [...items];
    case "toolStarted":
      return [...items, { kind: "tool", key: `live-${event.id}-${items.length}`, id: event.id, name: event.name, arguments: event.arguments, result: null, isError: false }];
    case "toolFinished":
      return items.map((item) => item.kind === "tool" && item.id === event.id && item.result === null ? { ...item, result: event.content, isError: event.isError } : item);
    default:
      return [...items];
  }
}

/** Parses tool arguments for display; invalid JSON yields `null`. */
export function toolArguments(text: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(text || "{}");
    return typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

/** A short location or query shown next to a tool name. */
export function toolSubject(name: string, argumentsText: string): string {
  const args = toolArguments(argumentsText);
  if (!args) return "";
  if (typeof args.sheet === "string") {
    const parts = [args.sheet];
    if (typeof args.row === "number") parts.push(String(args.row), String(typeof args.subrow === "number" ? args.subrow : 0));
    if (typeof args.column === "number") parts.push(String(args.column));
    return parts.join(":");
  }
  if (typeof args.query === "string") return `“${args.query}”`;
  if (name === "fetch_url" && typeof args.url === "string") return args.url.replace(/^https?:\/\//, "").slice(0, 60);
  if (name === "similar_translations" && typeof args.text === "string") return `“${args.text}”`;
  if (typeof args.unit_id === "string") return args.unit_id.slice(0, 12);
  return "";
}

/**
 * Picks the model for the next message: the conversation's own, then the
 * default, then the first configured model. A choice that no longer matches
 * the settings is skipped; an effort the model no longer accepts is dropped.
 */
export function resolveModel(providers: readonly AiProviderDto[], candidates: ReadonlyArray<AiModelSelection | null | undefined>): AiModelSelection | null {
  for (const candidate of candidates) {
    if (!candidate) continue;
    const model = providers.find((provider) => provider.id === candidate.providerId)?.models.find((entry) => entry.id === candidate.modelId);
    if (!model) continue;
    return { ...candidate, effort: candidate.effort && model.reasoningEfforts.includes(candidate.effort) ? candidate.effort : null };
  }
  for (const provider of providers) {
    const model = provider.models[0];
    if (model) return { providerId: provider.id, modelId: model.id, effort: null };
  }
  return null;
}

/** Share of the context window the last request used, from 0 to 1, when known. */
export function contextFill(lastPromptTokens: number | null, contextWindow: number | null): number | null {
  if (lastPromptTokens === null || !contextWindow) return null;
  return Math.min(1, lastPromptTokens / contextWindow);
}

export function totalTokens(usage: AiUsage): number {
  return usage.promptTokens + usage.completionTokens;
}

/** A block of Angelica's reply for rendering. */
export type ReplyBlock =
  | { kind: "paragraph"; spans: ReplySpan[] }
  | { kind: "list"; items: ReplySpan[][] }
  | { kind: "code"; text: string };

export type ReplySpan = { kind: "text" | "code" | "strong"; text: string };

/** Splits inline `code` and **strong** spans; everything else is plain text. */
export function parseSpans(text: string): ReplySpan[] {
  const spans: ReplySpan[] = [];
  const pattern = /`([^`\n]+)`|\*\*([^*\n]+)\*\*/g;
  let last = 0;
  for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
    if (match.index > last) spans.push({ kind: "text", text: text.slice(last, match.index) });
    spans.push(match[1] !== undefined ? { kind: "code", text: match[1] } : { kind: "strong", text: match[2]! });
    last = match.index + match[0].length;
  }
  if (last < text.length) spans.push({ kind: "text", text: text.slice(last) });
  return spans;
}

/**
 * A deliberately small Markdown subset: fenced code, bullet or numbered
 * lists, and paragraphs with inline code and bold. Nothing is rendered as
 * HTML, so game macros such as `<if(...)>` always stay visible text.
 */
export function parseReply(text: string): ReplyBlock[] {
  const blocks: ReplyBlock[] = [];
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let paragraph: string[] = [];
  let list: ReplySpan[][] = [];
  const flush = () => {
    if (paragraph.length) blocks.push({ kind: "paragraph", spans: parseSpans(paragraph.join("\n")) });
    if (list.length) blocks.push({ kind: "list", items: list });
    paragraph = [];
    list = [];
  };
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index]!;
    if (line.trimStart().startsWith("```")) {
      flush();
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !lines[index]!.trimStart().startsWith("```")) {
        code.push(lines[index]!);
        index += 1;
      }
      blocks.push({ kind: "code", text: code.join("\n") });
      continue;
    }
    const item = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(line);
    if (item) {
      if (paragraph.length) { blocks.push({ kind: "paragraph", spans: parseSpans(paragraph.join("\n")) }); paragraph = []; }
      list.push(parseSpans(item[1]!));
      continue;
    }
    if (!line.trim()) {
      flush();
      continue;
    }
    if (list.length) { blocks.push({ kind: "list", items: list }); list = []; }
    paragraph.push(line);
  }
  flush();
  return blocks;
}

/** Share of a job's strings with a final outcome, from 0 to 1. */
export function jobProgress(counts: JobCounts): number {
  if (counts.total === 0) return 1;
  return (counts.drafted + counts.rejected + counts.failed + counts.conflict) / counts.total;
}

/** Strings a job could not draft. */
export function jobProblems(counts: JobCounts): number {
  return counts.rejected + counts.failed + counts.conflict;
}

/** Jobs that still need attention first, then the newest. */
export function sortJobs(jobs: readonly JobSummary[]): JobSummary[] {
  const rank = (job: JobSummary) => job.status === "running" ? 0 : job.status === "paused" ? 1 : 2;
  return [...jobs].sort((left, right) => rank(left) - rank(right) || right.createdAtUnixMs - left.createdAtUnixMs);
}

/** One step inside an activity block. */
export type ActivityStep =
  | { kind: "reasoning"; key: string; text: string }
  | { kind: "tool"; item: Extract<TranscriptItem, { kind: "tool" }> };

/** What the conversation shows: messages, replies, and folded activity. */
export type TranscriptBlock =
  | { kind: "user" | "notice"; key: string; text: string }
  | { kind: "reply"; key: string; text: string }
  | { kind: "activity"; key: string; steps: ActivityStep[]; live: boolean };

/**
 * Folds reasoning and tool calls between two replies into one activity
 * block, so the conversation reads as messages with compact work between
 * them. The last block is live while a turn runs.
 */
export function groupTranscript(items: readonly TranscriptItem[], running: boolean): TranscriptBlock[] {
  const blocks: TranscriptBlock[] = [];
  let activity: Extract<TranscriptBlock, { kind: "activity" }> | null = null;
  const step = (key: string, next: ActivityStep) => {
    if (!activity) {
      activity = { kind: "activity", key: `a-${key}`, steps: [], live: false };
      blocks.push(activity);
    }
    activity.steps.push(next);
  };
  for (const item of items) {
    if (item.kind === "user" || item.kind === "notice") {
      activity = null;
      blocks.push({ kind: item.kind, key: item.key, text: item.text });
    } else if (item.kind === "tool") {
      step(item.key, { kind: "tool", item });
    } else {
      if (item.reasoning.trim()) step(item.key, { kind: "reasoning", key: `r-${item.key}`, text: item.reasoning });
      if (item.text) {
        activity = null;
        blocks.push({ kind: "reply", key: item.key, text: item.text });
      }
    }
  }
  const last = blocks.at(-1);
  if (running && last?.kind === "activity") last.live = true;
  return blocks;
}

/** Counts for an activity block's folded line. */
export function activitySummary(steps: readonly ActivityStep[]): { tools: number; failed: number; reasoning: boolean } {
  let tools = 0;
  let failed = 0;
  let reasoning = false;
  for (const entry of steps) {
    if (entry.kind === "reasoning") reasoning = true;
    else {
      tools += 1;
      if (entry.item.isError) failed += 1;
    }
  }
  return { tools, failed, reasoning };
}

/** The latest `**Title**` heading of reasoning text, as models summarize their steps. */
export function reasoningTitle(text: string): string | null {
  const titles = [...text.matchAll(/\*\*([^*\n]+)\*\*/g)];
  return titles.at(-1)?.[1]?.trim() || null;
}

/** Elapsed time as `12s` or `3m 05s`. */
export function formatElapsed(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, "0")}s`;
}

/** A compact token count: `950`, `5.2k`, `1.3M`. */
export function formatTokens(count: number): string {
  if (count < 1000) return String(count);
  if (count < 1_000_000) return `${(count / 1000).toFixed(count < 10_000 ? 1 : 0)}k`;
  return `${(count / 1_000_000).toFixed(1)}M`;
}

/**
 * What a running turn is doing, from its latest transcript item. Angelica is
 * one agent to the user, so time spent waiting for the provider is thinking.
 */
export type WorkingPhase = "thinking" | "tools" | "writing";

export function workingPhase(items: readonly TranscriptItem[]): WorkingPhase {
  const last = items.at(-1);
  if (last?.kind === "tool" && last.result === null) return "tools";
  if (last?.kind === "assistant" && last.streaming && last.text) return "writing";
  return "thinking";
}

/** Silence after which a playful status fills the wait. */
export const PLAYFUL_AFTER_MS = 8000;

/** Whether the model has been silent long enough for a playful status. */
export function isQuietWait(now: number, lastActivityAt: number): boolean {
  return now - lastActivityAt >= PLAYFUL_AFTER_MS;
}
