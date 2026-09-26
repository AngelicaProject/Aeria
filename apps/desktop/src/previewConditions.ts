import type { PreviewChoiceDto, PreviewOperandDto, PreviewPieceDto, PreviewTestDto } from "./types";

/**
 * Conditions of the game preview. A string's conditions read variables:
 * parameters such as `$n1`, global game variables such as `$gn68` (the
 * player's class), time values such as `$hour`, and properties of a
 * character (gender, whether it is the reading player). The preview
 * evaluates every condition from one set of values, as the game does, so
 * nested conditions follow from the values instead of being switched one
 * by one.
 */

export type ConditionVariableKind = "parameter" | "gameValue" | "gender" | "self" | "name" | "josa";

export type ConditionVariable = {
  /** `gn68`, `n1`, `hour`, or the character property's kind. */
  key: string;
  kind: ConditionVariableKind;
  /** The value that shows the first branch of the first condition reading it. */
  fallback: number;
};

export type ConditionValues = Readonly<Record<string, number>>;

function operandKey(operand: PreviewOperandDto): string | null {
  if (operand.type === "parameter") return `${operand.prefix}${operand.index}`;
  if (operand.type === "gameValue") return operand.name;
  return null;
}

/** The value of `key` that makes `operator` with `other` on the right hold. */
function satisfying(operator: string, other: number): number {
  switch (operator) {
    case ">": return other + 1;
    case "<": return Math.max(0, other - 1);
    case "!=": return other === 0 ? 1 : 0;
    default: return other;
  }
}

const mirrored: Readonly<Record<string, string>> = { "<": ">", "<=": ">=", ">": "<", ">=": "<=", "==": "==", "!=": "!=" };

function collectTest(test: PreviewTestDto, add: (key: string, kind: ConditionVariableKind, fallback: number) => void) {
  if (test.type === "value") {
    const key = operandKey(test.operand);
    if (key) add(key, test.operand.type === "gameValue" ? "gameValue" : "parameter", 1);
  } else if (test.type === "compare") {
    const left = operandKey(test.left);
    const right = operandKey(test.right);
    if (left) add(left, test.left.type === "gameValue" ? "gameValue" : "parameter", test.right.type === "int" ? satisfying(test.operator, test.right.value) : 1);
    if (right) add(right, test.right.type === "gameValue" ? "gameValue" : "parameter", test.left.type === "int" ? satisfying(mirrored[test.operator] ?? test.operator, test.left.value) : 1);
  }
}

/** Every variable the conditions of `pieces` read, in order of first use. */
export function conditionVariables(pieces: readonly PreviewPieceDto[]): ConditionVariable[] {
  const variables = new Map<string, ConditionVariable>();
  const add = (key: string, kind: ConditionVariableKind, fallback: number) => {
    if (!variables.has(key)) variables.set(key, { key, kind, fallback });
  };
  const walk = (list: readonly PreviewPieceDto[]) => {
    for (const piece of list) {
      if (piece.kind === "choice") {
        const choice = piece.choice;
        if (choice.type === "if") collectTest(choice.test, add);
        else if (choice.type === "switch") {
          const key = operandKey(choice.selector);
          if (key) add(key, choice.selector.type === "gameValue" ? "gameValue" : "parameter", 1);
        } else {
          const kind: ConditionVariableKind = choice.type === "myself" ? "self" : choice.type;
          add(kind, kind, kind === "self" || kind === "name" ? 1 : 0);
        }
        for (const branch of piece.branches) walk(branch);
      } else if (piece.kind === "ruby") {
        walk(piece.base);
        walk(piece.reading);
      }
    }
  };
  walk(pieces);
  return [...variables.values()];
}

function operandValue(operand: PreviewOperandDto, values: ConditionValues): number | null {
  if (operand.type === "int") return operand.value;
  const key = operandKey(operand);
  return key === null ? null : values[key] ?? null;
}

function holds(test: PreviewTestDto, values: ConditionValues): boolean | null {
  if (test.type === "value") {
    const value = operandValue(test.operand, values);
    return value === null ? null : value !== 0;
  }
  if (test.type === "compare") {
    const left = operandValue(test.left, values);
    const right = operandValue(test.right, values);
    if (left === null || right === null) return null;
    switch (test.operator) {
      case "==": return left === right;
      case "!=": return left !== right;
      case "<": return left < right;
      case "<=": return left <= right;
      case ">": return left > right;
      case ">=": return left >= right;
    }
  }
  return null;
}

/**
 * The branch the game shows for `values`: an index, -1 when no branch is
 * shown (a switch value without a case), or `null` when the condition
 * cannot be evaluated.
 */
export function chosenBranch(choice: PreviewChoiceDto, count: number, values: ConditionValues): number | null {
  switch (choice.type) {
    case "if": {
      const result = holds(choice.test, values);
      return result === null ? null : result ? 0 : 1;
    }
    case "switch": {
      const value = operandValue(choice.selector, values);
      if (value === null) return null;
      return value >= 1 && value <= count ? value - 1 : -1;
    }
    case "gender":
      return values.gender === 1 ? 1 : 0;
    case "myself":
      return values.self === 0 ? 1 : 0;
    case "name":
      return values.name === 0 ? 1 : 0;
    case "josa":
      return values.josa === 1 ? 1 : 0;
  }
}

/** The values to evaluate with: the chosen ones, else each variable's fallback. */
export function effectiveValues(variables: readonly ConditionVariable[], chosen: ConditionValues): ConditionValues {
  const values: Record<string, number> = {};
  for (const variable of variables) values[variable.key] = chosen[variable.key] ?? variable.fallback;
  return values;
}
