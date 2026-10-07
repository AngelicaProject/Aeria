import { useEffect, useMemo, useRef, useState, type Dispatch, type PointerEvent as ReactPointerEvent, type SetStateAction } from "react";
import { createPortal } from "react-dom";
import { useVirtualizer } from "@tanstack/react-virtual";
import { filterRows, folderPath, folderTree, inFolder, moveFolder, moveFolders, parentFolder, splitForms, type GlossaryRow, type RowProblem } from "../projectGuide";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { RightClickMenu, type MenuEntry } from "../ui/primitives/RightClickMenu";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";

export const problemLabels: Readonly<Record<RowProblem, MessageKey>> = {
  emptyTerm: "guide.problem.emptyTerm",
  emptyTranslation: "guide.problem.emptyTranslation",
  duplicateTerm: "guide.problem.duplicateTerm",
};

const ROW_HEIGHT = 46;
/** How far the pointer moves before a press becomes a drag, in pixels. */
const DRAG_THRESHOLD = 5;
/** How long a drop or a cancel animates, in milliseconds. */
const SETTLE_MS = 220;
/** How long a collapsed folder is hovered in a drag before it opens. */
const OPEN_DELAY_MS = 600;

/**
 * Terms being dragged onto a folder: the pointer, the folder under it
 * (`""` for All terms), and, once released, where the card settles.
 */
type TermDrag = {
  keys: number[];
  x: number;
  y: number;
  over: string | null;
  settle: "drop" | "cancel" | null;
};

/** The folder a drop target in the tree stands for, under a point. */
function folderAt(x: number, y: number): string | null {
  const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-folder]");
  return target ? target.dataset.dropFolder ?? null : null;
}

type GlossaryEditorProps = {
  rows: readonly GlossaryRow[];
  setRows: Dispatch<SetStateAction<GlossaryRow[]>>;
  problems: ReadonlyMap<number, RowProblem>;
  selected: number | null;
  onSelect: (key: number | null) => void;
  /** Adds an empty term in `folder` and selects it. */
  onAdd: (folder: string) => void;
  disabled: boolean;
};

/**
 * The glossary as folders, a list of terms, and the chosen term's fields.
 * Every change goes to `rows`, which the dialog saves.
 */
