import { useState } from "react";
import { Popover } from "radix-ui";
import type { CategoryCount, GlossaryFilter, GlossaryView } from "../glossaryModel";
import type { MessageKey } from "../i18n/translate";
import type { TermSheetDto } from "../types";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { RightClickMenu } from "../ui/primitives/RightClickMenu";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { CategoryField } from "./CategoryField";

/** Categories shown before the rest are folded away. */
const CATEGORIES_SHOWN = 8;

const SOURCES: readonly { view: GlossaryView; label: MessageKey; icon: UiIconName }[] = [
  { view: "all", label: "glossary.view.all", icon: "bookMarked" },
  { view: "entries", label: "glossary.view.entries", icon: "pencil" },
  { view: "sheets", label: "glossary.view.sheets", icon: "table2" },
];

const ATTENTION: readonly { view: GlossaryView; label: MessageKey; icon: UiIconName }[] = [
  { view: "problems", label: "glossary.view.problems", icon: "circleAlert" },
  { view: "unapplied", label: "glossary.view.unapplied", icon: "sparkles" },
  { view: "redundant", label: "glossary.view.redundant", icon: "copy" },
];

type GlossaryFacetsProps = {
  filter: GlossaryFilter;
  onFilter: (filter: GlossaryFilter) => void;
  views: Readonly<Record<GlossaryView, number>>;
  /** Views whose count is still being found. */
  pending: ReadonlySet<GlossaryView>;
  categories: readonly CategoryCount[];
  sheets: readonly TermSheetDto[];
  sheetCounts: ReadonlyMap<string, number>;
  /** The project's sheets, to declare one of them. */
  sheetNames: readonly string[];
  /** Every category in use, offered for a new sheet. */
  categoryNames: readonly string[];
  disabled: boolean;
  onSheetsChange: (sheets: TermSheetDto[]) => void;
};

/**
 * What the glossary list shows: every term, the project's own, or the
 * sheets'; the terms that need attention; one category; one sheet.
 */
export function GlossaryFacets({ filter, onFilter, views, pending, categories, sheets, sheetCounts, sheetNames, categoryNames, disabled, onSheetsChange }: GlossaryFacetsProps) {
  const { t } = useI18n();
  const [allCategories, setAllCategories] = useState(false);
  const shownCategories = allCategories ? categories : categories.slice(0, CATEGORIES_SHOWN);
  // The chosen category stays in view when the rest are folded.
  const chosenHidden = filter.category !== null && !shownCategories.some((category) => category.key === filter.category);
  const listed = chosenHidden ? [...shownCategories, ...categories.filter((category) => category.key === filter.category)] : shownCategories;

  const facet = (key: string, label: string, icon: UiIconName, count: number, chosen: boolean, choose: () => void, tone?: "danger" | "warning", counting = false) => (
    <button key={key} type="button" className={`glossary-facet${chosen ? " active" : ""}${tone && count > 0 ? ` ${tone}` : ""}`} aria-pressed={chosen} aria-busy={counting} onClick={choose}>
      <UiIcon icon={icon} size="xs" />
      <span className="glossary-facet-label">{label}</span>
      {counting
        ? <span className="glossary-facet-count counting" title={t("glossary.counting")}><UiIcon icon="refreshCw" size="xs" /></span>
        : <span className="glossary-facet-count">{count.toLocaleString()}</span>}
    </button>
  );

  return (
    <nav className="glossary-facets" aria-label={t("glossary.views")}>
      <div className="glossary-facet-group">
        {SOURCES.map((source) => facet(source.view, t(source.label), source.icon, views[source.view], filter.view === source.view, () => onFilter({ ...filter, view: source.view })))}
      </div>
      <div className="glossary-facet-head">{t("glossary.attention")}</div>
      <div className="glossary-facet-group">
        {ATTENTION.map((item) => facet(item.view, t(item.label), item.icon, views[item.view], filter.view === item.view, () => onFilter({ ...filter, view: filter.view === item.view ? "all" : item.view }), item.view === "problems" ? "danger" : "warning", pending.has(item.view)))}
      </div>
      <div className="glossary-facet-head">{t("glossary.categories")}</div>
      <div className="glossary-facet-group">
        {listed.map((category) => facet(`category:${category.key}`, category.label, "listFilter", category.count, filter.category === category.key, () => onFilter({ ...filter, category: filter.category === category.key ? null : category.key })))}
        {categories.length > CATEGORIES_SHOWN ? (
          <button type="button" className="glossary-facet glossary-facet-more" onClick={() => setAllCategories((current) => !current)}>
            <UiIcon icon={allCategories ? "chevronsUp" : "chevronDown"} size="xs" />
            <span className="glossary-facet-label">{allCategories ? t("glossary.categories.less") : t("glossary.categories.more", { count: categories.length - CATEGORIES_SHOWN })}</span>
          </button>
        ) : null}
      </div>
      <div className="glossary-facet-head">
        <span>{t("glossary.sheets")}</span>
        <AddSheet sheets={sheets} sheetNames={sheetNames} categories={categoryNames} disabled={disabled} onAdd={(sheet) => { onSheetsChange([...sheets, sheet]); onFilter({ ...filter, sheet: sheet.sheet }); }} />
      </div>
      <div className="glossary-facet-group">
        {[...new Set(sheets.map((sheet) => sheet.sheet))].map((name) => (
          <RightClickMenu key={name} entries={() => [{ id: "remove", icon: "trash", danger: true, label: t("glossary.sheets.remove", { sheet: name }), disabled, run: () => onSheetsChange(sheets.filter((other) => other.sheet !== name)) }]}>
            {facet(`sheet:${name}`, name, "table2", sheetCounts.get(name) ?? 0, filter.sheet === name, () => onFilter({ ...filter, sheet: filter.sheet === name ? null : name }))}
          </RightClickMenu>
        ))}
      </div>
    </nav>
  );
}

