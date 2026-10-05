import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { bindingKey } from "../binding";
import type { EntryChangeDto, EntryChangeKind, EntryVersionDto, ProjectArea, ProjectChangeDto, SourceBinding } from "../types";
import { COLLAPSE_THRESHOLD, changeBinding, flattenChanges, groupChanges, isFilterActive, kindCounts, type ChangeFilter, type ChangeGroup, type ChangeItem, type ChangeKind } from "../gitChanges";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

export { changeBinding, groupChanges, type ChangeGroup } from "../gitChanges";

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

/** How a change list was left: kept while the window lives, so reopening
 * the Git tab or a commit shows it the same way. */
type ChangeViewState = { filter: ChangeFilter; toggled: ReadonlyMap<string, boolean> };
const changeViews = new Map<string, ChangeViewState>();
const emptyView: ChangeViewState = { filter: { query: "", kind: "all" }, toggled: new Map() };

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

/** Height of one line of the change list, header or change. */
const CHANGE_ROW = 26;
/** Changes from which the search and kind filter are offered. */
const TOOLBAR_THRESHOLD = 12;

const kindFilterLabels: Record<ChangeKind, MessageKey> = { translated: "git.filter.translated", changed: "git.filter.changed", cleared: "git.filter.cleared", marked: "git.filter.marked" };

/**
 * Translation changes grouped by sheet: searchable, filterable by kind, with
 * sheets that collapse one by one or all together. Sheets start collapsed
 * when there are many changes. The list is virtualized, so thousands of
 * changes stay responsive.
 */
