import { useState, type ReactNode } from "react";
import type { ProjectArea, ProjectChangeDto, SourceBinding, UnitChangeDto, UnitVersionDto } from "../types";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

export type ChangeGroup = { sheetName: string; changes: UnitChangeDto[] };

export function changeBinding(change: UnitChangeDto): SourceBinding | null {
  return (change.after ?? change.before)?.sourceBinding ?? null;
}

/** Groups changes by sheet, ordered by sheet name then row coordinate. */
export function groupChanges(changes: readonly UnitChangeDto[], unknownSheet: string): ChangeGroup[] {
  const groups = new Map<string, UnitChangeDto[]>();
  for (const change of changes) {
    const sheetName = changeBinding(change)?.sheetName ?? unknownSheet;
    groups.set(sheetName, [...(groups.get(sheetName) ?? []), change]);
  }
  const order = (change: UnitChangeDto) => {
    const binding = changeBinding(change);
    return binding ? [binding.rowId, binding.subrowId, binding.columnIndex] : [0, 0, 0];
  };
  return [...groups].sort(([left], [right]) => left.localeCompare(right)).map(([sheetName, entries]) => ({
    sheetName,
    changes: entries.sort((left, right) => {
      const [a, b] = [order(left), order(right)];
      return a[0]! - b[0]! || a[1]! - b[1]! || a[2]! - b[2]!;
    }),
  }));
}

const kindLetter: Record<UnitChangeDto["kind"], string> = { added: "A", modified: "M", removed: "D" };
export const kindLabel: Record<UnitChangeDto["kind"], MessageKey> = { added: "git.kind.added", modified: "git.kind.modified", removed: "git.kind.removed" };

export function unitLabel(unit: UnitVersionDto | null, t: Translate): string {
  if (!unit) return t("git.unknownUnit");
  const binding = unit.sourceBinding;
  return t("common.cellLocation", { sheet: binding.sheetName, row: String(binding.rowId), subrow: String(binding.subrowId), column: String(binding.columnIndex) });
}

export function changeLabel(change: UnitChangeDto, t: Translate): string {
  if (change.kind === "added") return t("git.change.added");
  if (change.kind === "removed") return t("git.change.removed");
  const parts = [];
  if (change.targetChanged) parts.push(t("git.change.text"));
  if (change.reviewChanged) parts.push(t("git.change.review"));
  if (change.noteChanged) parts.push(t("git.change.note"));
  return parts.join(", ") || t("git.change.changed");
}

export function ChangeRow({ change, selected, onOpen }: { change: UnitChangeDto; selected: boolean; onOpen?: (() => void) | undefined }) {
  const { t } = useI18n();
  const binding = changeBinding(change);
  const text = change.after?.targetMacro ?? change.before?.targetMacro ?? "";
  const content = <>
    <span className={`git-kind git-kind-${change.kind}`} aria-label={t(kindLabel[change.kind])}>{kindLetter[change.kind]}</span>
    <span className="git-change-coord mono">{binding ? `${binding.rowId}:${binding.subrowId}` : "?"}{binding ? <small> {t("common.column", { column: String(binding.columnIndex) })}</small> : null}</span>
    <span className={change.kind === "removed" ? "git-change-text removed" : "git-change-text"}>{text || <em>{t("git.emptyText")}</em>}</span>
    <span className="git-change-meta">{changeLabel(change, t)}</span>
  </>;
  return onOpen
    ? <li><button type="button" className={selected ? "git-change selected" : "git-change"} onClick={onOpen} title={binding ? t("git.openUnit", { unit: unitLabel(change.after ?? change.before, t) }) : t("git.openString")}>{content}</button></li>
    : <li><div className={selected ? "git-change selected" : "git-change"}>{content}</div></li>;
}

/** Translation changes grouped by sheet, each group collapsible. */
export function TranslationChangeGroups({ changes, selectedUnitId, onRevealBinding }: { changes: readonly UnitChangeDto[]; selectedUnitId: string | null; onRevealBinding?: ((binding: SourceBinding) => void) | undefined }) {
  const { t } = useI18n();
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  return (
    <div className="git-change-groups">
      {groupChanges(changes, t("git.unknownSheet")).map((group) => {
        const closed = collapsed.has(group.sheetName);
        return (
          <div className="git-change-group" key={group.sheetName}>
            <button type="button" className="git-change-group-head" aria-expanded={!closed} onClick={() => setCollapsed((current) => {
              const next = new Set(current);
              if (next.has(group.sheetName)) next.delete(group.sheetName);
              else next.add(group.sheetName);
              return next;
            })}>
              <UiIcon icon={closed ? "chevronRight" : "chevronDown"} size="xs" />
              <UiIcon icon="table2" size="sm" />
              <span className="git-change-group-name">{group.sheetName}</span>
              <span className="git-count">{group.changes.length}</span>
            </button>
            {closed ? null : (
              <ul className="git-change-list">
                {group.changes.map((change) => {
                  const binding = changeBinding(change);
                  return <ChangeRow key={change.translationUnitId} change={change} selected={change.translationUnitId === selectedUnitId} onOpen={binding && onRevealBinding && change.kind !== "removed" ? () => onRevealBinding(binding) : undefined} />;
                })}
              </ul>
            )}
          </div>
        );
      })}
    </div>
  );
}

const areaInfo: Record<ProjectArea, { icon: UiIconName; title: MessageKey; order: number }> = {
  glossary: { icon: "languages", title: "git.area.glossary", order: 0 },
  guidance: { icon: "messageSquare", title: "git.area.guidance", order: 1 },
  packSettings: { icon: "arrowUpRight", title: "git.area.packSettings", order: 2 },
  fontSettings: { icon: "palette", title: "git.area.fonts", order: 3 },
  fontFile: { icon: "palette", title: "git.area.fonts", order: 3 },
  collaboration: { icon: "users", title: "git.area.collaboration", order: 4 },
  gitAttributes: { icon: "settings", title: "git.area.gitAttributes", order: 5 },
  feedWorkflow: { icon: "cloud", title: "git.area.feedWorkflow", order: 6 },
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
