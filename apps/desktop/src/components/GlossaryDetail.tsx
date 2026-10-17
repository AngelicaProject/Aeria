import { useEffect, useState, type ReactNode } from "react";
import { normalizeCommandError, termStrings } from "../ipc";
import type { GlossaryItem, GlossaryRow } from "../glossaryModel";
import type { SheetTermDto, TermSheetDto, TermStringsDto } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { CategoryField } from "./CategoryField";
import { FormsInput } from "./FormsInput";
import { problemLabels } from "./GlossaryTable";

/** Another meaning of a term's name, of the project or of a sheet. */
export type Sense = { id: string; category: string; translation: string; source: string };

type EntryDetailProps = {
  item: Extract<GlossaryItem, { kind: "entry" }>;
  /** The terms of the sheets the term takes the place of, or will once saved. */
  sheetsGive: readonly SheetTermDto[];
  /** The translations that do not use the term, and their correction. */
  fix: ReactNode;
  categories: readonly string[];
  senses: readonly Sense[];
  disabled: boolean;
  /** The note of the term's category, said before its own; empty when none. */
  categoryNote: string;
  onChange: (patch: Partial<GlossaryRow>) => void;
  onRemove: () => void;
  onOpen: (context: string) => void;
  onChoose: (id: string) => void;
  onOpenCategory: () => void;
};

