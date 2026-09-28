import type { ConditionDto, GuardDto, OperandDto } from "./types";
import type { MessageKey, Translate } from "./i18n/translate";

/**
 * Conditions of the scene view in words. Common script functions read as
 * phrases, such as "the player's sex" or "quest «A» is complete"; others read
 * as code. Presentation only: the conditions themselves come from
 * `sheet_dialogue`.
 */

/** Readable names of the script functions conditions test most often. */
const subjects: Readonly<Record<string, MessageKey>> = {
  GetSex: "scene.subject.sex",
  GetRace: "scene.subject.race",
  GetTribe: "scene.subject.tribe",
  GetClassJob: "scene.subject.classJob",
  GetQuestAcceptClassJob: "scene.subject.acceptClassJob",
  IsQuestCompleted: "scene.subject.questCompleted",
  IsQuestAccepted: "scene.subject.questAccepted",
  GetNumOfItems: "scene.subject.items",
  QuestReward: "scene.subject.reward",
};

/** How the facts the common script functions report read when they do not hold. */
const negatedSubjects: Readonly<Record<string, MessageKey>> = {
  IsQuestCompleted: "scene.subject.questNotCompleted",
  IsQuestAccepted: "scene.subject.questNotAccepted",
  QuestReward: "scene.subject.noReward",
};

/** A fact that does not hold, in words where the function is known. */
function negatedText(t: Translate, operand: OperandDto): string {
  if (operand.kind === "call" && operand.function && negatedSubjects[operand.function]) {
    return t(negatedSubjects[operand.function]!, { arguments: operand.arguments.map(operandCode).join(", ") });
  }
  return t("scene.not", { subject: subjectText(t, operand) });
}

const comparisons = { eq: "=", ne: "≠", lt: "<", le: "≤", gt: ">", ge: "≥" } as const;

/** An operand as code, such as `IsQuestCompleted(65575)`. */
export function operandCode(operand: OperandDto): string {
  switch (operand.kind) {
    case "oneOf": return operand.operands.map(operandCode).join(" | ");
    case "quest": return operand.name ? `«${operand.name}»` : operand.variable;
    case "call": return `${operand.function ?? "?"}(${operand.arguments.map(operandCode).join(", ")})`;
    case "number": return String(operand.value);
    case "boolean": return String(operand.value);
    case "nil": return "nil";
    case "string": return JSON.stringify(operand.value);
    case "field":
    case "global": return operand.name;
    case "answer": return "answer";
    case "unknown": return "?";
  }
}

function subjectText(t: Translate, operand: OperandDto): string {
  if (operand.kind === "answer") return t("scene.subject.answer");
  if (operand.kind === "unknown") return t("scene.subject.unknown");
  if (operand.kind === "oneOf") return t("scene.subject.oneOf", { values: operand.operands.map((inner) => subjectText(t, inner)).join("; ") });
  if (operand.kind === "call" && operand.function && subjects[operand.function]) {
    return t(subjects[operand.function]!, { arguments: operand.arguments.map(operandCode).join(", ") });
  }
  return operandCode(operand);
}

/** A condition in words where the script function is known, and as code otherwise. */
export function conditionText(t: Translate, condition: ConditionDto): string {
  const subject = subjectText(t, condition.subject);
  const { test } = condition;
  if (test.kind === "truthy") return test.value ? subject : negatedText(t, condition.subject);
  // `= true` reads as the fact itself, `= false` and `≠ true` as its negation.
  if (test.value.kind === "boolean" && (test.comparison === "eq" || test.comparison === "ne")) {
    return (test.comparison === "eq") === test.value.value ? subject : negatedText(t, condition.subject);
  }
  return `${subject} ${comparisons[test.comparison]} ${operandCode(test.value)}`;
}

/** A test of a quest function on one quest, and whether it holds when the function does. */
function questTest(guard: GuardDto): { function: string; quest: string; positive: boolean } | null {
  if (guard.kind !== "test" || guard.subject.kind !== "call") return null;
  const { function: name, arguments: [quest] } = guard.subject;
  if ((name !== "IsQuestCompleted" && name !== "IsQuestAccepted") || quest?.kind !== "quest") return null;
  const { test } = guard;
  let positive: boolean;
  if (test.kind === "truthy") positive = test.value;
  else if (test.value.kind === "boolean" && (test.comparison === "eq" || test.comparison === "ne")) positive = (test.comparison === "eq") === test.value.value;
  else return null;
  return { function: name, quest: operandCode(quest), positive };
}

