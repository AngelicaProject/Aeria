import { memo, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { bindingKey } from "../binding";
import { speakerName } from "../dialogueScene";
import { checkDraft, macroInsertions, normalizeCommandError, setTranslationTermException, stringHints } from "../ipc";
import { describeIssue, errorText } from "../issueText";
import type { MessageKey } from "../i18n/translate";
import { useMacroIdioms } from "../macroIdioms";
import { guideMacros, macroState, ruleOrder, type GuideMacro } from "../stringGuide";
import type { CommandError, DraftCheckDto, HintGameNameDto, HintTermDto, MacroInsertionDto, MacroRule, SourceBinding, StringHintsDto } from "../types";
import { useEditorFocus } from "../ui/editorFocus";
import { loadIcon } from "../ui/gameGlyphs";
import { useI18n } from "../ui/i18n";
import { useMacroView } from "../ui/useMacroView";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { chipLookup, learnTags, type ChipPick } from "./macroChipsExtension";

/** Typing pauses this long before the draft is checked. */
const CHECK_DELAY_MS = 250;

type StringGuideProps = {
  /** Bumps when the project's files may have changed (save, knowledge, sync). */
  revision: number;
  /** Opens the project's terms. */
  onOpenTerms: () => void;
  /** Opens a string in the editor, such as the one a name's translation comes from. */
  onReveal: (binding: SourceBinding) => void;
};

type Loaded<T> = { key: string; value: T };

/**
 * The string guide: what helps translate the string in the editor. It shows
 * what a machine translation request tells the model about the string (the
 * game's names and the project's terms in its source, who says it, the
 * length of an interface label, a line that varies with the player
 * character's gender), the parts of its macros with what a translation may
 * do with each, and what the checks find in the translation as it is typed.
 * Picking a name, a term, or a macro adds it to the translation.
 */
export const StringGuide = memo(function StringGuide({ revision, onOpenTerms, onReveal }: StringGuideProps) {
  const { t } = useI18n();
  const focus = useEditorFocus();
  const binding = focus?.binding ?? null;
  const key = binding ? bindingKey(binding) : null;
  const bindingRef = useRef(binding);
  bindingRef.current = binding;
  const draft = focus?.draft ?? "";

  const [hints, setHints] = useState<Loaded<StringHintsDto> | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  /** Bumps when the guide itself changed the string, such as a term exception. */
  const [own, setOwn] = useState(0);
  const [excepting, setExcepting] = useState(false);
  const setException = useCallback(async (term: string, add: boolean) => {
    const current = bindingRef.current;
    if (!current) return;
    setExcepting(true);
    try {
      await setTranslationTermException(current, term, add);
      setOwn((value) => value + 1);
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setExcepting(false);
    }
  }, []);
  const onException = excepting ? undefined : (term: string, add: boolean) => void setException(term, add);
  useEffect(() => {
    const current = bindingRef.current;
    if (key === null || current === null) return;
    let cancelled = false;
    stringHints(current)
      .then((value) => { if (!cancelled) { setHints({ key, value }); setError(null); } })
      .catch((caught: unknown) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [key, revision, own]);

  const [check, setCheck] = useState<Loaded<DraftCheckDto> | null>(null);
  const checkedKey = useRef<string | null>(null);
  useEffect(() => {
    const current = bindingRef.current;
    if (key === null || current === null) return;
    let cancelled = false;
    // A new string is checked at once; typing waits for a pause.
    const timer = window.setTimeout(() => {
      checkDraft(current, draft)
        .then((value) => {
          if (cancelled) return;
          checkedKey.current = key;
          setCheck({ key, value });
        })
        .catch(() => undefined);
    }, checkedKey.current === key ? CHECK_DELAY_MS : 0);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [key, draft, revision, own]);

  const sourceView = useMacroView(focus?.source ?? null);
  const idioms = useMacroIdioms();
  const macros = useMemo(() => {
    if (!focus || !sourceView || sourceView.text !== focus.source) return null;
    learnTags(sourceView.text, sourceView.view.tags);
    return guideMacros(focus.source, sourceView.view.tags, { ...chipLookup, idioms }, t);
  }, [focus?.source, idioms, sourceView, t]);

  if (!focus) {
    return <div className="string-guide-empty"><UiIcon icon="info" size="md" />{t("hints.noString")}</div>;
  }
  const current = hints?.key === key ? hints.value : null;
  const stale = hints !== null && hints.key !== key;
  const shown = current ?? hints?.value ?? null;
  const drafted = draft.trim().length > 0;
  const verdict = check?.key === key && drafted ? check.value : null;
  const pick = (value: ChipPick) => focus.apply(value);

  if (error && !shown) return <div className="string-guide-empty error">{errorText(error, t)}</div>;
  if (!shown) return <div className="string-guide-empty"><span className="spinner spinner-xs" />{t("common.loading")}</div>;

  const hasFacts = shown.speaker !== null || shown.maxLength !== null || shown.gendered.length > 0 || (shown.kind !== null && shown.kind !== "other");
  const hasWords = shown.names.length > 0 || shown.terms.length > 0;
  const hasMacros = (macros?.length ?? 0) > 0;
  const findings = verdict ? verdict.issues.length : 0;

  return (
    <div className={stale ? "string-guide is-stale" : "string-guide"} aria-busy={stale}>
      {hasFacts ? <StringFacts hints={shown} draft={draft} verdict={verdict} onPick={focus.busy ? undefined : pick} /> : null}
      <div className="string-guide-columns">
      {hasWords ? (
        <GuideColumn
          className="is-words"
          icon="bookMarked"
          title={t("hints.words")}
          count={shown.names.length + shown.terms.length}
          action={<button className="string-guide-link" type="button" onClick={onOpenTerms}>{t("hints.openTerms")}</button>}
        >
          <Words hints={shown} draft={draft} verdict={verdict} onPick={focus.busy ? undefined : pick} onReveal={onReveal} onException={onException} />
        </GuideColumn>
      ) : null}
      {hasMacros ? (
        <GuideColumn className="is-macros" icon="caseSensitive" title={t("hints.macros")} count={macros!.reduce((sum, macro) => sum + macro.count, 0)}>
          <Macros macros={macros!} missing={verdict?.missing ?? null} onPick={focus.busy ? undefined : pick} />
        </GuideColumn>
      ) : null}
      {!hasWords && !hasMacros ? (
        <GuideColumn className="is-plain" icon="circleCheck" title={t("hints.plain")}>
          <p className="string-guide-note">{t("hints.plainHint")}</p>
        </GuideColumn>
      ) : null}
      <GuideColumn className="is-check" icon="check" title={t("hints.check")} count={findings > 0 ? findings : undefined}>
        <DraftIssues drafted={drafted} verdict={verdict} onException={onException} />
      </GuideColumn>
      </div>
    </div>
  );
});

function GuideColumn({ className, icon, title, count, action, children }: { className: string; icon: UiIconName; title: string; count?: number | undefined; action?: ReactNode; children: ReactNode }) {
  return (
    <section className={`string-guide-column ${className}`} aria-label={title}>
      <header className="string-guide-head">
        <UiIcon icon={icon} size="xs" />
        <span className="string-guide-title">{title}</span>
        {count !== undefined ? <span className="count">{count}</span> : null}
        <span className="spacer" />
        {action}
      </header>
      <div className="string-guide-body">{children}</div>
    </section>
  );
}

/** A mark of how the translation stands with a hint: done, wrong, or still to do. */
function StateMark({ state, title }: { state: "ok" | "bad" | "todo" | null; title?: string }) {
  if (state === null) return <span className="string-guide-mark" aria-hidden />;
  const icon: UiIconName = state === "ok" ? "check" : state === "bad" ? "circleX" : "circle";
  return <span className={`string-guide-mark is-${state}`} title={title}><UiIcon icon={icon} size="xs" /></span>;
}

const KINDS: Readonly<Record<string, MessageKey>> = {
  journal: "hints.kind.journal",
  objective: "hints.kind.objective",
};

function StringFacts({ hints, draft, verdict, onPick }: { hints: StringHintsDto; draft: string; verdict: DraftCheckDto | null; onPick: ((pick: ChipPick) => void) | undefined }) {
  const { t } = useI18n();
  const [genderPick, setGenderPick] = useState<ChipPick | null>(null);
  const gendered = hints.gendered.length > 0;
  useEffect(() => {
    if (!gendered) return;
    let cancelled = false;
    macroInsertions()
      .then((insertions: MacroInsertionDto[]) => {
        const gender = insertions.find((insertion) => insertion.form === "branches" && insertion.parts[0]?.includes("$gn4"));
        if (!cancelled && gender) setGenderPick({ branches: [gender.parts[0] ?? "", gender.parts[1] ?? "", gender.parts[2] ?? ""] });
      })
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [gendered]);
  const drafted = draft.trim().length > 0;
  const kind = hints.kind ? KINDS[hints.kind] : undefined;
  const length = verdict?.length ?? null;
  const budget = hints.maxLength;
  const over = budget !== null && length !== null && length > budget.max;
  const bytes = budget?.unit === "bytes";
  const varies = draft.includes("$gn4");
  return (
    <div className="string-guide-facts" role="group" aria-label={t("hints.string")}>
      {hints.speaker ? (
        <span className="string-guide-fact" title={t("hints.speakerTitle", { label: hints.speaker.label })}>
          <UiIcon icon="user" size="xs" className="string-guide-icon" />
          <span className="string-guide-label">{t("hints.speaker")}</span>
          {hints.speaker.name ? (
            <span className="string-guide-value"><strong>{hints.speaker.name.translation}</strong><span className="muted"> · {hints.speaker.name.name}</span></span>
          ) : <span className="string-guide-value">{speakerName(hints.speaker.label)}</span>}
        </span>
      ) : null}
      {kind ? (
        <span className="string-guide-fact">
          <UiIcon icon="messageSquare" size="xs" className="string-guide-icon" />
          <span className="string-guide-value">{t(kind)}</span>
        </span>
      ) : null}
      {budget !== null ? (
        <span className={over ? "string-guide-fact is-todo" : "string-guide-fact"} title={bytes ? t("hints.nameLengthTitle", { max: String(budget.max) }) : t("hints.lengthTitle")}>
          <StateMark state={!drafted || length === null ? null : over ? "todo" : "ok"} />
          <span className="string-guide-label">{t(bytes ? "hints.nameLength" : "hints.length")}</span>
          <span className="string-guide-value mono">{drafted && length !== null
            ? t(bytes ? "hints.bytesOf" : "hints.lengthOf", { length: String(length), max: String(budget.max) })
            : t(bytes ? "hints.bytesMax" : "hints.lengthMax", { max: String(budget.max) })}</span>
        </span>
      ) : null}
      {gendered ? (
        <span className={drafted && !varies ? "string-guide-fact is-todo" : "string-guide-fact"} title={t("hints.genderTitle")}>
          <StateMark state={!drafted ? null : varies ? "ok" : "todo"} />
          <span className="string-guide-value">
            {t("hints.gender")}
            <span className="muted"> · {hints.gendered.map((text) => text === "source" ? t("hints.gender.source") : text.toUpperCase()).join(", ")}</span>
          </span>
          {genderPick && onPick ? <button className="string-guide-link" type="button" title={t("hints.genderInsertTitle")} onClick={() => onPick(genderPick)}>{t("hints.genderInsert")}</button> : null}
        </span>
      ) : null}
    </div>
  );
}

function termState(term: HintTermDto, verdict: DraftCheckDto | null): { state: "ok" | "bad" | "todo" | null; variant?: string } {
  if (term.excepted || verdict === null) return { state: null };
  const forbidden = verdict.issues.find((issue) => issue.kind === "forbiddenTerm" && issue.term === term.term);
  if (forbidden) return { state: "bad", variant: forbidden.variant ?? "" };
  if (verdict.issues.some((issue) => issue.kind === "termNotUsed" && issue.term === term.term)) return { state: "todo" };
  return { state: "ok" };
}

/** What the strings of a name sheet name, by sheet. */
const NAME_KINDS: Readonly<Record<string, MessageKey>> = {
  PlaceName: "hints.from.place",
  Town: "hints.from.town",
  Race: "hints.from.race",
  Tribe: "hints.from.clan",
  GuardianDeity: "hints.from.deity",
  ClassJob: "hints.from.classJob",
  Status: "hints.from.status",
  Action: "hints.from.action",
  Trait: "hints.from.trait",
  Item: "hints.from.item",
  EventItem: "hints.from.keyItem",
  Mount: "hints.from.mount",
  Companion: "hints.from.minion",
  Ornament: "hints.from.accessory",
  Title: "hints.from.title",
  ENpcResident: "hints.from.character",
  BNpcName: "hints.from.enemy",
  EObjName: "hints.from.object",
  Fate: "hints.from.fate",
  InstanceContent: "hints.from.duty",
  ContentFinderCondition: "hints.from.duty",
  Quest: "hints.from.quest",
};

function NameOrigin({ name, onReveal }: { name: HintGameNameDto; onReveal: (binding: SourceBinding) => void }) {
  const { t } = useI18n();
  const kinds: string[] = [];
  for (const sheet of name.sheets) {
    const key = NAME_KINDS[sheet];
    const kind = key ? t(key) : sheet;
    if (!kinds.includes(kind)) kinds.push(kind);
  }
  const kind = kinds.join(", ");
  const text = name.full ? t("hints.from.form", { kind, full: name.full }) : kind;
  const binding = name.binding;
  if (!binding) return <div className="string-guide-origin">{text}</div>;
  const where = `${binding.sheetName} ${binding.rowId}:${binding.subrowId}`;
  return (
    <button className="string-guide-origin is-link" type="button" title={t("hints.from.open", { where })} onClick={() => onReveal(binding)}>
      {text}<span className="mono"> · {where}</span>
    </button>
  );
}

/** Adds or removes a term exception of the string; absent while one is being written. */
type ExceptionHandler = ((term: string, add: boolean) => void) | undefined;

function Words({ hints, draft, verdict, onPick, onReveal, onException }: { hints: StringHintsDto; draft: string; verdict: DraftCheckDto | null; onPick: ((pick: ChipPick) => void) | undefined; onReveal: (binding: SourceBinding) => void; onException: ExceptionHandler }) {
  const { t } = useI18n();
  const drafted = draft.trim().length > 0;
  const insert = (text: string) => onPick?.({ insert: text });
  return (
    <ul className="string-guide-list">
      {hints.terms.map((term) => {
        const { state, variant } = termState(term, verdict);
        const stateTitle = state === "bad" ? t("hints.term.forbidden", { variant: variant ?? "" }) : state === "todo" ? t("hints.term.unused") : state === "ok" ? t("hints.term.used") : undefined;
        return (
          <li className={`string-guide-row is-term${term.excepted ? " is-excepted" : ""}${state === "bad" ? " is-bad" : ""}`} key={`term:${term.term}`}>
            <StateMark state={state} {...(stateTitle ? { title: stateTitle } : {})} />
            <div className="string-guide-entry">
              <div className="string-guide-pair">
                <span className="string-guide-source">{term.term}</span>
                <UiIcon icon="arrowRight" size="xs" className="string-guide-arrow" />
                <button className="string-guide-insert" type="button" disabled={!onPick} title={t("hints.insertTitle", { text: term.translation })} onClick={() => insert(term.translation)}>{term.translation}</button>
                <span className="string-guide-tag">{term.excepted ? t("hints.term.excepted") : t("hints.term")}</span>
                <button
                  className="string-guide-link string-guide-except"
                  type="button"
                  disabled={!onException}
                  title={term.excepted ? t("findings.removeException", { term: term.term }) : t("findings.exceptTitle", { term: term.term })}
                  onClick={() => onException?.(term.term, !term.excepted)}
                >
                  {t(term.excepted ? "hints.term.restore" : "hints.term.except")}
                </button>
              </div>
              {term.never.length > 0 ? <div className="string-guide-never">{t("hints.term.never", { variants: term.never.join(", ") })}</div> : null}
              {term.note ? <div className="string-guide-note">{term.note}</div> : null}
            </div>
          </li>
        );
      })}
      {hints.names.map((name) => {
        const used = verdict?.namesUsed.includes(name.name) ?? false;
        const state = !drafted || verdict === null ? null : used ? "ok" : "todo";
        return (
          <li className="string-guide-row" key={`name:${name.name}`}>
            <StateMark state={state} {...(state ? { title: t(used ? "hints.name.used" : "hints.name.unused") } : {})} />
            <div className="string-guide-entry">
              <div className="string-guide-pair">
                <span className="string-guide-source">{name.name}</span>
                <UiIcon icon="arrowRight" size="xs" className="string-guide-arrow" />
                <button className="string-guide-insert" type="button" disabled={!onPick} title={t("hints.insertTitle", { text: name.translation })} onClick={() => insert(name.translation)}>{name.translation}</button>
                <span className="string-guide-tag">{t("hints.name")}</span>
              </div>
              <NameOrigin name={name} onReveal={onReveal} />
            </div>
          </li>
        );
      })}
    </ul>
  );
}

const RULE_LABELS: Readonly<Record<MacroRule, MessageKey>> = {
  keep: "hints.rule.keep",
  condition: "hints.rule.condition",
  letterCase: "hints.rule.letterCase",
  formatting: "hints.rule.formatting",
  free: "hints.rule.free",
};

const RULE_HINTS: Readonly<Record<MacroRule, MessageKey>> = {
  keep: "hints.rule.keepHint",
  condition: "hints.rule.conditionHint",
  letterCase: "hints.rule.letterCaseHint",
  formatting: "hints.rule.formattingHint",
  free: "hints.rule.freeHint",
};

function GuideChip({ macro, onPick }: { macro: GuideMacro; onPick: ((pick: ChipPick) => void) | undefined }) {
  const { t } = useI18n();
  const [image, setImage] = useState<string | null>(null);
  useEffect(() => {
    if (macro.icon === undefined) return;
    let cancelled = false;
    void loadIcon(macro.icon).then((url) => { if (!cancelled) setImage(url); });
    return () => { cancelled = true; };
  }, [macro.icon]);
  const tone = macro.tone === "format" ? "value" : macro.tone;
  const spelling = "insert" in macro.pick ? macro.pick.insert : "wrap" in macro.pick ? macro.pick.wrap.join("…") : "";
  const title = [macro.title ?? spelling, t("hints.pickTitle")].filter(Boolean).join("\n");
  return (
    <button className={`string-guide-chip cm-chip cm-chip-${tone}${image ? " has-image" : ""}`} type="button" disabled={!onPick} aria-label={macro.label} title={title} onClick={() => onPick?.(macro.pick)}>
      {macro.tone === "format" ? <span className="string-guide-swatch" style={macro.color ? { background: macro.color.length === 9 && macro.color.endsWith("ff") ? macro.color.slice(0, 7) : macro.color } : undefined} /> : null}
      {image ? <img src={image} alt={macro.label} draggable={false} /> : macro.label}
    </button>
  );
}

function Macros({ macros, missing, onPick }: { macros: readonly GuideMacro[]; missing: readonly string[] | null; onPick: ((pick: ChipPick) => void) | undefined }) {
  const { t } = useI18n();
  return (
    <ul className="string-guide-list">
      {ruleOrder.map((rule) => {
        const group = macros.filter((macro) => macro.rule === rule);
        if (group.length === 0) return null;
        return (
          <li className="string-guide-group" key={rule}>
            <span className={`string-guide-rule is-${rule}`} title={t(RULE_HINTS[rule])}>{t(RULE_LABELS[rule])}</span>
            <div className="string-guide-chips">
              {group.map((macro, index) => {
                const state = missing === null ? null : macroState(macro, missing);
                return (
                  <span className={`string-guide-chip-wrap${state === "missing" ? " is-missing" : state === "present" ? " is-present" : ""}`} key={`${macro.label}:${index}`} title={state === "missing" ? t("hints.macro.missing") : state === "present" ? t("hints.macro.present") : undefined}>
                    <GuideChip macro={macro} onPick={onPick} />
                    {macro.count > 1 ? <span className="string-guide-times">×{macro.count}</span> : null}
                    {state === "missing" ? <UiIcon icon="circleX" size="xs" className="string-guide-missing" /> : state === "present" ? <UiIcon icon="check" size="xs" className="string-guide-present" /> : null}
                  </span>
                );
              })}
            </div>
          </li>
        );
      })}
    </ul>
  );
}

function DraftIssues({ drafted, verdict, onException }: { drafted: boolean; verdict: DraftCheckDto | null; onException: ExceptionHandler }) {
  const { t } = useI18n();
  if (!drafted) return <p className="string-guide-note">{t("hints.check.empty")}</p>;
  if (!verdict) return <p className="string-guide-note"><span className="spinner spinner-xs" /> {t("common.loading")}</p>;
  if (verdict.issues.length === 0) return <p className="string-guide-ok"><UiIcon icon="circleCheck" size="xs" />{t("hints.check.ok")}</p>;
  return (
    <ul className="string-guide-list">
      {verdict.issues.map((issue, index) => (
        <li className={issue.advice ? "string-guide-issue is-advice" : "string-guide-issue"} key={`${issue.group}:${index}`}>
          <UiIcon icon={issue.advice ? "triangleAlert" : "circleAlert"} size="xs" />
          <span>{describeIssue(issue, t)}</span>
          {issue.kind === "staleTermException" && issue.term ? (
            <button className="string-guide-link" type="button" disabled={!onException} title={t("findings.removeException", { term: issue.term })} onClick={() => onException?.(issue.term!, false)}>
              {t("hints.term.removeStale")}
            </button>
          ) : null}
        </li>
      ))}
    </ul>
  );
}