export function GlossaryEditor({ rows, setRows, problems, selected, onSelect, onAdd, disabled }: GlossaryEditorProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  // The folder shown, with its subfolders; `null` shows every term.
  const [folder, setFolder] = useState<string | null>(null);
  // Folders made here that have no term yet.
  const [newFolders, setNewFolders] = useState<string[]>([]);
  // Terms chosen together with Ctrl or Shift, to drag into a folder at once.
  const [marked, setMarked] = useState<ReadonlySet<number>>(new Set());
  const [drag, setDrag] = useState<TermDrag | null>(null);
  // The folder terms were just dropped on, which flashes.
  const [landed, setLanded] = useState<string | null>(null);
  // A press that became a drag is not also a click.
  const dragged = useRef(false);
  const stop = useRef<(() => void) | null>(null);
  useEffect(() => () => stop.current?.(), []);

  const filtered = useMemo(() => filterRows(rows, query, folder), [folder, query, rows]);
  const folders = useMemo(() => folderTree(rows, newFolders), [newFolders, rows]);
  const row = rows.find((candidate) => candidate.key === selected) ?? null;

  const change = (key: number, patch: Partial<GlossaryRow>) => {
    setRows((current) => current.map((candidate) => candidate.key === key ? { ...candidate, ...patch } : candidate));
  };

  // A folder renamed or removed takes its subfolders and terms along.
  const moveTo = (from: string, to: string) => {
    setRows((current) => moveFolder(current, from, to));
    setNewFolders((current) => moveFolders(current, from, to));
    setFolder((current) => current === null || !inFolder(current, from) ? current : folderPath(`${to}/${current.slice(from.length)}`) || null);
  };

  // Terms follow the pointer as a card and move into the folder they are
  // released on; Escape or a release elsewhere puts them back.
  const beginDrag = (event: ReactPointerEvent, keys: () => number[]) => {
    if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let moving: number[] | null = null;
    dragged.current = false;
    const settle = (kind: "drop" | "cancel", over: string | null) => {
      document.body.classList.remove("glossary-dragging");
      const target = over === null ? null : document.querySelector<HTMLElement>(`[data-drop-folder="${CSS.escape(over)}"]`);
      const box = target?.getBoundingClientRect();
      setDrag((current) => current && { ...current, settle: kind, over: null, ...(box ? { x: box.left + 40, y: box.top + box.height / 2 } : {}) });
      window.setTimeout(() => setDrag(null), SETTLE_MS);
    };
    const move = (next: PointerEvent) => {
      if (!moving) {
        if (Math.hypot(next.clientX - startX, next.clientY - startY) < DRAG_THRESHOLD) return;
        moving = keys();
        dragged.current = true;
        document.body.classList.add("glossary-dragging");
      }
      const terms = moving;
      setDrag({ keys: terms, x: next.clientX, y: next.clientY, over: folderAt(next.clientX, next.clientY), settle: null });
    };
    const up = (next: PointerEvent) => {
      cleanup();
      if (!moving) return;
      const over = folderAt(next.clientX, next.clientY);
      if (over === null) { settle("cancel", null); return; }
      const terms = new Set(moving);
      setRows((current) => current.map((candidate) => terms.has(candidate.key) ? { ...candidate, folder: over } : candidate));
      settle("drop", over);
      setLanded(over);
      window.setTimeout(() => setLanded((current) => current === over ? null : current), 700);
    };
    const key = (next: KeyboardEvent) => {
      if (next.key !== "Escape" || !moving) return;
      next.preventDefault();
      next.stopPropagation();
      cleanup();
      settle("cancel", null);
    };
    const cleanup = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      window.removeEventListener("keydown", key, true);
      stop.current = null;
    };
    stop.current?.();
    stop.current = () => { cleanup(); document.body.classList.remove("glossary-dragging"); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    window.addEventListener("keydown", key, true);
  };

  // The selection moves to the nearest term left, the next one first.
  const remove = (keys: readonly number[]) => {
    const gone = new Set(keys);
    const last = Math.max(...keys.map((key) => filtered.findIndex((candidate) => candidate.key === key)));
    const next = filtered.slice(last + 1).find((candidate) => !gone.has(candidate.key))
      ?? filtered.slice(0, Math.max(0, last)).reverse().find((candidate) => !gone.has(candidate.key))
      ?? null;
    setRows((current) => current.filter((candidate) => !gone.has(candidate.key)));
    setMarked(new Set());
    onSelect(next?.key ?? null);
  };

  // A right-click on one of several chosen terms acts on all of them.
  const termMenu = (key: number): MenuEntry[] => {
    const keys = marked.has(key) && marked.size > 1 ? [...marked] : [key];
    const moving = new Set(keys);
    const move = (to: string) => setRows((current) => current.map((candidate) => moving.has(candidate.key) ? { ...candidate, folder: to } : candidate));
    return [
      {
        id: "move",
        icon: "folderInput",
        label: t("guide.glossary.moveTo"),
        disabled,
        items: [
          { id: "top", icon: "bookMarked", label: t("guide.folders.top"), run: () => move("") },
          ...folders.map((node): MenuEntry => ({ id: `folder:${node.path}`, icon: "folder", label: node.path.split("/").join(" / "), run: () => move(node.path) })),
        ],
      },
      { id: "separator", separator: true },
      { id: "remove", icon: "trash", danger: true, label: keys.length > 1 ? t("guide.glossary.removeMany", { count: keys.length }) : t("guide.glossary.remove"), disabled, run: () => remove(keys) },
    ];
  };

  return (
    <div className="glossary">
      <FolderTree
        rows={rows}
        folders={folders}
        folder={folder}
        onFolder={setFolder}
        onCreate={(path) => setNewFolders((current) => [...current, path])}
        onMove={moveTo}
        over={drag?.settle === null ? drag.over : null}
        landed={landed}
      />
      <section className="glossary-list" aria-label={t("guide.tab.terms")}>
        <div className="glossary-list-toolbar">
          <span className="glossary-search">
            <UiIcon icon="search" size="xs" />
            <input className="input" type="search" value={query} placeholder={t("guide.glossary.filter")} aria-label={t("guide.glossary.filter")} onChange={(event) => setQuery(event.target.value)} />
          </span>
          <IconButton icon="plus" label={t("guide.glossary.add")} disabled={disabled} onClick={() => { setQuery(""); onAdd(folder ?? ""); }} />
        </div>
        <TermList
          rows={filtered}
          total={rows.length}
          problems={problems}
          selected={selected}
          marked={marked}
          dragging={drag?.settle === null ? drag.keys : null}
          onSelect={onSelect}
          onMark={setMarked}
          onDragBegin={beginDrag}
          wasDragged={() => { const was = dragged.current; dragged.current = false; return was; }}
          showFolders={folder === null}
          menuFor={termMenu}
        />
      </section>
      {row ? (
        <TermDetail
          key={row.key}
          row={row}
          problem={problems.get(row.key) ?? null}
          folders={folders.map((node) => node.path)}
          onChange={(patch) => change(row.key, patch)}
          onRemove={() => remove([row.key])}
        />
      ) : (
        <section className="glossary-detail glossary-detail-empty">
          <UiIcon icon="bookMarked" size="xl" />
          <span>{rows.length === 0 ? t("guide.glossary.empty") : t("guide.glossary.choose")}</span>
          <button className="button button-secondary" type="button" disabled={disabled} onClick={() => { setQuery(""); onAdd(folder ?? ""); }}><UiIcon icon="plus" size="sm" />{t("guide.glossary.add")}</button>
        </section>
      )}
      {drag ? <DragCard drag={drag} rows={rows} /> : null}
    </div>
  );
}

