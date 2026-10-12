import { forwardRef, useEffect, useImperativeHandle, useRef, type KeyboardEvent, type MouseEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { type GlossaryItem, type GlossarySort, type SortKey, type Unapplied, itemCategory, itemTerm, itemTranslation, unappliedOf, usageKey } from "../glossaryModel";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { RightClickMenu, type MenuEntry } from "../ui/primitives/RightClickMenu";
import { UiIcon } from "../ui/primitives/UiIcon";

const ROW_HEIGHT = 30;

export const problemLabels: Readonly<Record<NonNullable<Extract<GlossaryItem, { kind: "entry" }>["problem"]>, MessageKey>> = {
  emptyTerm: "glossary.problem.emptyTerm",
  emptyTranslation: "glossary.problem.emptyTranslation",
  emptyCategory: "glossary.problem.emptyCategory",
  duplicateTerm: "glossary.problem.duplicateTerm",
};

const COLUMNS: readonly { key: Exclude<SortKey, "order">; label: MessageKey; title?: MessageKey }[] = [
  { key: "term", label: "glossary.column.term" },
  { key: "translation", label: "glossary.column.translation" },
  { key: "category", label: "glossary.column.category" },
  { key: "source", label: "glossary.column.source" },
  { key: "uses", label: "glossary.column.uses", title: "glossary.column.usesTitle" },
];

export type GlossaryTableHandle = { focus: () => void };

type GlossaryTableProps = {
  items: readonly GlossaryItem[];
  /** The terms chosen, by id; the active one shows its fields. */
  selected: ReadonlySet<string>;
  active: string | null;
  sort: GlossarySort;
  onSort: (key: SortKey) => void;
  /** How many strings use each term; null until counted. */
  usage: ReadonlyMap<string, number> | null;
  counting: boolean;
  /** How many translations do not use each term; null until counted. */
  unapplied: Unapplied | null;
  /** Chooses `id`; `extend` adds a range from the active term, `toggle` adds or removes one. */
  onChoose: (id: string, how: "only" | "toggle" | "extend") => void;
  onSelectAll: () => void;
  onRemove: () => void;
  /** Moves into the active term's fields. */
  onEdit: () => void;
  menuFor: (item: GlossaryItem) => MenuEntry[];
  empty: string;
};

/**
 * The glossary as one dense, virtualized table: a term per row with its
 * translation, category, source, and use, each column ordered by its header.
 */
export const GlossaryTable = forwardRef<GlossaryTableHandle, GlossaryTableProps>(function GlossaryTable({ items, selected, active, sort, onSort, usage, counting, unapplied, onChoose, onSelectAll, onRemove, onEdit, menuFor, empty }, ref) {
  const { t } = useI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  useImperativeHandle(ref, () => ({ focus: () => scrollRef.current?.focus() }), []);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    getItemKey: (index) => items[index]!.id,
    overscan: 12,
  });

  const index = active === null ? -1 : items.findIndex((item) => item.id === active);
  useEffect(() => {
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
  }, [index, virtualizer]);

  const page = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? ROW_HEIGHT * 10) / ROW_HEIGHT) - 1);
  const move = (by: number, extend: boolean) => {
    if (items.length === 0) return;
    const from = index < 0 ? (by > 0 ? -1 : items.length) : index;
    const next = items[Math.min(items.length - 1, Math.max(0, from + by))];
    if (next) onChoose(next.id, extend ? "extend" : "only");
  };

  const keyDown = (event: KeyboardEvent) => {
    const extend = event.shiftKey;
    if (event.key === "ArrowDown") { event.preventDefault(); move(1, extend); }
    else if (event.key === "ArrowUp") { event.preventDefault(); move(-1, extend); }
    else if (event.key === "PageDown") { event.preventDefault(); move(page, extend); }
    else if (event.key === "PageUp") { event.preventDefault(); move(-page, extend); }
    else if (event.key === "Home") { event.preventDefault(); move(-items.length, extend); }
    else if (event.key === "End") { event.preventDefault(); move(items.length, extend); }
    else if (event.key === "Enter" && index >= 0) { event.preventDefault(); onEdit(); }
    else if (event.key === "Delete") { event.preventDefault(); onRemove(); }
    else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") { event.preventDefault(); onSelectAll(); }
  };

  const press = (event: MouseEvent, item: GlossaryItem) => {
    if (event.button !== 0) return;
    if (event.shiftKey) onChoose(item.id, "extend");
    else if (event.ctrlKey || event.metaKey) onChoose(item.id, "toggle");
    else onChoose(item.id, "only");
  };

  const sortIcon = (key: SortKey) => sort.key !== key ? null : <UiIcon icon={sort.descending ? "arrowDown" : "arrowUp"} size="xs" />;

  return (
    <div className="glossary-table">
      <div className="glossary-row glossary-head" role="row">
        {COLUMNS.map((column) => (
          <button
            key={column.key}
            type="button"
            className={`glossary-cell glossary-cell-${column.key}${sort.key === column.key ? " sorted" : ""}`}
            role="columnheader"
            aria-sort={sort.key !== column.key ? "none" : sort.descending ? "descending" : "ascending"}
            {...(column.title ? { title: t(column.title) } : {})}
            onClick={() => onSort(column.key)}
          >
            <span>{column.key === "uses" && counting ? t("glossary.counting") : t(column.label)}</span>
            {sortIcon(column.key)}
          </button>
        ))}
      </div>
      <div
        ref={scrollRef}
        className="glossary-body"
        role="grid"
        aria-multiselectable
        aria-rowcount={items.length}
        tabIndex={0}
        aria-label={t("glossary.tab")}
        aria-activedescendant={index >= 0 ? `glossary-row-${index}` : undefined}
        onKeyDown={keyDown}
      >
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((virtual) => {
            const item = items[virtual.index]!;
            const chosen = selected.has(item.id);
            const style = { position: "absolute", top: 0, left: 0, right: 0, height: ROW_HEIGHT, transform: `translateY(${virtual.start}px)` } as const;
            return (
              <RightClickMenu key={virtual.key} entries={() => menuFor(item)}>
                <div
                  id={`glossary-row-${virtual.index}`}
                  role="row"
                  aria-selected={chosen}
                  className={`glossary-row${chosen ? " selected" : ""}${item.id === active ? " active" : ""}${item.kind === "entry" && item.problem ? " invalid" : ""}`}
                  style={style}
                  onMouseDown={(event) => press(event, item)}
                  onDoubleClick={onEdit}
                >
                  <Cells item={item} usage={usage} unapplied={unapplied} />
                </div>
              </RightClickMenu>
            );
          })}
        </div>
        {items.length === 0 ? <p className="glossary-empty-list">{empty}</p> : null}
      </div>
    </div>
  );
});