/** Sheet names offered at once while one is typed. */
const SHEETS_SHOWN = 8;

/** A sheet chosen by name with its column and the category of its terms, added at once. */
function AddSheet({ sheets, sheetNames, categories, disabled, onAdd }: { sheets: readonly TermSheetDto[]; sheetNames: readonly string[]; categories: readonly string[]; disabled: boolean; onAdd: (sheet: TermSheetDto) => void }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [column, setColumn] = useState("0");
  const [category, setCategory] = useState("");
  const wanted = name.trim();
  const wantedColumn = /^\d+$/.test(column.trim()) ? Number(column.trim()) : null;
  const known = sheetNames.includes(wanted);
  const declared = wantedColumn !== null && sheets.some((sheet) => sheet.sheet === wanted && sheet.column === wantedColumn);
  const ready = known && !declared && wantedColumn !== null && category !== "";
  const needle = wanted.toLocaleLowerCase();
  const matches = known ? [] : sheetNames.filter((sheet) => sheet.toLocaleLowerCase().includes(needle)).slice(0, SHEETS_SHOWN);
  const add = () => {
    if (!ready || wantedColumn === null) return;
    onAdd({ sheet: wanted, column: wantedColumn, category });
    setName("");
    setColumn("0");
    setCategory("");
    setOpen(false);
  };
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <IconButton icon="plus" size="xs" label={t("glossary.sheets.add")} disabled={disabled} />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="menu-content glossary-add-sheet" align="start" side="right" sideOffset={6} collisionPadding={8}>
          <form onSubmit={(event) => { event.preventDefault(); add(); }}>
            <input
              className="input"
              value={name}
              placeholder={t("glossary.sheets.sheet")}
              aria-label={t("glossary.sheets.sheet")}
              spellCheck={false}
              autoFocus
              onChange={(event) => setName(event.target.value)}
              // Enter takes the first sheet offered.
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                if (!known && matches[0]) setName(matches[0]);
                else add();
              }}
            />
            {wanted !== "" && matches.length > 0 ? (
              <div className="glossary-add-sheet-matches">
                {matches.map((sheet) => (
                  <button key={sheet} type="button" className="menu-item" onClick={() => setName(sheet)}>
                    <UiIcon icon="table2" size="xs" />{sheet}
                  </button>
                ))}
              </div>
            ) : null}
            <label className="glossary-add-sheet-field">
              <span>{t("glossary.sheets.column")}</span>
              <input className="input" inputMode="numeric" value={column} aria-label={t("glossary.sheets.column")} onChange={(event) => setColumn(event.target.value)} />
            </label>
            <div className="glossary-add-sheet-field">
              <label htmlFor="glossary-add-sheet-category">{t("glossary.sheets.category")}</label>
              <CategoryField id="glossary-add-sheet-category" value={category} categories={categories} onCommit={setCategory} />
            </div>
            <button className="button button-primary" type="submit" disabled={!ready}>{t("glossary.sheets.addShort")}</button>
          </form>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