/** The card that follows the pointer while terms are dragged. */
function DragCard({ drag, rows }: { drag: TermDrag; rows: readonly GlossaryRow[] }) {
  const { t } = useI18n();
  const first = rows.find((row) => row.key === drag.keys[0]);
  const settle = drag.settle ? ` ${drag.settle}` : "";
  return createPortal(
    <div className={`glossary-drag${drag.keys.length > 1 ? " stacked" : ""}${drag.over !== null ? " over" : ""}${settle}`} style={{ transform: `translate(${drag.x}px, ${drag.y}px)` }} aria-hidden>
      <div className="glossary-drag-card">
        <span className="glossary-drag-head">{first?.term.trim() || t("guide.glossary.untitled")}</span>
        {first?.translation.trim() ? <span className="glossary-drag-translation">{first.translation.trim()}</span> : null}
        {drag.keys.length > 1 ? <span className="glossary-drag-count">{drag.keys.length}</span> : null}
      </div>
    </div>,
    document.body,
  );
}

type FolderTreeProps = {
  rows: readonly GlossaryRow[];
  folders: ReturnType<typeof folderTree>;
  folder: string | null;
  onFolder: (folder: string | null) => void;
  onCreate: (path: string) => void;
  onMove: (from: string, to: string) => void;
  /** The folder dragged terms are over; `""` for All terms. */
  over: string | null;
  /** The folder terms were just dropped on. */
  landed: string | null;
};

