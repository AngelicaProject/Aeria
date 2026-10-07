import { useEffect, useState, type ReactNode } from "react";
import { RightClickMenu, copyText, type MenuEntry } from "../ui/primitives/RightClickMenu";
import type { EntryChangeDto, FileChangeDto, FileGroup, GitFileKind, SourceBinding } from "../types";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";
import { ChangeRow, ProjectDetails, areaIcon, changeBinding, changeKey, useStickyState } from "./GitShared";

/** The letter and colour of how a file changed, as Git status shows it. */
const kindMark: Record<GitFileKind, { letter: string; tone: string }> = {
  added: { letter: "A", tone: "added" },
  modified: { letter: "M", tone: "modified" },
  deleted: { letter: "D", tone: "removed" },
  renamed: { letter: "R", tone: "modified" },
  copied: { letter: "C", tone: "added" },
  typeChanged: { letter: "T", tone: "modified" },
  untracked: { letter: "U", tone: "added" },
  conflicted: { letter: "!", tone: "removed" },
};

export const fileKindLabel: Record<GitFileKind, MessageKey> = {
  added: "git.file.added",
  modified: "git.file.modified",
  deleted: "git.file.deleted",
  renamed: "git.file.renamed",
  copied: "git.file.copied",
  typeChanged: "git.file.typeChanged",
  untracked: "git.file.untracked",
  conflicted: "git.file.conflicted",
};

/** A list of the file list: staged changes, or a group of the working tree's or a commit's files. */
type Section = "staged" | FileGroup;

const sections: readonly { section: Section; title: MessageKey; icon: UiIconName; hint?: MessageKey }[] = [
  { section: "staged", title: "git.files.staged", icon: "check" },
  { section: "translations", title: "git.files.translations", icon: "languages" },
  { section: "project", title: "git.files.project", icon: "settings" },
  { section: "other", title: "git.files.other", icon: "eyeOff", hint: "git.files.otherHint" },
];

/** What can be done with uncommitted files; absent for a commit's files. */
export type FileActions = {
  stage: (files: readonly FileChangeDto[]) => void;
  unstage: (files: readonly FileChangeDto[]) => void;
  discard: (files: readonly FileChangeDto[]) => void;
  disabled: boolean;
};

/** Files shown in a group before **Show more**. */
const FILES_SHOWN = 200;
/** String changes shown in an open PO file before **Show more**. */
const STRINGS_SHOWN = 300;
/** Files from which the path filter is offered. */
const FILTER_THRESHOLD = 12;

type FileChangeListProps = {
  /** The working tree's changes, or a commit's files. */
  files: readonly FileChangeDto[];
  /** What the index holds; only for the working tree. */
  staged?: readonly FileChangeDto[] | undefined;
  /** The string changes of one PO file, read when it is opened. */
  stringsOf: (path: string, staged: boolean) => Promise<EntryChangeDto[]>;
  actions?: FileActions | undefined;
  /** Shows the commits that changed a file. */
  onHistory?: ((path: string) => void) | undefined;
  /** Changes when the files may have changed, so open files read again. */
  revision?: number | undefined;
  selectedKey: string | null;
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
  /** Keeps which files and groups are open while the window lives. */
  viewKey: string;
};

/**
 * Changed files as Git sees them. For the working tree, the staged changes
 * come first, then the working tree's changes in three groups by what a
 * commit does with them: translations, project files, and other files. A PO
 * file opens to its string changes, a project file to its readable change.
 */