/** The fields of a term of the project, with what the sheets give for its name. */
export function EntryDetail({ item, sheetsGive, fix, categories, senses, disabled, categoryNote, onChange, onRemove, onOpen, onChoose, onOpenCategory }: EntryDetailProps) {
  const { t } = useI18n();
  const row = item.row;
  const id = `glossary-entry-${row.key}`;
  const translation = row.translation.trim();
  return (
    <section className="glossary-detail" aria-label={row.term || t("glossary.untitled")}>
      <div className="glossary-field">
        <label htmlFor={`${id}-term`}>{t("glossary.field.term")}</label>
        <input id={`${id}-term`} data-glossary-headword className="input glossary-detail-headword" value={row.term} readOnly={disabled} autoFocus={!row.term} onChange={(event) => onChange({ term: event.target.value })} />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-translation`}>{t("glossary.field.translation")}</label>
        <input id={`${id}-translation`} data-glossary-edit className="input" value={row.translation} readOnly={disabled} onChange={(event) => onChange({ translation: event.target.value })} />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-category`}>{t("glossary.field.category")}</label>
        <CategoryField id={`${id}-category`} value={row.category} categories={categories} disabled={disabled} onCommit={(category) => onChange({ category })} />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-forms`}>{t("glossary.field.forms")}</label>
        <FormsInput id={`${id}-forms`} forms={row.forms} onChange={(forms) => onChange({ forms })} />
      </div>
      <div className="glossary-field glossary-field-grow">
        <label htmlFor={`${id}-note`}>{t("glossary.field.note")}</label>
        <textarea id={`${id}-note`} className="input glossary-note" value={row.note} spellCheck readOnly={disabled} onChange={(event) => onChange({ note: event.target.value })} />
      </div>
      <CategoryNote category={row.category.trim()} note={categoryNote} onOpen={onOpenCategory} />
      <label className="glossary-check" title={t("glossary.field.matchCaseTitle")}>
        <input type="checkbox" checked={row.matchCase} disabled={disabled} onChange={(event) => onChange({ matchCase: event.target.checked })} />
        {t("glossary.field.matchCase")}
      </label>
      <label className="glossary-check" title={t("glossary.field.indeclinableTitle")}>
        <input type="checkbox" checked={row.indeclinable} disabled={disabled} onChange={(event) => onChange({ indeclinable: event.target.checked })} />
        {t("glossary.field.indeclinable")}
      </label>
      {item.problem ? <p className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{t(problemLabels[item.problem])}</p> : null}
      {fix}
      {sheetsGive.length > 0 ? (
        <div className="glossary-field">
          <span className="glossary-field-label">{t("glossary.detail.sheets")}</span>
          <ul className="glossary-sheet-gives">
            {sheetsGive.map((term) => (
              <li key={term.context} className={term.translation !== translation ? "differs" : undefined}>
                <span className="glossary-sheet-gives-translation">{term.translation}</span>
                <span className="muted">{term.sheet} · {t("glossary.detail.strings", { count: term.strings })}</span>
                <button type="button" className="link-button" onClick={() => onOpen(term.context)}>{t("glossary.detail.open")}</button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <SenseList senses={senses} onChoose={onChoose} />
      <TermStrings term={row.term.trim()} category={row.category.trim()} onOpen={onOpen} />
      <div className="glossary-detail-actions">
        {item.redundant ? <span className="glossary-badge">{t("glossary.badge.same")}</span> : null}
        <button className="button button-ghost glossary-remove" type="button" disabled={disabled} onClick={onRemove}>
          <UiIcon icon="trash" size="sm" />{item.redundant ? t("glossary.detail.takeOut") : t("glossary.remove")}
        </button>
      </div>
    </section>
  );
}

type SheetTermDetailProps = {
  term: SheetTermDto;
  /** The translations that do not use the term, and their correction. */
  fix: ReactNode;
  senses: readonly Sense[];
  disabled: boolean;
  /** The note of the term's category; empty when none. */
  categoryNote: string;
  /** Makes the term a term of the project with this translation. */
  onSettle: (translation: string) => void;
  onOpen: (context: string) => void;
  onChoose: (id: string) => void;
  onOpenCategory: () => void;
};

/**
 * A term a sheet gives: its translation is the one most of its strings
 * have. Another translation, typed or taken from the strings, makes it a
 * term of the project.
 */
export function SheetTermDetail({ term, fix, senses, disabled, categoryNote, onSettle, onOpen, onChoose, onOpenCategory }: SheetTermDetailProps) {
  const { t } = useI18n();
  const id = `glossary-sheet-${term.context}`;
  const [translation, setTranslation] = useState(term.translation);
  useEffect(() => { setTranslation(term.translation); }, [term.translation, term.context]);
  const settle = () => {
    const next = translation.trim();
    if (next !== "" && next !== term.translation) onSettle(next);
    else setTranslation(term.translation);
  };
  return (
    <section className="glossary-detail" aria-label={term.term}>
      <div className="glossary-field">
        <label htmlFor={`${id}-term`}>{t("glossary.field.term")}</label>
        <input id={`${id}-term`} className="input glossary-detail-headword" value={term.term} readOnly />
      </div>
      <div className="glossary-field">
        <label htmlFor={`${id}-translation`}>{t("glossary.field.translation")}</label>
        <input
          id={`${id}-translation`}
          data-glossary-edit
          className="input"
          value={translation}
          readOnly={disabled}
          onChange={(event) => setTranslation(event.target.value)}
          onBlur={settle}
          onKeyDown={(event) => {
            if (event.key === "Enter") { event.preventDefault(); event.currentTarget.blur(); }
            else if (event.key === "Escape") { event.preventDefault(); setTranslation(term.translation); }
          }}
        />
      </div>
      <div className="glossary-field">
        <span className="glossary-field-label">{t("glossary.field.category")}</span>
        <span className="glossary-detail-value">{term.category}</span>
      </div>
      {term.forms.length > 0 ? (
        <div className="glossary-field">
          <span className="glossary-field-label">{t("glossary.field.forms")}</span>
          <span className="glossary-detail-value">{term.forms.join("; ")}</span>
        </div>
      ) : null}
      <CategoryNote category={term.category} note={categoryNote} onOpen={onOpenCategory} />
      {fix}
      <div className="glossary-field">
        <span className="glossary-field-label">{t("glossary.detail.variants")}</span>
        <ul className="glossary-variants">
          {term.variants.map((variant, index) => (
            <li key={variant.translation} className={index === 0 ? "chosen" : undefined}>
              <span className="glossary-variant-translation">{variant.translation}</span>
              <span className="muted">{t("glossary.detail.strings", { count: variant.strings })}</span>
              {index > 0 ? <button type="button" className="link-button" disabled={disabled} onClick={() => onSettle(variant.translation)}>{t("glossary.detail.use")}</button> : null}
            </li>
          ))}
        </ul>
      </div>
      <div className="glossary-field">
        <span className="glossary-field-label">{t("glossary.column.source")}</span>
        <span className="glossary-detail-value"><UiIcon icon="table2" size="xs" />{term.sheet}</span>
      </div>
      <SenseList senses={senses} onChoose={onChoose} />
      <TermStrings term={term.term} category={term.category} onOpen={onOpen} />
      <div className="glossary-detail-actions">
        <button className="button button-secondary" type="button" onClick={() => onOpen(term.context)}><UiIcon icon="externalLink" size="sm" />{t("glossary.detail.open")}</button>
        <button className="button button-ghost" type="button" disabled={disabled} onClick={() => onSettle(term.translation)}><UiIcon icon="pencil" size="sm" />{t("glossary.detail.edit")}</button>
      </div>
    </section>
  );
}

/** The note of a term's category, shown as it is said before the term's own, with a way to its settings. */
function CategoryNote({ category, note, onOpen }: { category: string; note: string; onOpen: () => void }) {
  const { t } = useI18n();
  if (!note || !category) return null;
  return (
    <div className="glossary-field">
      <span className="glossary-field-label">
        {t("glossary.category.from", { category })}
        <button type="button" className="link-button" onClick={onOpen}>{t("glossary.category.edit")}</button>
      </span>
      <p className="glossary-category-note">{note}</p>
    </div>
  );
}

/** How long the choice rests on a term before its strings are looked for, in milliseconds. */
const STRINGS_DELAY_MS = 150;

/**
 * The strings whose source uses a term of the saved glossary, each opened
 * in the editor by a click. A term not saved yet has none to show.
 */
function TermStrings({ term, category, onOpen }: { term: string; category: string; onOpen: (context: string) => void }) {
  const { t } = useI18n();
  const [found, setFound] = useState<TermStringsDto | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setFound(undefined);
    setError(null);
    if (!term || !category) { setFound(null); return; }
    let current = true;
    const timer = window.setTimeout(() => {
      void termStrings(term, category)
        .then((result) => { if (current) setFound(result); })
        .catch((reason: unknown) => { if (current) { setFound(null); setError(normalizeCommandError(reason).message); } });
    }, STRINGS_DELAY_MS);
    return () => { current = false; window.clearTimeout(timer); };
  }, [category, term]);
  if (found === null && !error) return null;
  return (
    <div className="glossary-field">
      <span className="glossary-field-label">
        {t("glossary.strings.title")}
        {found ? <span className="glossary-strings-count">{found.strings.length < found.total ? t("glossary.strings.shown", { count: found.strings.length, total: found.total }) : found.total.toLocaleString()}</span> : null}
      </span>
      {error ? <p className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{error}</p> : null}
      {found === undefined ? <span className="muted glossary-strings-status">{t("common.loading")}</span> : null}
      {found && found.total === 0 ? <span className="muted glossary-strings-status">{t("glossary.strings.none")}</span> : null}
      {found && found.strings.length > 0 ? (
        <ul className="glossary-strings">
          {found.strings.map((string) => (
            <li key={string.context}>
              <button type="button" className="glossary-string" title={string.context} onClick={() => onOpen(string.context)}>
                <span className="glossary-string-sheet">{string.context.split(":")[0]}</span>
                <span className="glossary-string-source">{string.source}</span>
                <span className={string.translation ? "glossary-string-translation" : "glossary-string-translation empty"}>
                  {string.translation || t("glossary.strings.untranslated")}
                  {string.fuzzy ? <span className="glossary-badge">{t("glossary.strings.fuzzy")}</span> : null}
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function SenseList({ senses, onChoose }: { senses: readonly Sense[]; onChoose: (id: string) => void }) {
  const { t } = useI18n();
  if (senses.length === 0) return null;
  return (
    <div className="glossary-field">
      <span className="glossary-field-label">{t("glossary.detail.senses")}</span>
      <ul className="glossary-senses">
        {senses.map((sense) => (
          <li key={sense.id}>
            <button type="button" className="link-button" onClick={() => onChoose(sense.id)}>{sense.translation}</button>
            <span className="glossary-badge">{sense.category || "—"}</span>
            <span className="muted">{sense.source}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

type SheetSettingsProps = {
  /** The declared columns of one sheet. */
  sheets: readonly TermSheetDto[];
  count: number;
  categories: readonly string[];
  disabled: boolean;
  onChange: (before: TermSheetDto, after: TermSheetDto) => void;
  onRemove: () => void;
};

/** A declared sheet: the category and note of the terms of each of its columns, saved at once. */
export function SheetSettings({ sheets, count, categories, disabled, onChange, onRemove }: SheetSettingsProps) {
  const { t } = useI18n();
  const name = sheets[0]?.sheet ?? "";
  return (
    <section className="glossary-detail" aria-label={name}>
      <div className="glossary-detail-title">
        <UiIcon icon="table2" size="sm" />
        <span>{name}</span>
        <span className="muted">{t("glossary.count", { count })}</span>
      </div>
      {sheets.map((sheet) => <SheetColumn key={sheet.column} sheet={sheet} titled={sheets.length > 1} categories={categories} disabled={disabled} onChange={(next) => onChange(sheet, next)} />)}
      <div className="glossary-detail-actions">
        <button className="button button-ghost glossary-remove" type="button" disabled={disabled} onClick={onRemove}><UiIcon icon="trash" size="sm" />{t("glossary.sheets.remove", { sheet: name })}</button>
      </div>
    </section>
  );
}

function SheetColumn({ sheet, titled, categories, disabled, onChange }: { sheet: TermSheetDto; titled: boolean; categories: readonly string[]; disabled: boolean; onChange: (sheet: TermSheetDto) => void }) {
  const { t } = useI18n();
  const id = `glossary-sheet-settings-${sheet.sheet}-${sheet.column}`;
  const savedNote = sheet.note ?? "";
  const [note, setNote] = useState(savedNote);
  useEffect(() => { setNote(savedNote); }, [savedNote, sheet.sheet]);
  return (
    <>
      {titled ? <span className="glossary-field-label">{t("glossary.sheets.column")} {String(sheet.column)}</span> : null}
      <div className="glossary-field">
        <label htmlFor={`${id}-category`}>{t("glossary.sheets.category")}</label>
        <CategoryField id={`${id}-category`} value={sheet.category} categories={categories} disabled={disabled} onCommit={(category) => onChange({ ...sheet, category })} />
      </div>
      <div className="glossary-field glossary-field-grow">
        <label htmlFor={`${id}-note`}>{t("glossary.sheets.note")}</label>
        <textarea id={`${id}-note`} className="input glossary-note" value={note} spellCheck title={t("glossary.sheets.noteTitle")} readOnly={disabled} onChange={(event) => setNote(event.target.value)} onBlur={() => { if (note.trim() !== savedNote) onChange({ ...sheet, note: note.trim() || null }); }} />
      </div>
    </>
  );
}

type BulkDetailProps = {
  /** How many terms are chosen, and how many of them are the project's. */
  count: number;
  entries: number;
  categories: readonly string[];
  disabled: boolean;
  /** The correction of the chosen terms' translations, when some do not use them. */
  fix: ReactNode;
  onCategory: (category: string) => void;
  onRemove: () => void;
};

/** Several chosen terms: what can be done to them at once. */
export function BulkDetail({ count, entries, categories, disabled, fix, onCategory, onRemove }: BulkDetailProps) {
  const { t } = useI18n();
  return (
    <section className="glossary-detail" aria-label={t("glossary.selected", { count })}>
      <div className="glossary-detail-title"><span>{t("glossary.selected", { count })}</span></div>
      {fix}
      {entries > 0 ? (
        <>
          <div className="glossary-field">
            <label htmlFor="glossary-bulk-category">{t("glossary.setCategory")}</label>
            <CategoryField id="glossary-bulk-category" value="" categories={categories} disabled={disabled} onCommit={onCategory} />
          </div>
          <div className="glossary-detail-actions">
            <button className="button button-ghost glossary-remove" type="button" disabled={disabled} onClick={onRemove}><UiIcon icon="trash" size="sm" />{t("glossary.removeMany", { count: entries })}</button>
          </div>
        </>
      ) : null}
    </section>
  );
}

type CategorySettingsProps = {
  /** The category as its terms write it. */
  category: string;
  count: number;
  /** The declared sheets of the category, which keep its note. */
  sheets: readonly TermSheetDto[];
  disabled: boolean;
  /** Why the model's decisions cannot be dropped now, or null. */
  forgetBlock: string | null;
  /** What the last drop of the model's decisions did, or null. */
  forgotten: string | null;
  onNote: (note: string | null) => void;
  onForget: () => void;
  onOpenSheet: (sheet: string) => void;
};

/**
 * A category: the note its sheets keep, saved to all of them at once, the
 * sheets themselves, and a way to have the model decide its terms again.
 */
export function CategorySettings({ category, count, sheets, disabled, forgetBlock, forgotten, onNote, onForget, onOpenSheet }: CategorySettingsProps) {
  const { t } = useI18n();
  const notes = [...new Set(sheets.map((sheet) => (sheet.note ?? "").trim()))];
  const differ = notes.length > 1;
  const savedNote = notes.filter((note) => note !== "").join(" ");
  const [note, setNote] = useState(savedNote);
  useEffect(() => { setNote(savedNote); }, [savedNote, category]);
  const names = [...new Set(sheets.map((sheet) => sheet.sheet))];
  return (
    <section className="glossary-detail" aria-label={category}>
      <div className="glossary-detail-title">
        <UiIcon icon="listFilter" size="sm" />
        <span>{category}</span>
        <span className="muted">{t("glossary.count", { count })}</span>
      </div>
      {sheets.length > 0 ? (
        <div className="glossary-field glossary-field-grow">
          <label htmlFor="glossary-category-note">{t("glossary.category.note")}</label>
          <textarea
            id="glossary-category-note"
            className="input glossary-note"
            value={note}
            spellCheck
            title={t("glossary.category.noteTitle")}
            readOnly={disabled}
            onChange={(event) => setNote(event.target.value)}
            onBlur={() => { if (note.trim() !== savedNote) onNote(note.trim() || null); }}
          />
          {differ ? <p className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{t("glossary.category.notesDiffer")}</p> : null}
        </div>
      ) : <p className="muted">{t("glossary.category.noSheets")}</p>}
      {names.length > 0 ? (
        <div className="glossary-field">
          <span className="glossary-field-label">{t("glossary.category.sheets")}</span>
          <ul className="glossary-senses">
            {names.map((name) => (
              <li key={name}>
                <UiIcon icon="table2" size="xs" />
                <button type="button" className="link-button" onClick={() => onOpenSheet(name)}>{name}</button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <div className="glossary-detail-actions">
        <button className="button button-secondary" type="button" disabled={disabled || forgetBlock !== null} title={forgetBlock ?? t("glossary.category.forgetTitle")} onClick={onForget}>
          <UiIcon icon="sparkles" size="sm" />{t("glossary.category.forget")}
        </button>
      </div>
      {forgotten ? <p className="muted">{forgotten}</p> : null}
    </section>
  );
}