function FolderTree({ rows, folders, folder, onFolder, onCreate, onMove, over, landed }: FolderTreeProps) {
  const { t } = useI18n();
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [renaming, setRenaming] = useState<{ path: string; name: string } | null>(null);
  const [removing, setRemoving] = useState<(typeof folders)[number] | null>(null);

  // A collapsed folder held under a drag opens, so terms can go deeper.
  useEffect(() => {
    if (!over || !collapsed.has(over)) return;
    const timer = window.setTimeout(() => setCollapsed((current) => { const next = new Set(current); next.delete(over); return next; }), OPEN_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [collapsed, over]);

  // Folders inside a collapsed folder are hidden.
  const shown = folders.filter((node) => ![...collapsed].some((path) => node.path.startsWith(`${path}/`)));

  const create = (parent: string) => {
    const taken = new Set(folders.map((node) => node.path));
    const base = t("guide.folders.new");
    let name = base;
    for (let number = 2; taken.has(folderPath(`${parent}/${name}`)); number += 1) name = `${base} ${number}`;
    const path = folderPath(`${parent}/${name}`);
    onCreate(path);
    setCollapsed((current) => { const next = new Set(current); next.delete(parent); return next; });
    onFolder(path);
    setRenaming({ path, name });
  };

  const finishRename = () => {
    if (!renaming) return;
    const to = folderPath(`${parentFolder(renaming.path)}/${renaming.name.replaceAll("/", " ")}`);
    if (to && to !== renaming.path) {
      onMove(renaming.path, to);
      setCollapsed((current) => new Set([...current].map((path) => inFolder(path, renaming.path) ? folderPath(`${to}/${path.slice(renaming.path.length)}`) : path)));
    }
    setRenaming(null);
  };

  const toggle = (path: string) => {
    setCollapsed((current) => { const next = new Set(current); if (!next.delete(path)) next.add(path); return next; });
  };

  // A folder with terms or subfolders is removed only after asking.
  const askRemove = (node: (typeof folders)[number]) => {
    if (node.count === 0 && !folders.some((other) => other.path.startsWith(`${node.path}/`))) onMove(node.path, parentFolder(node.path));
    else setRemoving(node);
  };

  const rowClass = (path: string | null) => {
    const key = path ?? "";
    return `glossary-folder${folder === path ? " active" : ""}${over === key ? " drop" : ""}${landed === key ? " landed" : ""}`;
  };

  return (
    <nav className="glossary-folders" aria-label={t("guide.folders.label")}>
      <div className="glossary-pane-head">
        <span>{t("guide.folders.label")}</span>
        <IconButton icon="folderPlus" label={t("guide.folders.new")} onClick={() => create(folder ?? "")} />
      </div>
      <div className="glossary-folders-list">
        <RightClickMenu entries={() => [{ id: "new", icon: "folderPlus", label: t("guide.folders.new"), run: () => create("") }]}>
          <div className={rowClass(null)} data-drop-folder="">
            <span className="glossary-folder-toggle" />
            <button type="button" className="glossary-folder-name" onClick={() => onFolder(null)}>
              <UiIcon icon="bookMarked" size="xs" />
              <span>{t("guide.folders.all")}</span>
            </button>
            <span className="glossary-count">{rows.length}</span>
          </div>
        </RightClickMenu>
        {shown.map((node) => {
          const parent = folders.some((other) => other.path.startsWith(`${node.path}/`));
          const open = !collapsed.has(node.path);
          const indent = { paddingLeft: 4 + node.depth * 14 };
          if (renaming?.path === node.path) {
            return (
              <div key={node.path} className="glossary-folder active" style={indent}>
                <span className="glossary-folder-toggle" />
                <input
                  className="input glossary-folder-rename"
                  value={renaming.name}
                  aria-label={t("guide.folders.name")}
                  autoFocus
                  onFocus={(event) => event.target.select()}
                  onChange={(event) => setRenaming({ path: node.path, name: event.target.value })}
                  onBlur={finishRename}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") finishRename();
                    else if (event.key === "Escape") { event.stopPropagation(); setRenaming(null); }
                  }}
                />
              </div>
            );
          }
          return (
            <RightClickMenu
              key={node.path}
              entries={() => [
                { id: "new", icon: "folderPlus", label: t("guide.folders.newInside"), run: () => create(node.path) },
                { id: "rename", icon: "pencil", label: t("guide.folders.rename"), run: () => setRenaming({ path: node.path, name: node.name }) },
                { id: "separator", separator: true },
                { id: "remove", icon: "trash", danger: true, label: t("guide.folders.remove"), run: () => askRemove(node) },
              ]}
            >
              <div className={rowClass(node.path)} style={indent} data-drop-folder={node.path}>
                {parent ? (
                  <button type="button" className="glossary-folder-toggle" aria-label={node.name} aria-expanded={open} onClick={() => toggle(node.path)}>
                    <UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" />
                  </button>
                ) : <span className="glossary-folder-toggle" />}
                <button type="button" className="glossary-folder-name" onClick={() => onFolder(node.path)} onDoubleClick={() => setRenaming({ path: node.path, name: node.name })}>
                  <UiIcon icon={folder === node.path ? "folderOpen" : "folder"} size="xs" />
                  <span>{node.name}</span>
                </button>
                <span className="glossary-count">{node.count}</span>
                <span className="glossary-folder-actions">
                  <IconButton icon="pencil" size="xs" label={t("guide.folders.rename")} onClick={() => setRenaming({ path: node.path, name: node.name })} />
                  <IconButton icon="trash" size="xs" label={t("guide.folders.remove")} onClick={() => askRemove(node)} />
                </span>
              </div>
            </RightClickMenu>
          );
        })}
      </div>
      <ConfirmDialog
        open={removing !== null}
        title={t("guide.folders.removeTitle", { name: removing?.name ?? "" })}
        message={removing ? (parentFolder(removing.path)
          ? t("guide.folders.removeInto", { count: removing.count, parent: parentFolder(removing.path).split("/").join(" / ") })
          : t("guide.folders.removeToTop", { count: removing.count })) : ""}
        confirmLabel={t("guide.folders.removeConfirm")}
        cancelLabel={t("guide.cancel")}
        onKeepEditing={() => setRemoving(null)}
        onDiscard={() => { if (removing) onMove(removing.path, parentFolder(removing.path)); setRemoving(null); }}
      />
    </nav>
  );
}

