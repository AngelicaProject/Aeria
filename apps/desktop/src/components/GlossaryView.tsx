import { forwardRef, useCallback, useDeferredValue, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import {
  type GlossaryFilter,
  type GlossaryItem,
  type GlossaryRow,
  type GlossarySort,
  type SortKey,
  categoryKey,
  duplicateOf,
  originsForRows,
  emptyGlossaryFilter,
  entryId,
  entryItems,
  facetCounts,
  filterItems,
  inputsFromRows,
  itemCategory,
  itemTerm,
  rowProblems,
  rowsChanged,
  rowsFromEntries,
  sheetItems,
  sortItems,
  outdatedOf,
  usageKey,
} from "../glossaryModel";
import { normalizeCommandError, projectKnowledge, saveKnowledgeSheets, saveKnowledgeTerms, stringBinding, termOutdated, termOutdatedStrings, termUsage, translationFix, translationStatus } from "../ipc";
import type { CommandError, GlossaryCategory, ProjectKnowledgeDto, SourceBinding, TermKey, TermSheetDto, TranslationStatus } from "../types";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import type { MenuEntry } from "../ui/primitives/RightClickMenu";
import { UiIcon } from "../ui/primitives/UiIcon";
import { NewCategoryDialog, MergeCategoriesDialog, type CategoryMergeDraft } from "./GlossaryCategoryDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";
import { BulkDetail, CategorySettings, EntryDetail, SheetSettings, SheetTermDetail, type Sense } from "./GlossaryDetail";
import { GlossaryFacets } from "./GlossaryFacets";
import { FixControls, type FixBlock, RunPanel, OutdatedStrings } from "./GlossaryFix";
import { GlossaryTable, type GlossaryTableHandle } from "./GlossaryTable";

export type GlossaryViewHandle = {
  save: () => void;
  focusSearch: () => void;
};

/** A term to show when the glossary opens, by its name and category. */
export type GlossaryFocus = { term: string; category: string; nonce: number };

type GlossaryViewProps = {
  /** Whether the glossary's tab is the one shown. */
  active: boolean;
  /** Changes when files of the project may have changed. */
  revision: number;
  sheetNames: readonly string[];
  onRevealBinding: (binding: SourceBinding) => void;
  onDirtyChange: (dirty: boolean) => void;
  /** A file of the knowledge was written. */
  onSaved: () => void;
  /** A correction run wrote translations. */
  onFilesChanged: () => void;
  /** Opens the settings of machine translation, to choose a model. */
  onOpenSettings: () => void;
  /** Opens a run's report in a tab of its own. */
  onOpenReport: (reportId: string, label: string) => void;
  focus: GlossaryFocus | null;
};

/** How often a correction run the glossary started is read, in milliseconds. */
const POLL_MS = 1000;

type Selection = { ids: ReadonlySet<string>; active: string | null };

const noSelection: Selection = { ids: new Set(), active: null };

/**
 * One editable glossary, with standalone and sheet-backed terms in one
 * table. Terms, independent categories and sheet defaults are saved together.
 */
export const GlossaryView = forwardRef<GlossaryViewHandle, GlossaryViewProps>(function GlossaryView({ active, revision, sheetNames, onRevealBinding, onDirtyChange, onSaved, onFilesChanged, onOpenSettings, onOpenReport, focus }, ref) {
  const { t } = useI18n();
  const { preferences } = usePreferences();
  const [saved, setSaved] = useState<ProjectKnowledgeDto | null>(null);
  const [rows, setRows] = useState<GlossaryRow[]>([]);
  const [categories, setCategories] = useState<GlossaryCategory[]>([]);
  const [sheets, setSheets] = useState<TermSheetDto[]>([]);
  const [merges, setMerges] = useState<{ from: string; into: string }[]>([]);
  const [addingCategory, setAddingCategory] = useState(false);
  const [categoryMerge, setCategoryMerge] = useState<CategoryMergeDraft | null>(null);
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  // The key of the row read from each entry of the saved file.
  const nextKey = useRef(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; error: CommandError } | null>(null);
  const [confirm, setConfirm] = useState<{ message: string; label?: string; run: () => void } | null>(null);
  const [filter, setFilter] = useState<GlossaryFilter>(emptyGlossaryFilter);
  const [sort, setSort] = useState<GlossarySort>({ key: "order", descending: false });
  const [selection, setSelection] = useState<Selection>(noSelection);
  const [usage, setUsage] = useState<ReadonlyMap<string, number> | null>(null);
  const [counting, setCounting] = useState(false);
  // How many translations were written with another version of each term, by name; null until counted.
  const [outdated, setOutdated] = useState<ReadonlyMap<string, number> | null>(null);
  const [checking, setChecking] = useState(false);
  // Counts read before the terms changed stay shown until counted again, so
  // a save neither empties a view of them nor reorders the list twice.
  const [usageStale, setUsageStale] = useState(false);
  const [outdatedStale, setOutdatedStale] = useState(false);
  const recount = useCallback(() => { setUsageStale(true); setOutdatedStale(true); }, []);
  // A correction run the glossary started, with the terms it follows, and
  // whether another run goes.
  const [run, setRun] = useState<{ terms: string[]; status: TranslationStatus | null } | null>(null);
  const [otherRun, setOtherRun] = useState(false);
  const [starting, setStarting] = useState(false);
  // Bumps when a run ends, so the lists of strings are read again.
  const [stringsRevision, setStringsRevision] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const tableRef = useRef<GlossaryTableHandle>(null);
  const detailRef = useRef<HTMLDivElement>(null);
  const loadedRevision = useRef<number | null>(null);

  // The rows shown keep their keys, so the chosen terms stay chosen.
  const show = useCallback((knowledge: ProjectKnowledgeDto) => {
    setSaved(knowledge);
    recount();
    const next = rowsFromEntries(knowledge.entries, rowsRef.current, knowledge.termIds);
    setCategories(knowledge.catalog?.categories ?? []);
    setSheets(knowledge.sheets);
    setMerges([]);
    nextKey.current = Math.max(nextKey.current, ...next.map((row) => row.key + 1));
    setRows(next);
  }, [recount]);

  const receive = useCallback((knowledge: ProjectKnowledgeDto) => {
    show(knowledge);
    if (knowledge.migrated) { loadedRevision.current = revision + 1; onSaved(); }
  }, [show, revision, onSaved]);

  const dirty = saved !== null && rowsChanged(rows, saved.entries, saved.termIds) || saved !== null && (JSON.stringify(categories) !== JSON.stringify(saved.catalog?.categories ?? []) || JSON.stringify(sheets) !== JSON.stringify(saved.sheets) || merges.length > 0);
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  // The files are read when the tab is first shown, and again when the
  // project changed while nothing is being edited.
  useEffect(() => {
    if (!active || loadedRevision.current === revision || dirty) return;
    loadedRevision.current = revision;
    void projectKnowledge(true).then(receive).catch((reason: unknown) => setError({ title: t("glossary.loadError"), error: normalizeCommandError(reason) }));
  }, [active, dirty, revision, receive, t]);

  const problems = useMemo(() => rowProblems(rows), [rows]);
  const fromSheets = useMemo(() => sheetItems(saved?.sheetTerms ?? []), [saved]);
  const origins = useMemo(() => originsForRows(rows, saved?.termIds ?? [], saved?.replaced ?? [], merges), [rows, saved, merges]);
  const entries = useMemo(() => entryItems(rows, problems, origins, [], rows.map((row) => row.key)), [problems, rows, origins]);
  const items = useMemo(() => [...entries, ...fromSheets], [entries, fromSheets]);
  const byId = useMemo(() => new Map(items.map((item) => [item.id, item])), [items]);
  const counted = useMemo(() => facetCounts(items, outdated), [items, outdated]);
  const counts = useMemo(() => {
    return { ...counted, categories: categories.map((category) => ({ key: categoryKey(category.name), label: category.name, count: counted.categories.find((c) => c.key === categoryKey(category.name))?.count ?? 0 })).sort((a, b) => b.count - a.count || a.label.localeCompare(b.label)) };
  }, [counted, categories]);
  const query = useDeferredValue(filter.query);
  // The active term stays in the list while it is edited and saved, though
  // it no longer matches; choosing another term or changing the filter
  // lets it go.
  const [kept, setKept] = useState<{ id: string | null; filter: GlossaryFilter }>({ id: selection.active, filter });
  if (kept.id !== selection.active) setKept({ id: selection.active, filter });
  const keep = kept.filter === filter ? kept.id : null;
  const shown = useMemo(() => sortItems(filterItems(items, { ...filter, query }, outdated, keep), sort, usage), [filter, items, keep, query, sort, outdated, usage]);
  const categoryNames = useMemo(() => categories.map((category) => category.name), [categories]);
  const noteFor = (name: string) => categories.find((category) => categoryKey(category.name) === categoryKey(name))?.note ?? "";

  const countUsage = useCallback(() => {
    if (counting) return;
    setCounting(true);
    setUsageStale(false);
    void termUsage()
      .then((found) => setUsage(new Map(found.map((term) => [usageKey(term.term, term.category), term.strings]))))
      // A failed count is not asked again until the terms are read again.
      .catch((reason: unknown) => { setUsage(new Map()); setError({ title: t("glossary.loadError"), error: normalizeCommandError(reason) }); })
      .finally(() => setCounting(false));
  }, [counting, t]);

  // The uses are counted once the terms are read, and again after each read.
  useEffect(() => {
    if (active && saved && (usage === null || usageStale) && !counting) countUsage();
  }, [active, countUsage, counting, saved, usage, usageStale]);

  // The translations written with another version of a term are counted
  // once the terms are read, and again after a run.
  const checkOutdated = useCallback(() => {
    setChecking(true);
    setOutdatedStale(false);
    void termOutdated()
      .then((found) => setOutdated(new Map(found.map((term) => [usageKey(term.term, term.category), term.strings]))))
      .catch((reason: unknown) => { setOutdated(new Map()); setError({ title: t("glossary.loadError"), error: normalizeCommandError(reason) }); })
      .finally(() => setChecking(false));
  }, [t]);
  useEffect(() => {
    if (active && saved && (outdated === null || outdatedStale) && !checking) checkOutdated();
  }, [active, checkOutdated, checking, saved, outdated, outdatedStale]);

  // A run of the glossary is read every second until it ends; then what it
  // wrote reaches the editor and the counts.
  const runGoing = run?.status?.running ?? run !== null;
  useEffect(() => {
    if (!run || run.status?.running === false) return;
    let current = true;
    const read = () => void translationStatus()
      .then((status) => {
        if (!current) return;
        setRun((value) => value && { ...value, status });
        if (status && !status.running) {
          onFilesChanged();
          recount();
          setStringsRevision((value) => value + 1);
        }
      })
      .catch((reason: unknown) => { if (current) setError({ title: t("glossary.fix.error"), error: normalizeCommandError(reason) }); });
    read();
    const timer = window.setInterval(read, POLL_MS);
    return () => { current = false; window.clearInterval(timer); };
  }, [onFilesChanged, recount, run?.status?.running, run !== null, t]);

  // Another run, from the machine translation dialog or search, is asked
  // about while the tab is shown.
  useEffect(() => {
    if (!active || runGoing) return;
    let current = true;
    const read = () => void translationStatus().then((status) => { if (current) setOtherRun(status?.running ?? false); }).catch(() => undefined);
    read();
    const timer = window.setInterval(read, POLL_MS * 3);
    return () => { current = false; window.clearInterval(timer); };
  }, [active, runGoing]);

  const block: FixBlock = !preferences.translationModel ? "noModel" : otherRun || runGoing ? "running" : dirty ? "unsaved" : null;

  /** Brings the translations written with another version of the meanings `terms`, or of any term when empty, to the terms as they are. */
  const fix = (terms: TermKey[], request: string | null) => {
    if (block !== null || starting) return;
    setStarting(true);
    setError(null);
    void (async () => {
      try {
        const strings = await termOutdatedStrings(terms);
        if (strings.length === 0) { checkOutdated(); return; }
        const paths = [...new Set(strings.map((string) => string.path))];
        const names = [...new Set(terms.map((term) => term.term))];
        await translationFix(paths, strings.map((string) => string.context), { issues: false, groups: [], adapt: false, request, terms: names }, preferences.translationModel, preferences.translationEffort || null);
        setRun({ terms: [...new Set(terms.map((term) => term.term))], status: null });
      } catch (reason) {
        setError({ title: t("glossary.fix.error"), error: normalizeCommandError(reason) });
      } finally {
        setStarting(false);
      }
    })();
  };

  // A header orders by its column, then the other way, then as the files have it.
  // Uses start with the most used.
  const sortBy = (key: SortKey) => {
    setSort((current) => {
      const first = key === "uses";
      if (current.key !== key) return { key, descending: first };
      if (current.descending === first) return { key, descending: !first };
      return { key: "order", descending: false };
    });
    if (key === "uses" && usage === null) countUsage();
  };

  // Choosing --------------------------------------------------------------

  const choose = (id: string, how: "only" | "toggle" | "extend") => {
    setSelection((current) => {
      if (how === "toggle") {
        const ids = new Set(current.ids);
        if (!ids.delete(id)) ids.add(id);
        return { ids, active: id };
      }
      if (how === "extend" && current.active !== null) {
        const from = shown.findIndex((item) => item.id === current.active);
        const to = shown.findIndex((item) => item.id === id);
        if (from >= 0 && to >= 0) {
          const [low, high] = from < to ? [from, to] : [to, from];
          return { ids: new Set(shown.slice(low, high + 1).map((item) => item.id)), active: current.active === id ? id : current.active };
        }
      }
      return { ids: new Set([id]), active: id };
    });
  };
  const chooseOnly = (id: string) => setSelection({ ids: new Set([id]), active: id });

  // Choosing a category or a sheet lets the chosen terms go, so the detail
  // shows its settings.
  const changeFilter = (next: GlossaryFilter) => {
    if ((next.category !== null && next.category !== filter.category) || (next.sheet !== null && next.sheet !== filter.sheet)) setSelection(noSelection);
    setFilter(next);
  };
  const openCategory = (category: string) => changeFilter({ ...filter, category: categoryKey(category), sheet: null });

  const activeItem = selection.active === null ? null : byId.get(selection.active) ?? null;
  const chosen = [...selection.ids].flatMap((id) => byId.get(id) ?? []);
  const chosenEntries = chosen.filter((item): item is Extract<GlossaryItem, { kind: "entry" }> => item.kind === "entry");

  // A term asked for from elsewhere is shown once, whatever the filter.
  const focused = useRef<number | null>(null);
  useEffect(() => {
    if (!focus || !saved || focused.current === focus.nonce) return;
    focused.current = focus.nonce;
    const name = focus.term.toLowerCase();
    const wanted = items.find((item) => itemTerm(item).toLowerCase() === name && categoryKey(itemCategory(item)) === categoryKey(focus.category))
      ?? items.find((item) => itemTerm(item).toLowerCase() === name);
    if (!wanted) return;
    setFilter(emptyGlossaryFilter);
    setSelection({ ids: new Set([wanted.id]), active: wanted.id });
  }, [focus, items, saved]);

  // Editing --------------------------------------------------------------

  const ensureCategory = (name: string) => {
    const trimmed = name.trim();
    if (trimmed && !categories.some((c) => categoryKey(c.name) === categoryKey(trimmed))) setCategories((current) => [...current, { id: `c-${crypto.randomUUID()}`, name: trimmed, note: "" }]);
  };
  const change = (key: number, patch: Partial<GlossaryRow>) => {
    if (patch.category) ensureCategory(patch.category);
    const row = rows.find((candidate) => candidate.key === key);
    const existing = row && !row.id && patch.category !== undefined ? duplicateOf(rows, { ...row, ...patch }) : undefined;
    if (existing) { setRows((current) => current.filter((candidate) => candidate.key !== key)); chooseOnly(entryId(existing.key)); return; }
    setRows((current) => current.map((row) => row.key === key ? { ...row, ...patch } : row));
  };

  const openExistingTerm = (key: number) => {
    const candidate = rows.find((row) => row.key === key);
    const existing = candidate && !candidate.id ? duplicateOf(rows, candidate) : undefined;
    if (existing) { setRows((current) => current.filter((row) => row.key !== key)); chooseOnly(entryId(existing.key)); }
  };

  // The new term stays in the list as the chosen one, under the filter as it
  // is. A search that found nothing names it.
  const add = (row: Partial<GlossaryRow> = {}) => {
    const key = nextKey.current++;
    const category = row.category ?? counts.categories.find((candidate) => candidate.key === filter.category)?.label ?? "";
    const term = shown.length === 0 ? filter.query.trim() : "";
    setRows((current) => [...current, { key, term, translation: "", forms: [], note: "", matchCase: false, ...row, category }]);
    chooseOnly(entryId(key));
  };

  // A term added by its button is typed at once: its name, or its
  // translation when the search named it.
  const focusAdded = useRef<string | null>(null);
  const addTerm = () => {
    focusAdded.current = shown.length === 0 && filter.query.trim() !== "" ? "[data-glossary-edit]" : "[data-glossary-headword]";
    add();
  };
  useEffect(() => {
    const field = focusAdded.current;
    if (field === null) return;
    focusAdded.current = null;
    detailRef.current?.querySelector<HTMLElement>(field)?.focus();
  });

  const remove = (keys: readonly number[]) => {
    if (keys.length === 0) return;
    const gone = new Set(keys.map(entryId));
    const at = shown.findIndex((item) => gone.has(item.id));
    const next = shown.slice(at + 1).find((item) => !gone.has(item.id)) ?? shown.slice(0, Math.max(0, at)).reverse().find((item) => !gone.has(item.id)) ?? null;
    const removing = new Set(keys.filter((key) => { const item = byId.get(entryId(key)); return item?.kind === "entry" && item.replaced.length === 0; }));
    setRows((current) => current.filter((row) => !removing.has(row.key)));
    setSelection(next ? { ids: new Set([next.id]), active: next.id } : noSelection);
  };

  const setCategory = (keys: readonly number[], category: string) => {
    ensureCategory(category);
    const changing = new Set(keys);
    setRows((current) => current.map((row) => changing.has(row.key) ? { ...row, category } : row));
  };

  const saveTerms = () => {
    if (!saved || !dirty || problems.size > 0 || busy || runGoing || otherRun) return;
    const write = () => {
      setBusy(true);
      setError(null);
      void saveKnowledgeTerms(saved.termsText, inputsFromRows(rows), categories, merges, sheets)
        // The save bumps the project's revision by one: what it wrote is shown already.
        .then((knowledge) => { show(knowledge); loadedRevision.current = revision + 1; onSaved(); })
        .catch((reason: unknown) => setError({ title: t("glossary.saveError"), error: normalizeCommandError(reason) }))
        .finally(() => setBusy(false));
    };
    if (saved.termsError && saved.termsText !== null) setConfirm({ message: t("glossary.replaceBroken"), run: write });
    else if (saved.diagnostics.length > 0) setConfirm({ message: t("glossary.dropExcluded", { count: saved.diagnostics.length }), run: write });
    else write();
  };

  const revert = () => {
    if (!saved) return;
    show(saved);
    setSelection(noSelection);
  };

  const saveSheets = (next: TermSheetDto[]) => {
    const missing = [...new Set(next.map((sheet) => sheet.category))].filter((name) => !categories.some((c) => categoryKey(c.name) === categoryKey(name)));
    if (missing.length) setCategories((current) => [...current, ...missing.map((name) => ({ id: `c-${crypto.randomUUID()}`, name, note: "" }))]);
    setSheets(next);
  };

  const renameCategory = (id: string, name: string) => {
    const existing = categories.find((c) => c.id !== id && categoryKey(c.name) === categoryKey(name));
    if (existing || !name.trim()) return;
    const old = categories.find((c) => c.id === id)!;
    setCategories((current) => current.map((c) => c.id === id ? { ...c, name: name.trim() } : c));
    setRows((current) => current.map((row) => categoryKey(row.category) === categoryKey(old.name) ? { ...row, category: name.trim() } : row));
    setSheets((current) => current.map((sheet) => categoryKey(sheet.category) === categoryKey(old.name) ? { ...sheet, category: name.trim() } : sheet));
    setFilter((current) => ({ ...current, category: categoryKey(name) }));
  };

  const beginMerge = (sourceId: string, targetId: string) => {
    const source = categories.find((c) => c.id === sourceId)!;
    const target = categories.find((c) => c.id === targetId)!;
    const targetRows = new Map(rows.filter((row) => categoryKey(row.category) === categoryKey(target.name)).map((row) => [row.term.trim().toLowerCase(), row]));
    const pairs = rows.filter((row) => categoryKey(row.category) === categoryKey(source.name)).flatMap((row) => {
      const other = targetRows.get(row.term.trim().toLowerCase());
      return other ? [{ source: row, target: other }] : [];
    });
    setCategoryMerge({ source, target, pairs });
  };

  const applyMerge = (choices: ReadonlyMap<number, number>) => {
    if (!categoryMerge) return;
    const { source, target, pairs } = categoryMerge;
    const removed = new Set<number>();
    const replacement = new Map<number, GlossaryRow>();
    const operations: { from: string; into: string }[] = [];
    for (const pair of pairs) {
      const chosen = choices.get(pair.source.key) === pair.source.key ? pair.source : pair.target;
      const base = pair.target.id ? pair.target : pair.source;
      const other = base.key === pair.target.key ? pair.source : pair.target;
      removed.add(other.key);
      replacement.set(base.key, { ...chosen, key: base.key, ...(base.id ? { id: base.id } : {}), category: target.name });
      if (other.id && base.id) operations.push({ from: other.id, into: base.id });
    }
    setRows((current) => current.filter((row) => !removed.has(row.key)).map((row) => replacement.get(row.key) ?? (categoryKey(row.category) === categoryKey(source.name) ? { ...row, category: target.name } : row)));
    setMerges((current) => [...current, ...operations]);
    setCategories((current) => current.filter((c) => c.id !== source.id).map((c) => c.id === target.id ? { ...c, note: [...new Set([target.note, source.note].filter(Boolean))].join("\n\n") } : c));
    setSheets((current) => current.map((sheet) => categoryKey(sheet.category) === categoryKey(source.name) ? { ...sheet, category: target.name } : sheet));
    setSelection(noSelection);
    setFilter((current) => ({ ...current, category: categoryKey(target.name) }));
    setCategoryMerge(null);
  };

  const openString = (context: string) => {
    void stringBinding(context)
      .then((binding) => { if (binding) onRevealBinding(binding); })
      .catch((reason: unknown) => setError({ title: t("glossary.loadError"), error: normalizeCommandError(reason) }));
  };

  useImperativeHandle(ref, () => ({
    save: saveTerms,
    focusSearch: () => { searchRef.current?.focus(); searchRef.current?.select(); },
  }));

  // A right-click on one of several chosen terms acts on all of them.
  const menuFor = (item: GlossaryItem): MenuEntry[] => {
    const acting = selection.ids.has(item.id) && selection.ids.size > 1 ? chosen : [item];
    if (!selection.ids.has(item.id)) chooseOnly(item.id);
    const keys = acting.flatMap((candidate) => candidate.kind === "entry" ? [candidate.row.key] : []);
    const menu: MenuEntry[] = [];
    if (item.kind === "sheet") {
      menu.push({ id: "open", icon: "externalLink", label: t("glossary.detail.open"), run: () => openString(item.term.context) });
      menu.push({ id: "edit", icon: "pencil", label: t("glossary.detail.edit"), disabled: busy, run: editActive });
    }
    if (keys.length > 0) {
      if (menu.length > 0) menu.push({ id: "separator-category", separator: true });
      menu.push({
        id: "category",
        icon: "listFilter",
        label: t("glossary.setCategory"),
        disabled: busy,
        items: categoryNames.map((category): MenuEntry => ({ id: `category:${category}`, label: category, run: () => setCategory(keys, category) })),
      });
      menu.push({ id: "separator-remove", separator: true });
      menu.push({ id: "remove", icon: "trash", danger: true, disabled: busy || acting.some((candidate) => candidate.kind === "entry" && candidate.replaced.length > 0), label: keys.length > 1 ? t("glossary.removeMany", { count: keys.length }) : t("glossary.remove"), run: () => remove(keys) });
    }
    return menu;
  };

  // A sheet's term of an entry's name and category is the one the entry
  // takes the place of, once saved if not yet.
  const sheetsGive = (item: Extract<GlossaryItem, { kind: "entry" }>) => {
    if (item.replaced.length > 0) return item.replaced;
    const name = itemTerm(item).toLowerCase();
    const category = categoryKey(itemCategory(item));
    return name ? fromSheets.flatMap((other) => other.kind === "sheet" && other.term.term.toLowerCase() === name && categoryKey(other.term.category) === category ? [other.term] : []) : [];
  };
  const sensesOf = (item: GlossaryItem): Sense[] => {
    const name = itemTerm(item).toLowerCase();
    if (!name) return [];
    const replaced = item.kind === "entry" ? new Set(sheetsGive(item)) : new Set();
    return items
      .filter((other) => other !== item && itemTerm(other).toLowerCase() === name && !(other.kind === "sheet" && replaced.has(other.term)))
      .map((other) => ({ id: other.id, category: itemCategory(other), translation: other.kind === "entry" ? other.row.translation.trim() : other.term.translation, source: other.kind === "entry" ? t("glossary.source.project") : other.term.sheet }));
  };

  const editActive = () => {
    const field = detailRef.current?.querySelector<HTMLElement>("[data-glossary-edit]");
    field?.focus();
  };

  if (!saved) {
    return (
      <div className="glossary-view">
        {error ? <><ErrorBanner title={error.title} error={error.error} onDismiss={() => setError(null)} /><button className="button button-secondary" onClick={() => { setError(null); void projectKnowledge(true).then(receive).catch((reason: unknown) => setError({ title: t("glossary.loadError"), error: normalizeCommandError(reason) })); }}>{t("glossary.retry")}</button></> : <p className="muted glossary-loading">{t("common.loading")}</p>}
      </div>
    );
  }

  const filterSheets = filter.sheet === null ? [] : sheets.filter((sheet) => sheet.sheet === filter.sheet);
  const filterChips: { key: string; label: string; clear: () => void }[] = [
    ...(filter.category === null ? [] : [{ key: "category", label: counts.categories.find((category) => category.key === filter.category)?.label ?? filter.category, clear: () => setFilter((current) => ({ ...current, category: null })) }]),
    ...(filter.sheet === null ? [] : [{ key: "sheet", label: filter.sheet, clear: () => setFilter((current) => ({ ...current, sheet: null })) }]),
  ];

  const outdatedFor = (item: GlossaryItem) => {
    const count = outdatedOf(item, outdated);
    const key = { term: itemTerm(item), category: itemCategory(item) };
    return <OutdatedStrings term={key} count={count} revision={stringsRevision} block={block} busy={starting} onFix={(request) => fix([key], request)} onOpen={openString} onOpenSettings={onOpenSettings} />;
  };
  const keysOf = (list: readonly GlossaryItem[]): TermKey[] => list.filter((item) => outdatedOf(item, outdated) > 0).map((item) => ({ term: itemTerm(item), category: itemCategory(item) }));
  const chosenKeys = keysOf(chosen);

  const detail = (() => {
    if (chosen.length > 1) {
      const bulkFix = chosenKeys.length > 0 ? <FixControls label={t("glossary.fix.runMany", { count: chosenKeys.length })} block={block} busy={starting} onFix={(request) => fix(chosenKeys, request)} onOpenSettings={onOpenSettings} /> : null;
      return <BulkDetail count={chosen.length} entries={chosenEntries.filter((item) => item.replaced.length === 0).length} categories={categoryNames} disabled={busy} fix={bulkFix} onCategory={(category) => setCategory(chosenEntries.map((item) => item.row.key), category)} onRemove={() => remove(chosenEntries.map((item) => item.row.key))} />;
    }
    if (activeItem?.kind === "entry") {
      const category = activeItem.row.category;
      return <EntryDetail key={activeItem.id} item={activeItem} sheetsGive={sheetsGive(activeItem)} fix={outdatedFor(activeItem)} categories={categoryNames} senses={sensesOf(activeItem)} disabled={busy} categoryNote={noteFor(category)} onChange={(patch) => change(activeItem.row.key, patch)} onTermCommit={() => openExistingTerm(activeItem.row.key)} onRemove={() => remove([activeItem.row.key])} onOpen={openString} onChoose={chooseOnly} onOpenCategory={() => openCategory(category)} />;
    }
    if (activeItem?.kind === "sheet") {
      const category = activeItem.term.category;
      return <SheetTermDetail key={activeItem.id} term={activeItem.term} fix={outdatedFor(activeItem)} senses={sensesOf(activeItem)} disabled={busy} categoryNote={noteFor(category)} onSettle={() => undefined} onOpen={openString} onChoose={chooseOnly} onOpenCategory={() => openCategory(category)} />;
    }
    if (filterSheets.length > 0) {
      const name = filterSheets[0]!.sheet;
      return <SheetSettings key={name} sheets={filterSheets} count={counts.sheets.get(name) ?? 0} categories={categoryNames} disabled={busy} onChange={(before, after) => saveSheets(sheets.map((sheet) => sheet === before ? after : sheet))} onRemove={() => { setFilter((current) => ({ ...current, sheet: null })); saveSheets(sheets.filter((sheet) => sheet.sheet !== name)); }} />;
    }
    const category = filter.category === null ? undefined : counts.categories.find((candidate) => candidate.key === filter.category);
    if (category) {
      const key = category.key;
      return (
        <CategorySettings
          key={key}
          category={categories.find((c) => categoryKey(c.name) === key)!}
          count={category.count}
          categories={categories}
          usedAsDefault={sheets.some((sheet) => categoryKey(sheet.category) === key)}
          disabled={busy}
          onRename={renameCategory}
          onNote={(note) => setCategories((current) => current.map((c) => categoryKey(c.name) === key ? { ...c, note } : c))}
          onDelete={() => { setCategories((current) => current.filter((c) => categoryKey(c.name) !== key)); setFilter((current) => ({ ...current, category: null })); }}
          onMerge={(target) => beginMerge(categories.find((c) => categoryKey(c.name) === key)!.id, target)}
        />
      );
    }
    return (
      <section className="glossary-detail glossary-detail-empty">
        <UiIcon icon="bookMarked" size="xl" />
        {items.length === 0 ? <span>{t("glossary.empty")}</span> : null}
        <button className="button button-secondary" type="button" disabled={busy} onClick={addTerm}><UiIcon icon="plus" size="sm" />{t("glossary.add")}</button>
      </section>
    );
  })();

  return (
    <div className="glossary-view">
      <NewCategoryDialog open={addingCategory} categories={categories} onClose={() => setAddingCategory(false)} onCreate={(name) => { ensureCategory(name); setAddingCategory(false); setSelection(noSelection); setFilter({ ...emptyGlossaryFilter, category: categoryKey(name) }); }} />
      <MergeCategoriesDialog draft={categoryMerge} onClose={() => setCategoryMerge(null)} onMerge={applyMerge} />
      {error ? <ErrorBanner title={error.title} error={error.error} onDismiss={() => setError(null)} /> : null}
      {saved.termsError ? <p className="glossary-file-problem"><UiIcon icon="circleAlert" size="xs" />{saved.termsError}</p> : null}
      {saved.diagnostics.length > 0 ? (
        <details className="glossary-file-problem">
          <summary>{t("glossary.excluded", { count: saved.diagnostics.length })}</summary>
          <ul>{saved.diagnostics.map((problem) => <li key={problem.line}>{t("glossary.line", { line: String(problem.line) })}: {problem.message}</li>)}</ul>
        </details>
      ) : null}
      <div className="glossary-panes">
        <GlossaryFacets
          filter={filter}
          onFilter={changeFilter}
          views={counts.views}
          pending={outdated === null ? new Set(["outdated"]) : new Set()}
          categories={counts.categories}
          sheets={sheets}
          sheetCounts={counts.sheets}
          sheetNames={sheetNames}
          categoryNames={categoryNames}
          disabled={busy}
          onSheetsChange={saveSheets}
          onAddCategory={() => setAddingCategory(true)}
        />
        <section className="glossary-list" aria-label={t("glossary.tab")}>
          <div className="glossary-toolbar">
            <span className="glossary-search">
              <UiIcon icon="search" size="xs" />
              <input
                ref={searchRef}
                className="input"
                type="search"
                value={filter.query}
                placeholder={t("glossary.search")}
                aria-label={t("glossary.search")}
                spellCheck={false}
                onChange={(event) => setFilter((current) => ({ ...current, query: event.target.value }))}
                onKeyDown={(event) => {
                  if (event.key === "ArrowDown" || event.key === "Enter") { event.preventDefault(); tableRef.current?.focus(); if (shown[0] && selection.active === null) chooseOnly(shown[0].id); }
                  else if (event.key === "Escape" && filter.query) { event.preventDefault(); event.stopPropagation(); setFilter((current) => ({ ...current, query: "" })); }
                }}
              />
            </span>
            {filterChips.map((chip) => (
              <button key={chip.key} type="button" className="glossary-chip" aria-label={t("glossary.clearFilter", { name: chip.label })} onClick={chip.clear}>
                <span>{chip.label}</span><UiIcon icon="x" size="xs" />
              </button>
            ))}
            {filter.view === "outdated" && shown.length > 0 ? (
              // Every term of the view: by meaning when the view is narrowed, else every term at once.
              <FixControls
                label={t("glossary.fix.runAll", { count: shown.length })}
                block={block}
                busy={starting}
                onFix={(request) => fix(filter.category === null && filter.sheet === null && filter.query.trim() === "" ? [] : keysOf(shown), request)}
                onOpenSettings={onOpenSettings}
              />
            ) : null}
            <button className="button button-ghost glossary-add" type="button" disabled={busy} onClick={addTerm}><UiIcon icon="plus" size="sm" />{t("glossary.add")}</button>
          </div>
          <GlossaryTable
            ref={tableRef}
            items={shown}
            selected={selection.ids}
            active={selection.active}
            sort={sort}
            onSort={sortBy}
            usage={usage}
            counting={counting}
            outdated={outdated}
            onChoose={choose}
            onSelectAll={() => setSelection((current) => ({ ids: new Set(shown.map((item) => item.id)), active: current.active ?? shown[0]?.id ?? null }))}
            onRemove={() => remove(chosenEntries.map((item) => item.row.key))}
            onEdit={editActive}
            menuFor={menuFor}
            empty={items.length === 0 ? t("glossary.empty") : t("glossary.noMatches")}
          />
          <footer className="glossary-footer">
            <span className="muted">{shown.length === items.length ? t("glossary.count", { count: items.length }) : t("glossary.shown", { count: shown.length, total: items.length })}</span>
            {problems.size > 0 ? (
              <button className="link-button glossary-problems" type="button" onClick={() => setFilter((current) => ({ ...current, view: "problems", category: null, sheet: null, query: "" }))}>
                <UiIcon icon="circleAlert" size="xs" />{t("glossary.problems", { count: problems.size })}
              </button>
            ) : null}
            <span className="glossary-footer-actions">
              <button className="button button-ghost" type="button" disabled={busy || !dirty} onClick={revert}>{t("glossary.revert")}</button>
              <button className="button button-primary" type="button" disabled={busy || !dirty || problems.size > 0 || runGoing || otherRun} title={runGoing || otherRun ? t("glossary.fix.busy") : "Ctrl+S"} onClick={saveTerms}>{t("glossary.save")}</button>
            </span>
          </footer>
        </section>
        <div ref={detailRef} className="glossary-detail-pane">
          {run?.status ? <RunPanel status={run.status} terms={run.terms} onReveal={onRevealBinding} onClose={() => setRun(null)} onOpenReport={onOpenReport} onContinued={() => setRun((value) => value && { ...value, status: null })} /> : null}
          {detail}
        </div>
      </div>
      <ConfirmDialog open={confirm !== null} message={confirm?.message ?? ""} confirmLabel={confirm?.label ?? t("glossary.save")} cancelLabel={t("glossary.cancel")} onKeepEditing={() => setConfirm(null)} onDiscard={() => { const run = confirm?.run; setConfirm(null); run?.(); }} />
    </div>
  );
});