export function FileChangeList({ files, staged = [], stringsOf, actions, onHistory, revision = 0, selectedKey, onRevealBinding, viewKey }: FileChangeListProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [closedGroups, setClosedGroups] = useStickyState<readonly Section[]>(`${viewKey}:closedGroups`, []);
  const [toggled, setToggled] = useStickyState<ReadonlyMap<string, boolean>>(`${viewKey}:files`, new Map());
  const [limits, setLimits] = useState<ReadonlyMap<Section, number>>(new Map());
  const needle = query.trim().toLowerCase();
  const matches = (file: FileChangeDto) => !needle || file.path.toLowerCase().includes(needle) || Boolean(file.originalPath?.toLowerCase().includes(needle));
  const shownStaged = staged.filter(matches);
  const shownFiles = files.filter(matches);
  const total = files.length + staged.length;

  const openKey = (section: Section, file: FileChangeDto) => `${section === "staged" ? "staged" : "changes"}:${file.path}`;
  // Project files start open: their change is the reason to look.
  const isOpen = (section: Section, file: FileChangeDto) => toggled.get(openKey(section, file)) ?? (file.group === "project" && file.project !== null);
  const toggle = (section: Section, file: FileChangeDto) => setToggled(new Map(toggled).set(openKey(section, file), !isOpen(section, file)));

  return (
    <div className="git-files">
      {total >= FILTER_THRESHOLD ? (
        <label className="git-change-search">
          <UiIcon icon="search" size="xs" />
          <input className="input" type="search" value={query} placeholder={t("git.files.filter")} aria-label={t("git.files.filter")} onChange={(event) => setQuery(event.target.value)} />
        </label>
      ) : null}
      {needle && shownFiles.length + shownStaged.length === 0 ? <p className="muted">{t("git.filter.nothing")}</p> : null}
      {sections.map(({ section, title, icon, hint }) => {
        const members = section === "staged" ? shownStaged : shownFiles.filter((file) => file.group === section);
        if (members.length === 0) return null;
        const closed = closedGroups.includes(section);
        const strings = section === "translations" || section === "staged" ? members.reduce((sum, file) => sum + (file.strings ?? 0), 0) : 0;
        const limit = limits.get(section) ?? FILES_SHOWN;
        const actionable = members.filter((file) => file.kind !== "conflicted");
        const groupActions = actions && actionable.length > 0 ? (
          section === "staged"
            ? <IconButton icon="minus" size="xs" label={t("git.action.unstageAll")} disabled={actions.disabled} onClick={() => actions.unstage(actionable)} />
            : <>
              <IconButton icon="undo" size="xs" label={t("git.action.discardAll")} disabled={actions.disabled} onClick={() => actions.discard(actionable)} />
              <IconButton icon="plus" size="xs" label={t("git.action.stageAll")} disabled={actions.disabled} onClick={() => actions.stage(actionable)} />
            </>
        ) : null;
        return (
          <section className={`git-file-group git-file-group-${section}`} key={section}>
            <div className="git-file-group-head">
              <button type="button" className="git-change-group-head" aria-expanded={!closed} title={hint ? t(hint) : undefined} onClick={() => setClosedGroups(closed ? closedGroups.filter((other) => other !== section) : [...closedGroups, section])}>
                <UiIcon icon={closed ? "chevronRight" : "chevronDown"} size="xs" />
                <UiIcon icon={icon} size="sm" />
                <span className="git-change-group-name">{t(title)}</span>
                {strings > 0 ? <span className="git-file-meta">{t("git.files.strings", { count: strings })}</span> : null}
              </button>
              {groupActions ? <span className="git-file-actions">{groupActions}</span> : null}
              <span className="git-count">{members.length}</span>
            </div>
            {closed ? null : <>
              {hint && actions ? <p className="git-file-hint">{t(hint)}</p> : null}
              <ul className="git-file-list">
                {members.slice(0, limit).map((file) => (
                  <FileRow
                    key={file.path}
                    file={file}
                    staged={section === "staged"}
                    open={isOpen(section, file)}
                    onToggle={() => toggle(section, file)}
                    stringsOf={stringsOf}
                    actions={actions}
                    onHistory={onHistory}
                    revision={revision}
                    selectedKey={selectedKey}
                    onRevealBinding={onRevealBinding}
                  />
                ))}
              </ul>
              {members.length > limit ? (
                <button className="link-button git-file-more" type="button" onClick={() => setLimits(new Map(limits).set(section, limit + FILES_SHOWN))}>{t("git.files.showMore", { count: members.length - limit })}</button>
              ) : null}
            </>}
          </section>
        );
      })}
    </div>
  );
}

type FileRowProps = {
  file: FileChangeDto;
  /** The row stands for the file's staged change. */
  staged: boolean;
  open: boolean;
  onToggle: () => void;
  stringsOf: (path: string, staged: boolean) => Promise<EntryChangeDto[]>;
  actions?: FileActions | undefined;
  onHistory?: ((path: string) => void) | undefined;
  revision: number;
  selectedKey: string | null;
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
};