/** The cells of one term. */
function Cells({ item, usage, unapplied }: { item: GlossaryItem; usage: ReadonlyMap<string, number> | null; unapplied: Unapplied | null }) {
  const { t } = useI18n();
  const term = itemTerm(item);
  const translation = itemTranslation(item);
  const uses = usage?.get(usageKey(term, itemCategory(item)));
  const forms = item.kind === "entry" ? item.row.forms : item.term.forms;
  const notApplied = unappliedOf(item, unapplied);
  return (
    <>
      <span role="gridcell" className="glossary-cell glossary-cell-term">
        {item.kind === "entry" && item.problem ? <span className="glossary-problem-mark" title={t(problemLabels[item.problem])}><UiIcon icon="circleAlert" size="xs" /></span> : null}
        <span className={term ? "glossary-headword" : "glossary-headword placeholder"}>{term || t("glossary.untitled")}</span>
        {forms.length > 0 ? <span className="glossary-forms-inline">{forms.join(", ")}</span> : null}
      </span>
      <span role="gridcell" className="glossary-cell glossary-cell-translation">
        <span className="glossary-translation">{translation || "—"}</span>
        {notApplied > 0 ? <span className="glossary-badge glossary-badge-unapplied">{t("glossary.badge.unapplied", { count: notApplied })}</span> : null}
        {item.kind === "entry" && item.redundant ? <span className="glossary-badge">{t("glossary.badge.same")}</span> : null}
      </span>
      <span role="gridcell" className="glossary-cell glossary-cell-category">{itemCategory(item) || "—"}</span>
      <span role="gridcell" className="glossary-cell glossary-cell-source">
        {item.kind === "sheet"
          ? <><UiIcon icon="table2" size="xs" /><span>{item.term.sheet}</span></>
          : <><UiIcon icon="bookMarked" size="xs" /><span>{item.replaced[0] ? item.replaced[0].sheet : t("glossary.source.project")}</span></>}
      </span>
      <span role="gridcell" className="glossary-cell glossary-cell-uses">{uses ?? ""}</span>
    </>
  );
}
