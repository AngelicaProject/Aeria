import type { MessageKey, Translate } from "./i18n/translate";
import type { CommandError, IssueDto } from "./types";

const full: Partial<Record<IssueDto["kind"], MessageKey>> = {
  lineBreak: "issue.lineBreak",
  structure: "issue.structure",
  forbiddenTerm: "issue.forbiddenTerm",
  mark: "issue.mark",
  mixedAlphabets: "issue.mixedAlphabets",
  bothGenders: "issue.bothGenders",
  repeatedWord: "issue.repeatedWord",
  termNotUsed: "issue.termNotUsed",
  genderNotVaried: "issue.genderNotVaried",
  genderInOtherLanguages: "issue.genderInOtherLanguages",
  machinePhrasing: "issue.machinePhrasing",
  staleTermException: "issue.staleTermException",
  labelTooLong: "issue.labelTooLong",
  nameTooLong: "issue.nameTooLong",
};

const short: Partial<Record<IssueDto["kind"], MessageKey>> = {
  lineBreak: "issue.short.lineBreak",
  structure: "issue.short.structure",
  forbiddenTerm: "issue.short.forbiddenTerm",
  mark: "issue.short.mark",
  mixedAlphabets: "issue.short.mixedAlphabets",
  bothGenders: "issue.short.bothGenders",
  repeatedWord: "issue.short.repeatedWord",
  termNotUsed: "issue.short.termNotUsed",
  genderNotVaried: "issue.short.genderNotVaried",
  genderInOtherLanguages: "issue.short.genderInOtherLanguages",
  machinePhrasing: "issue.short.machinePhrasing",
  staleTermException: "issue.short.staleTermException",
  labelTooLong: "issue.short.labelTooLong",
  nameTooLong: "issue.short.nameTooLong",
};

function params(issue: IssueDto): Record<string, string> {
  return {
    term: issue.term ?? "",
    translation: issue.translation ?? "",
    variant: issue.variant ?? "",
    text: issue.text ?? "",
    phrases: issue.phrases.join(", "),
    length: String(issue.length ?? ""),
    max: String(issue.max ?? ""),
    // The structure policy writes its reasons for the model, in English.
    message: issue.message,
  };
}

/** A finding of the checks in the interface language; unknown kinds keep their English message. */
export function describeIssue(issue: IssueDto, t: Translate): string {
  const key = full[issue.kind];
  return key ? t(key, params(issue)) : issue.message;
}

/** A short label of an issue's group, for the summary of a search. */
export function issueLabel(issue: IssueDto, t: Translate): string {
  const key = short[issue.kind];
  return key ? t(key, params(issue)) : issue.message;
}

/** The glossary term an exception can be made for: a term issue's term without a comma, which a flag cannot hold. */
export function exceptionTerm(issue: IssueDto): string | null {
  if (issue.kind !== "termNotUsed" && issue.kind !== "forbiddenTerm") return null;
  const term = issue.term?.trim() ?? "";
  return term && !term.includes(",") ? term : null;
}

/** A command error's text: its issues in the interface language when it has them, else its message. */
export function errorText(error: CommandError, t: Translate): string {
  return error.issues && error.issues.length > 0 ? error.issues.map((issue) => describeIssue(issue, t)).join("; ") : error.message;
}