function FileRow({ file, staged, open, onToggle, stringsOf, actions, onHistory, revision, selectedKey, onRevealBinding }: FileRowProps) {
  const { t } = useI18n();
  const mark = kindMark[file.kind];
  const slash = file.path.lastIndexOf("/");
  const name = file.path.slice(slash + 1);
  const folder = slash >= 0 ? file.path.slice(0, slash) : "";
  const status = t(fileKindLabel[file.kind]);
  const opens = file.group === "translations" ? (file.strings ?? 0) > 0 : file.group === "project" && file.project !== null;
  const icon: UiIconName = file.project ? areaIcon(file.project.area) : file.group === "translations" ? "languages" : "fileDiff";
  const conflicted = file.kind === "conflicted";
  const fileActions = conflicted ? undefined : actions;
  const rowActions = fileActions ? (
    staged
      ? <IconButton icon="minus" size="xs" label={t("git.action.unstage")} disabled={fileActions.disabled} onClick={() => fileActions.unstage([file])} />
      : <>
        <IconButton icon="undo" size="xs" label={t(file.kind === "untracked" ? "git.action.delete" : "git.action.discard")} disabled={fileActions.disabled} onClick={() => fileActions.discard([file])} />
        <IconButton icon="plus" size="xs" label={t("git.action.stage")} disabled={fileActions.disabled} onClick={() => fileActions.stage([file])} />
      </>
  ) : null;
  const head = <>
    <span className="git-file-toggle">{opens ? <UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" /> : null}</span>
    <UiIcon icon={icon} size="xs" />
    <span className="git-file-path">
      <span className="git-file-name">{name}</span>
      {folder ? <span className="git-file-folder">{folder}</span> : null}
      {file.originalPath ? <span className="git-file-folder">{t("git.files.from", { path: file.originalPath })}</span> : null}
    </span>
    {file.group === "translations" && !conflicted ? <span className="git-file-meta">{file.strings ? t("git.files.strings", { count: file.strings }) : t("git.files.noStrings")}</span> : null}
  </>;
  const title = `${file.path}\n${status}`;
  return (
    <FileMenu file={file} staged={staged} actions={fileActions} onHistory={onHistory}>
      <li className={open && opens ? "git-file open" : "git-file"}>
        <div className="git-file-row">
          {opens
            ? <button type="button" className="git-file-head" aria-expanded={open} title={title} onClick={onToggle}>{head}</button>
            : <div className="git-file-head static" title={title}>{head}</div>}
          {rowActions ? <span className="git-file-actions">{rowActions}</span> : null}
          <span className={`git-kind git-kind-${mark.tone}`} aria-label={status} title={status}>{mark.letter}</span>
        </div>
        {open && opens ? (
          file.group === "translations"
            ? <FileStrings path={file.path} staged={staged} stringsOf={stringsOf} revision={revision} selectedKey={selectedKey} onRevealBinding={onRevealBinding} />
            : file.project ? <ProjectDetails change={file.project} /> : null
        ) : null}
      </li>
    </FileMenu>
  );
}

/** The right-click menu of a file: the actions of its row, its history, and its path. */
function FileMenu({ file, staged, actions, onHistory, children }: { file: FileChangeDto; staged: boolean; actions?: FileActions | undefined; onHistory?: ((path: string) => void) | undefined; children: ReactNode }) {
  const { t } = useI18n();
  const entries = (): MenuEntry[] => [
    ...(actions ? [
      ...(staged
        ? [{ id: "unstage", icon: "minus" as const, label: t("git.action.unstage"), disabled: actions.disabled, run: () => actions.unstage([file]) }]
        : [
          { id: "stage", icon: "plus" as const, label: t("git.action.stage"), disabled: actions.disabled, run: () => actions.stage([file]) },
          { id: "discard", icon: "undo" as const, label: t(file.kind === "untracked" ? "git.action.delete" : "git.action.discard"), danger: true, disabled: actions.disabled, run: () => actions.discard([file]) },
        ]),
      { id: "separator", separator: true } as const,
    ] : []),
    ...(onHistory && file.kind !== "untracked" ? [{ id: "history", icon: "history" as const, label: t("git.action.fileHistory"), run: () => onHistory(file.path) }] : []),
    { id: "copyPath", icon: "copy", label: t("git.action.copyPath"), run: () => copyText(file.path) },
  ];
  return <RightClickMenu entries={entries}>{children}</RightClickMenu>;
}

function FileStrings({ path, staged, stringsOf, revision, selectedKey, onRevealBinding }: { path: string; staged: boolean; stringsOf: (path: string, staged: boolean) => Promise<EntryChangeDto[]>; revision: number; selectedKey: string | null; onRevealBinding?: ((binding: SourceBinding) => void) | undefined }) {
  const { t } = useI18n();
  const [changes, setChanges] = useState<EntryChangeDto[] | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [limit, setLimit] = useState(STRINGS_SHOWN);
  useEffect(() => {
    let cancelled = false;
    stringsOf(path, staged)
      .then((next) => { if (!cancelled) { setChanges(next); setFailed(null); } })
      .catch((caught: unknown) => { if (!cancelled) setFailed(caught instanceof Error ? caught.message : String((caught as { message?: string })?.message ?? caught)); });
    return () => { cancelled = true; };
  }, [path, staged, stringsOf, revision]);
  if (failed) return <p className="git-file-note git-hint">{failed}</p>;
  if (!changes) return <p className="git-file-note muted"><span className="spinner" /></p>;
  const multiColumn = new Set(changes.map((change) => changeBinding(change)?.columnIndex)).size > 1;
  return (
    <div className="git-file-strings" role="list">
      {changes.slice(0, limit).map((change) => {
        const binding = changeBinding(change);
        return (
          <div role="listitem" key={changeKey(change)}>
            <ChangeRow change={change} selected={changeKey(change) === selectedKey} showColumn={multiColumn} onOpen={binding && onRevealBinding ? () => onRevealBinding(binding) : undefined} />
          </div>
        );
      })}
      {changes.length > limit ? <button className="link-button git-file-more" type="button" onClick={() => setLimit(limit + STRINGS_SHOWN)}>{t("git.files.showMore", { count: changes.length - limit })}</button> : null}
    </div>
  );
}

/** Whether a commit would take anything: what is staged, or else the translations and project files. */
export function hasCommittable(files: readonly FileChangeDto[], staged: readonly FileChangeDto[]): boolean {
  return staged.length > 0 || files.some((file) => file.group !== "other");
}