/** `any` or `all` of the same quest function on several quests, said once: "one of these quests is complete: A, B". */
const questGroups: Readonly<Record<string, Readonly<Record<string, MessageKey>>>> = {
  IsQuestCompleted: { anyTrue: "scene.quests.completedAny", allTrue: "scene.quests.completedAll", allFalse: "scene.quests.completedNone", anyFalse: "scene.quests.completedNotAll" },
  IsQuestAccepted: { anyTrue: "scene.quests.acceptedAny", allTrue: "scene.quests.acceptedAll", allFalse: "scene.quests.acceptedNone", anyFalse: "scene.quests.acceptedNotAll" },
};

function questGroupText(t: Translate, guard: GuardDto): string | null {
  if (guard.kind === "test" || guard.guards.length < 2) return null;
  const tests = guard.guards.map(questTest);
  const first = tests[0];
  if (!first || tests.some((test) => !test || test.function !== first.function || test.positive !== first.positive)) return null;
  const key = questGroups[first.function]?.[`${guard.kind}${first.positive ? "True" : "False"}`];
  return key ? t(key, { quests: tests.map((test) => test!.quest).join(", ") }) : null;
}

/**
 * One value compared with several: `any` of `x = a`, `x = b` reads "x is one
 * of: a, b", and `all` of `x ≠ a`, `x ≠ b` reads "x is none of: a, b".
 */
function valueGroupText(t: Translate, guard: GuardDto): string | null {
  if (guard.kind === "test" || guard.guards.length < 2) return null;
  const comparison = guard.kind === "any" ? "eq" : "ne";
  const values: string[] = [];
  let subject: string | null = null;
  for (const inner of guard.guards) {
    if (!groupable(inner, comparison)) return null;
    const code = operandCode(inner.subject);
    if (subject !== null && code !== subject) return null;
    subject = code;
    values.push(operandCode(inner.test.value));
  }
  const first = guard.guards[0] as ConditionDto;
  return t(guard.kind === "any" ? "scene.oneOfValues" : "scene.noneOfValues", { subject: subjectText(t, first.subject), values: values.join(", ") });
}

/**
 * A comparison of a known value that reads together with others of the same
 * value. Values the script computes, or answers, may differ though they read
 * the same.
 */
function groupable(guard: GuardDto, comparison: "eq" | "ne"): guard is ConditionDto & { test: { kind: "compare" } } {
  return guard.kind === "test"
    && guard.test.kind === "compare"
    && guard.test.comparison === comparison
    && guard.subject.kind !== "unknown"
    && guard.subject.kind !== "answer";
}

type GroupDto = Extract<GuardDto, { kind: "any" | "all" }>;

/**
 * The guards of a group, with the comparisons of one value merged where the
 * first of them is: `a = 1 or b or a = 2` is `a is one of: 1, 2` or `b`.
 */
function groupParts(guard: GroupDto): GuardDto[] {
  const comparison = guard.kind === "any" ? "eq" : "ne";
  const bySubject = new Map<string, GuardDto[]>();
  const parts: Array<GuardDto | GuardDto[]> = [];
  for (const inner of guard.guards) {
    if (!groupable(inner, comparison)) {
      parts.push(inner);
      continue;
    }
    const code = operandCode(inner.subject);
    const same = bySubject.get(code);
    if (same) same.push(inner);
    else {
      const list = [inner];
      bySubject.set(code, list);
      parts.push(list);
    }
  }
  return parts.map((part) => !Array.isArray(part) ? part : part.length === 1 ? part[0]! : { kind: guard.kind, guards: part });
}

/** Conditions joined in words: `a, or b, or c`. A group inside another reads in brackets. */
export function guardText(t: Translate, guard: GuardDto): string {
  if (guard.kind === "test") return conditionText(t, guard);
  const group = questGroupText(t, guard) ?? valueGroupText(t, guard);
  if (group) return group;
  const joiner = t(guard.kind === "any" ? "scene.or" : "scene.and");
  return groupParts(guard).map((part) => {
    if (part.kind === "test") return conditionText(t, part);
    return questGroupText(t, part) ?? valueGroupText(t, part) ?? `(${guardText(t, part)})`;
  }).join(` ${joiner} `);
}

const negations = { eq: "ne", ne: "eq", lt: "ge", ge: "lt", le: "gt", gt: "le" } as const;

/** The opposite condition, for a branch that runs only when its condition fails. */
export function negate(guard: GuardDto): GuardDto {
  if (guard.kind === "any") return { kind: "all", guards: guard.guards.map(negate) };
  if (guard.kind === "all") return { kind: "any", guards: guard.guards.map(negate) };
  const { test } = guard;
  return {
    kind: "test",
    subject: guard.subject,
    test: test.kind === "truthy" ? { kind: "truthy", value: !test.value } : { ...test, comparison: negations[test.comparison] },
  };
}
