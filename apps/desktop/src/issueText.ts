import type { MessageKey, Translate } from "./i18n/translate";
import type { CommandError, IssueDto } from "./types";

const full: Partial<Record<IssueDto["kind"], MessageKey>> = {
  lineBreak: "issue.lineBreak",
  structure: "issue.structure",
  staleTermException: "issue.staleTermException",
  nameTooLong: "issue.nameTooLong",
};

const short: Partial<Record<IssueDto["kind"], MessageKey>> = {
  lineBreak: "issue.short.lineBreak",
  structure: "issue.short.structure",
  staleTermException: "issue.short.staleTermException",
  nameTooLong: "issue.short.nameTooLong",
};

function params(issue: IssueDto): Record<string, string> {
  return {
    term: issue.term ?? "",
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

/** A command error's text: its issues in the interface language when it has them, else its message. */
export function errorText(error: CommandError, t: Translate): string {
  return error.issues && error.issues.length > 0 ? error.issues.map((issue) => describeIssue(issue, t)).join("; ") : error.message;
}