type TermListProps = {
  rows: readonly GlossaryRow[];
  total: number;
  problems: ReadonlyMap<number, RowProblem>;
  selected: number | null;
  /** Terms chosen together with the selected one. */
  marked: ReadonlySet<number>;
  /** Terms being dragged, shown faded. */
  dragging: readonly number[] | null;
  onSelect: (key: number) => void;
  onMark: (keys: ReadonlySet<number>) => void;
  onDragBegin: (event: ReactPointerEvent, keys: () => number[]) => void;
  /** Whether the last press became a drag, which then is not a click. */
  wasDragged: () => boolean;
  /** Whether each term shows its folder: when the list spans folders. */
  showFolders: boolean;
  menuFor: (key: number) => MenuEntry[];
};

function TermList({ rows, total, problems, selected, marked, dragging, onSelect, onMark, onDragBegin, wasDragged, showFolders, menuFor }: TermListProps) {
  const { t } = useI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    getItemKey: (index) => rows[index]!.key,
    overscan: 10,
  });

  const index = rows.findIndex((row) => row.key === selected);
  useEffect(() => {
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
  }, [index, virtualizer]);

  const choose = (key: number) => {
    onSelect(key);
    onMark(new Set([key]));
  };

  const step = (by: number) => {
    const next = rows[Math.min(rows.length - 1, Math.max(0, (index < 0 ? (by > 0 ? -1 : rows.length) : index) + by))];
    if (next) choose(next.key);
  };

  // Ctrl adds or removes a term, Shift takes the terms from the selected one.
  const press = (event: MouseEvent, at: number, key: number) => {
    if (event.ctrlKey || event.metaKey) {
      const next = new Set(marked.size > 0 ? marked : selected === null ? [] : [selected]);
      if (!next.delete(key)) next.add(key);
      onMark(next);
      onSelect(key);
    } else if (event.shiftKey && index >= 0) {
      const [from, to] = at < index ? [at, index] : [index, at];
      onMark(new Set(rows.slice(from, to + 1).map((row) => row.key)));
      onSelect(key);
    } else if (!marked.has(key)) {
      choose(key);
    }
  };

  // The terms a drag takes: every chosen one when it starts on one of them.
  const taken = (key: number) => marked.has(key) && marked.size > 1 ? rows.filter((row) => marked.has(row.key)).map((row) => row.key) : [key];
  const moving = new Set(dragging ?? []);

  return (
    <>
      <div
        ref={scrollRef}
        className="glossary-terms"
        role="listbox"
        aria-multiselectable
        tabIndex={0}
        aria-label={t("guide.tab.terms")}
        aria-activedescendant={index >= 0 ? `glossary-term-${selected}` : undefined}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") { event.preventDefault(); step(1); }
          else if (event.key === "ArrowUp") { event.preventDefault(); step(-1); }
          else if (event.key === "Home") { event.preventDefault(); step(-rows.length); }
          else if (event.key === "End") { event.preventDefault(); step(rows.length); }
        }}
      >
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index]!;
            const problem = problems.get(row.key);
            return (
              <RightClickMenu key={item.key} entries={() => menuFor(row.key)}>
                <div
                  id={`glossary-term-${row.key}`}
                  role="option"
                  aria-selected={row.key === selected || marked.has(row.key)}
                  className={`glossary-term${row.key === selected ? " selected" : marked.has(row.key) ? " marked" : ""}${problem ? " invalid" : ""}${moving.has(row.key) ? " dragging" : ""}`}
                  style={{ position: "absolute", top: 0, left: 0, right: 0, height: ROW_HEIGHT, transform: `translateY(${item.start}px)` }}
                  onPointerDown={(event) => onDragBegin(event, () => taken(row.key))}
                  onMouseDown={(event) => press(event.nativeEvent, item.index, row.key)}
                  onClick={(event) => { if (!wasDragged() && !event.ctrlKey && !event.metaKey && !event.shiftKey) choose(row.key); }}
                >
                  <span className="glossary-term-line">
                    <span className={row.term.trim() ? "glossary-term-head" : "glossary-term-head placeholder"}>{row.term.trim() || t("guide.glossary.untitled")}</span>
                    {row.forms.length > 0 ? <span className="glossary-term-forms">{row.forms.join(", ")}</span> : null}
                    {problem ? <span className="glossary-term-problem" title={t(problemLabels[problem])}><UiIcon icon="circleAlert" size="xs" /></span> : null}
                  </span>
                  <span className="glossary-term-line">
                    <span className="glossary-term-translation">{row.translation.trim() || "—"}</span>
                    {showFolders && row.folder ? <span className="glossary-term-folder">{folderPath(row.folder).split("/").join(" / ")}</span> : null}
                  </span>
                </div>
              </RightClickMenu>
            );
          })}
        </div>
        {rows.length === 0 && total > 0 ? <p className="glossary-list-empty">{t("guide.glossary.noMatches")}</p> : null}
      </div>
      <div className="glossary-list-foot">{rows.length === total ? t("guide.glossary.count", { count: total }) : t("guide.glossary.shown", { count: rows.length, total })}</div>
    </>
  );
}

