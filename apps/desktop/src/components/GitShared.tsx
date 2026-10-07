import { useCallback, useState, type ReactNode } from "react";
import { bindingKey } from "../binding";
import type { EntryChangeDto, EntryChangeKind, EntryVersionDto, ProjectArea, ProjectChangeDto, SourceBinding } from "../types";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

/** Where a change's string is in the game; null when the game has no such string. */
export function changeBinding(change: EntryChangeDto): SourceBinding | null {
  return change.sourceBinding;
}

const kindLetter: Record<EntryChangeKind, string> = { translated: "A", changed: "M", cleared: "D", marked: "•" };
/** The colour class of a kind, shared with file changes. */
const kindClass: Record<EntryChangeKind, string> = { translated: "added", changed: "modified", cleared: "removed", marked: "modified" };
export const kindLabel: Record<EntryChangeKind, MessageKey> = { translated: "git.kind.translated", changed: "git.kind.changed", cleared: "git.kind.cleared", marked: "git.kind.marked" };

/** Where a string is, for people: sheet, row, subrow, and column. */
export function bindingLabel(binding: SourceBinding | null, t: Translate): string {
  if (!binding) return t("git.unknownUnit");
  return t("common.cellLocation", { sheet: binding.sheetName, row: String(binding.rowId), subrow: String(binding.subrowId), column: String(binding.columnIndex) });
}

/** What changed about a string besides its kind: its note, its fuzzy mark, or its review. */
export function changeLabel(change: Pick<EntryChangeDto, "kind" | "before" | "after">, t: Translate): string {
  const parts: string[] = [];
  if (change.kind !== "marked") parts.push(t(kindLabel[change.kind]));
  if (change.before.fuzzy !== change.after.fuzzy) parts.push(t(change.after.fuzzy ? "git.change.fuzzy" : "git.change.notFuzzy"));
  if (change.before.translatorNote !== change.after.translatorNote) parts.push(t("git.change.note"));
  if (Boolean(change.before.reviewed) !== Boolean(change.after.reviewed)) parts.push(t(change.after.reviewed ? "git.change.reviewed" : "git.change.unreviewed"));
  return parts.join(", ") || t("git.change.changed");
}

/** The text a change shows: the translation after it, or before it for a removal. */
export function changeText(change: EntryChangeDto): string {
  return change.kind === "cleared" ? change.before.targetMacro : change.after.targetMacro;
}

export function versionText(version: EntryVersionDto): string {
  return version.targetMacro;
}

/** Whether a change needs its own "what changed" note: a changed translation
 * says it with its letter; marks and notes need words. */
function changeNote(change: EntryChangeDto, t: Translate): string | null {
  const marked = change.before.fuzzy !== change.after.fuzzy
    || change.before.translatorNote !== change.after.translatorNote
    || Boolean(change.before.reviewed) !== Boolean(change.after.reviewed);
  return change.kind === "marked" || marked ? changeLabel(change, t) : null;
}

export function ChangeRow({ change, selected, showColumn = true, onOpen }: { change: EntryChangeDto; selected: boolean; showColumn?: boolean; onOpen?: (() => void) | undefined }) {
  const { t } = useI18n();
  const binding = changeBinding(change);
  const text = changeText(change);
  const note = changeNote(change, t);
  const content = <>
    <span className={`git-kind git-kind-${kindClass[change.kind]}`} aria-label={t(kindLabel[change.kind])}>{kindLetter[change.kind]}</span>
    <span className="git-change-coord mono">{binding ? `${binding.rowId}:${binding.subrowId}` : "?"}{binding && showColumn ? <small>{` · ${binding.columnIndex}`}</small> : null}</span>
    <span className={change.kind === "cleared" ? "git-change-text removed" : "git-change-text"}>{text || <em>{t("git.emptyText")}</em>}</span>
    {note ? <span className="git-change-meta">{note}</span> : null}
  </>;
  const title = [bindingLabel(binding, t), t(kindLabel[change.kind]), text].filter(Boolean).join("\n");
  return onOpen
    ? <button type="button" className={selected ? "git-change selected" : "git-change"} onClick={onOpen} title={title}>{content}</button>
    : <div className={selected ? "git-change selected" : "git-change"} title={title}>{content}</div>;
}