export function TranslationChangeGroups({ changes, selectedKey, onRevealBinding, viewKey }: { changes: readonly EntryChangeDto[]; selectedKey: string | null; onRevealBinding?: ((binding: SourceBinding) => void) | undefined; viewKey?: string | undefined }) {
  const { t, formatNumber } = useI18n();
  const [view, setViewState] = useState<ChangeViewState>(() => (viewKey ? changeViews.get(viewKey) : undefined) ?? emptyView);
  const setView = (next: ChangeViewState) => {
    if (viewKey) changeViews.set(viewKey, next);
    setViewState(next);
  };
  const groups = useMemo(() => groupChanges(changes, t("git.unknownSheet")), [changes, t]);
  const counts = useMemo(() => kindCounts(changes), [changes]);
  const closedByDefault = changes.length > COLLAPSE_THRESHOLD && groups.length > 1;
  const isClosed = (sheetName: string) => !(view.toggled.get(sheetName) ?? !closedByDefault);
  const items = useMemo(
    () => flattenChanges(groups, view.filter, (sheetName) => !(view.toggled.get(sheetName) ?? !closedByDefault)),
    [groups, view, closedByDefault],
  );
  const filtering = isFilterActive(view.filter);
  const anyOpen = !filtering && groups.some((group) => !isClosed(group.sheetName));

  const toggle = (group: ChangeGroup) => {
    const toggled = new Map(view.toggled);
    toggled.set(group.sheetName, isClosed(group.sheetName));
    setView({ ...view, toggled });
  };
  const setAll = (open: boolean) => setView({ ...view, toggled: new Map(groups.map((group) => [group.sheetName, open])) });

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({ count: items.length, getScrollElement: () => scrollRef.current, estimateSize: () => CHANGE_ROW, overscan: 16 });
  const selectedIndex = selectedKey === null ? -1 : items.findIndex((item) => item.type === "change" && changeKey(item.change) === selectedKey);
  useEffect(() => {
    if (selectedIndex >= 0) virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
  }, [selectedIndex, virtualizer]);

  // The sheet whose changes fill the top of the list, pinned above them once
  // its own header scrolled away.
  const topItem = items[Math.floor((virtualizer.scrollOffset ?? 0) / CHANGE_ROW)];
  const pinned = topItem?.type === "change" ? topItem.group : null;
  const pinnedItem = pinned ? items.find((item): item is Extract<ChangeItem, { type: "group" }> => item.type === "group" && item.group === pinned) : undefined;

  const kinds = (["translated", "changed", "cleared", "marked"] as const).filter((kind) => counts[kind] > 0);
  const showToolbar = changes.length >= TOOLBAR_THRESHOLD;
  const renderItem = (item: ChangeItem) => {
    if (item.type === "group") {
      const { group } = item;
      return (
        <button type="button" className="git-change-group-head" aria-expanded={!item.closed} disabled={filtering} onClick={() => toggle(group)}>
          <UiIcon icon={item.closed ? "chevronRight" : "chevronDown"} size="xs" />
          <UiIcon icon="table2" size="sm" />
          <span className="git-change-group-name">{group.sheetName}</span>
          <span className="git-count">{filtering ? `${formatNumber(item.shown)} / ${formatNumber(group.changes.length)}` : formatNumber(group.changes.length)}</span>
        </button>
      );
    }
    const binding = changeBinding(item.change);
    return <ChangeRow change={item.change} selected={changeKey(item.change) === selectedKey} showColumn={item.group.multiColumn} onOpen={binding && onRevealBinding ? () => onRevealBinding(binding) : undefined} />;
  };

  return (
    <div className="git-changes">
      {showToolbar ? (
        <div className="git-change-toolbar">
          <label className="git-change-search">
            <UiIcon icon="search" size="xs" />
            <input className="input" type="search" value={view.filter.query} placeholder={t("git.filter.search")} aria-label={t("git.filter.search")} onChange={(event) => setView({ ...view, filter: { ...view.filter, query: event.target.value } })} />
          </label>
          {groups.length > 1 ? <IconButton icon={anyOpen ? "chevronsUp" : "chevronDown"} label={t(anyOpen ? "git.collapseAll" : "git.expandAll")} disabled={filtering} onClick={() => setAll(!anyOpen)} /> : null}
          {kinds.length > 1 ? (
            <Segmented<ChangeKind | "all">
              value={view.filter.kind}
              label={t("git.filter.kind")}
              onChange={(kind) => setView({ ...view, filter: { ...view.filter, kind } })}
              options={[
                { value: "all", label: `${t("git.filter.all")} ${formatNumber(changes.length)}` },
                ...kinds.map((kind) => ({
                  value: kind,
                  title: t(kindFilterLabels[kind]),
                  label: <><span className={`git-kind git-kind-${kindClass[kind]}`} aria-hidden="true">{kindLetter[kind]}</span>{` ${formatNumber(counts[kind])}`}</>,
                })),
              ]}
            />
          ) : null}
        </div>
      ) : null}
      {items.length === 0 ? <p className="muted">{t("git.filter.nothing")}</p> : (
        <div className="git-change-scroll" ref={scrollRef} role="list" style={{ height: Math.min(items.length * CHANGE_ROW, 480) }}>
          {pinnedItem ? <div className="git-change-pinned">{renderItem(pinnedItem)}</div> : null}
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((row) => {
              const item = items[row.index]!;
              return (
                <div key={item.type === "group" ? `g:${item.group.sheetName}` : changeKey(item.change)} role="listitem" className="git-change-slot" style={{ transform: `translateY(${row.start}px)`, height: CHANGE_ROW }}>
                  {renderItem(item)}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

const areaInfo: Record<ProjectArea, { icon: UiIconName; title: MessageKey; order: number }> = {
  terms: { icon: "languages", title: "git.area.terms", order: 0 },
  knowledge: { icon: "messageSquare", title: "git.area.knowledge", order: 1 },
  projectSettings: { icon: "settings", title: "git.area.projectSettings", order: 1 },
  packSettings: { icon: "arrowUpRight", title: "git.area.packSettings", order: 2 },
  fontSettings: { icon: "palette", title: "git.area.fonts", order: 3 },
  fontFile: { icon: "palette", title: "git.area.fonts", order: 3 },
  gitAttributes: { icon: "settings", title: "git.area.gitAttributes", order: 5 },
  feedWorkflow: { icon: "cloud", title: "git.area.feedWorkflow", order: 6 },
  checkWorkflow: { icon: "circleCheck", title: "git.area.checkWorkflow", order: 7 },
};

const VISIBLE_DETAILS = 12;

function formatSize(bytes: number, locale: string): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toLocaleString(locale, { maximumFractionDigits: 1 })} ${units[unit]}`;
}

function ProjectChangeItem({ change }: { change: ProjectChangeDto }) {
  const { t, locale } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const fileName = change.path.split("/").at(-1) ?? change.path;
  if (change.area === "fontFile") {
    return (
      <li className="git-project-detail">
        <span className={`git-kind git-kind-${change.kind}`}>{change.kind === "added" ? "+" : change.kind === "removed" ? "−" : "~"}</span>
        <span>{t(change.kind === "added" ? "git.project.fileAdded" : change.kind === "removed" ? "git.project.fileRemoved" : "git.project.fileChanged", { file: fileName, size: formatSize(change.size, locale) })}</span>
      </li>
    );
  }
  if (change.area === "feedWorkflow") {
    return (
      <li className="git-project-detail">
        <span className={`git-kind git-kind-${change.kind}`}>{change.kind === "added" ? "+" : change.kind === "removed" ? "−" : "~"}</span>
        <span>{t(change.kind === "added" ? "git.project.workflowAdded" : change.kind === "removed" ? "git.project.workflowRemoved" : "git.project.workflowUpdated")}</span>
      </li>
    );
  }
  if (change.unreadable) {
    return <li className="git-project-detail muted">{t("git.project.unreadable", { file: change.path })}</li>;
  }
  if (change.details.length === 0) {
    return <li className="git-project-detail muted">{t(change.kind === "added" ? "git.project.created" : change.kind === "removed" ? "git.project.deleted" : "git.project.formatting", { file: change.path })}</li>;
  }
  const shown = expanded ? change.details : change.details.slice(0, VISIBLE_DETAILS);
  return <>
    {change.kind !== "modified" ? <li className="git-project-detail muted">{t(change.kind === "added" ? "git.project.created" : "git.project.deleted", { file: change.path })}</li> : null}
    {shown.map((detail, index) => (
      <li className="git-project-detail" key={`${detail.label}:${index}`}>
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
  </>;
}

/** Glossary, guidance, settings, and font file changes, grouped by area. */
export function ProjectChangeList({ changes }: { changes: readonly ProjectChangeDto[] }) {
  const { t } = useI18n();
  const groups = new Map<MessageKey, { icon: UiIconName; order: number; changes: ProjectChangeDto[] }>();
  for (const change of changes) {
    const info = areaInfo[change.area];
    const group = groups.get(info.title) ?? { icon: info.icon, order: info.order, changes: [] };
    group.changes.push(change);
    groups.set(info.title, group);
  }
  return (
    <div className="git-change-groups">
      {[...groups].sort(([, a], [, b]) => a.order - b.order).map(([title, group]) => (
        <div className="git-change-group" key={title}>
          <div className="git-change-group-head static">
            <UiIcon icon={group.icon} size="sm" />
            <span className="git-change-group-name">{t(title)}</span>
          </div>
          <ul className="git-project-details">
            {group.changes.map((change) => <ProjectChangeItem key={change.path} change={change} />)}
          </ul>
        </div>
      ))}
    </div>
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