type TermDetailProps = {
  row: GlossaryRow;
  problem: RowProblem | null;
  folders: readonly string[];
  onChange: (patch: Partial<GlossaryRow>) => void;
  onRemove: () => void;
};

function TermDetail({ row, problem, folders, onChange, onRemove }: TermDetailProps) {
  const { t } = useI18n();
  const id = `glossary-${row.key}`;
  const options = useMemo(() => [
    { value: "", label: t("guide.folders.top") },
    ...folders.map((path) => ({ value: path, label: path.split("/").join(" / ") })),
  ], [folders, t]);
  return (
    <section className="glossary-detail" aria-label={row.term || t("guide.glossary.untitled")}>
      <div className="glossary-field">
        <label htmlFor={`${id}-term`}>{t("guide.glossary.term")}</label>
        <input id={`${id}-term`} className="input glossary-headword" value={row.term} autoFocus={!row.term} onChange={(event) => onChange({ term: event.target.value })} />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-forms`}>{t("guide.glossary.forms")}</label>
        <FormsInput id={`${id}-forms`} forms={row.forms} onChange={(forms) => onChange({ forms })} />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-translation`}>{t("guide.glossary.translation")}</label>
        <input id={`${id}-translation`} className="input" value={row.translation} onChange={(event) => onChange({ translation: event.target.value })} />
      </div>
      <div className="glossary-field glossary-field-grow">
        <label htmlFor={`${id}-note`}>{t("guide.glossary.note")}</label>
        <textarea id={`${id}-note`} className="input glossary-note" value={row.note} spellCheck onChange={(event) => onChange({ note: event.target.value })} />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-folder`}>{t("guide.glossary.folder")}</label>
        <Select id={`${id}-folder`} value={folderPath(row.folder)} options={options} onChange={(path) => onChange({ folder: path })} />
      </div>
      <label className="glossary-check" title={t("guide.terms.matchCaseHint")}>
        <input type="checkbox" checked={row.matchCase} onChange={(event) => onChange({ matchCase: event.target.checked })} />
        {t("guide.terms.matchCase")}
      </label>
      {problem ? <p className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{t(problemLabels[problem])}</p> : null}
      <div className="glossary-detail-actions">
        <button className="button button-ghost glossary-remove" type="button" onClick={onRemove}><UiIcon icon="trash" size="sm" />{t("guide.glossary.remove")}</button>
      </div>
    </section>
  );
}

/** The other forms of a term as chips; Enter or `;` adds what is typed. */
function FormsInput({ id, forms, onChange }: { id: string; forms: readonly string[]; onChange: (forms: string[]) => void }) {
  const { t } = useI18n();
  const [text, setText] = useState("");
  const commit = () => {
    const added = splitForms(text);
    if (added.length > 0) onChange([...forms, ...added]);
    setText("");
  };
  // A chip goes back into the field to be edited.
  const edit = (index: number) => {
    setText(forms[index] ?? "");
    onChange([...forms.filter((_, other) => other !== index), ...splitForms(text)]);
  };
  return (
    <div className="input glossary-forms" onMouseDown={(event) => { if (event.target === event.currentTarget) { event.preventDefault(); document.getElementById(id)?.focus(); } }}>
      {forms.map((form, index) => (
        <span key={index} className="glossary-form" title={t("guide.glossary.formEdit")} onDoubleClick={() => edit(index)}>
          {form}
          <button type="button" className="glossary-form-remove" aria-label={t("guide.glossary.formRemove", { form })} onClick={() => onChange(forms.filter((_, other) => other !== index))}><UiIcon icon="x" size="xs" /></button>
        </span>
      ))}
      <input
        id={id}
        className="glossary-form-input"
        value={text}
        placeholder={forms.length === 0 ? t("guide.glossary.formAdd") : undefined}
        onChange={(event) => setText(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === ";") {
            event.preventDefault();
            commit();
          } else if (event.key === "Backspace" && text === "" && forms.length > 0) {
            event.preventDefault();
            edit(forms.length - 1);
          }
        }}
      />
    </div>
  );
}