/** The key a change is selected and listed by: its string's coordinate, or its `msgctxt`. */
export function changeKey(change: EntryChangeDto): string {
  return change.sourceBinding ? bindingKey(change.sourceBinding) : change.context;
}

/** A value kept per key for the life of the window. */
const stickyValues = new Map<string, unknown>();
export function useStickyState<T>(key: string, initial: T): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => (stickyValues.has(key) ? stickyValues.get(key) as T : initial));
  const update = useCallback((next: T) => {
    stickyValues.set(key, next);
    setValue(next);
  }, [key]);
  return [value, update];
}

const areaIcons: Record<ProjectArea, UiIconName> = {
  terms: "languages",
  knowledge: "messageSquare",
  projectSettings: "settings",
  packSettings: "arrowUpRight",
  fontSettings: "palette",
  fontFile: "palette",
  gitAttributes: "settings",
  feedWorkflow: "cloud",
  guardWorkflow: "shieldCheck",
};

/** The icon of a project file's area. */
export function areaIcon(area: ProjectArea): UiIconName {
  return areaIcons[area];
}

const VISIBLE_DETAILS = 12;

function formatSize(bytes: number, locale: string): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toLocaleString(locale, { maximumFractionDigits: 1 })} ${units[unit]}`;
}

/**
 * What changed inside a project file: terms by term, settings by field,
 * other text by line. A file that does not read as its format shows its
 * lines, and one that cannot be compared says so.
 */
export function ProjectDetails({ change }: { change: ProjectChangeDto }) {
  const { t, locale } = useI18n();
  const [expanded, setExpanded] = useState(false);
  if (change.area === "fontFile") {
    return <ul className="git-project-details"><li className="git-project-detail note">{t("git.files.size", { size: formatSize(change.size, locale) })}</li></ul>;
  }
  if (change.unreadable) {
    return <ul className="git-project-details"><li className="git-project-detail note">{t("git.files.unreadable")}</li></ul>;
  }
  if (change.details.length === 0) {
    return change.kind === "modified" ? <ul className="git-project-details"><li className="git-project-detail note">{t("git.files.formatting")}</li></ul> : null;
  }
  const shown = expanded ? change.details : change.details.slice(0, VISIBLE_DETAILS);
  return (
    <ul className="git-project-details">
      {change.byLine ? <li className="git-project-detail note">{t("git.files.byLine")}</li> : null}
      {shown.map((detail, index) => (
        // A detail without a label is a line of the file, shown as a diff line.
        <li className={detail.label ? "git-project-detail" : `git-project-detail line is-${detail.kind}`} key={`${detail.label}:${index}`}>
          <span className={`git-kind git-kind-${detail.kind}`}>{detail.kind === "added" ? "+" : detail.kind === "removed" ? "−" : "~"}</span>
          {detail.label ? <span className="git-project-label">{detail.label}</span> : null}
          <span className="git-project-values">
            {detail.before !== null && detail.after !== null ? <><del>{detail.before || t("common.empty")}</del><UiIcon icon="arrowRight" size="xs" /><ins>{detail.after || t("common.empty")}</ins></>
              : detail.after !== null ? <ins>{detail.after || t("common.empty")}</ins>
              : <del>{detail.before || t("common.empty")}</del>}
          </span>
        </li>
      ))}
      {change.details.length > VISIBLE_DETAILS ? (
        <li><button className="link-button" type="button" onClick={() => setExpanded(!expanded)}>{expanded ? t("git.project.showLess") : t("git.project.showAll", { count: change.details.length })}</button></li>
      ) : null}
      {change.truncated ? <li className="muted">{t("git.project.truncated")}</li> : null}
    </ul>
  );
}

export function Section({ title, icon, meta, action, children }: { title: string; icon: UiIconName; meta?: ReactNode; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="git-section" aria-label={title}>
      <header className="git-section-head">
        <UiIcon icon={icon} size="sm" />
        <strong>{title}</strong>
        {meta !== undefined ? <span className="git-count">{meta}</span> : null}
        <span className="spacer" />
        {action}
      </header>
      {children}
    </section>
  );
}
