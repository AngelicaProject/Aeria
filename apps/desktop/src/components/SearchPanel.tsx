import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { normalizeCommandError, projectEditUndo, projectReplaceApply, projectReplacePreview, projectRetranslate, projectSearch, projectSearchEntries, projectTermException } from "../ipc";
import type { BulkEditDto, CommandError, EntryRefDto, ReplaceChangeDto, SearchCheck, SearchEntryDto, SearchField, SearchFileDto, SearchHitDto, SearchQueryDto, SearchResultDto, SearchState, SourceBinding } from "../types";
import { describeIssue, errorText, exceptionTerm, issueLabel } from "../issueText";
import { choose, chosenByPath, commonTerm, groupBySheet, hitKey, reconcileChosen, unchooseFiles, type Chosen, type SheetGroup } from "../searchResults";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import { IconButton } from "../ui/primitives/IconButton";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ReplacePreviewDialog } from "./ReplacePreviewDialog";
import { useStickyState } from "./GitShared";

type SearchPanelProps = {
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
  /** A bulk edit wrote files: the editor reads them again. */
  onWorkspaceChanged?: (() => void) | undefined;
  /** Shows the machine translation dialog, which follows a running run. */
  onOpenTranslate?: (() => void) | undefined;
  /** Text to search for, set from outside (the palette's `#`); a new `nonce` applies it again. */
  seed?: { text: string; nonce: number } | undefined;
};

/** How long typing pauses before a search runs. */
const DEBOUNCE_MS = 300;
/** How long changes of the project's files settle before the result is read again. */
const LIVE_MS = 500;
/** Characters of a long text shown around its first match. */
const SNIPPET = 140;
/** Up to this many strings, every file of a result opens expanded. */
const EXPANDED_UP_TO = 200;
/** Issue groups the summary shows before "more". */
const SUMMARY_ISSUES = 12;

const fieldLabels: Record<SearchField, MessageKey> = {
  translation: "search.field.translation",
  source: "search.field.source",
  note: "search.field.note",
  context: "search.field.context",
};

const stateLabels: Record<SearchState, MessageKey> = {
  untranslated: "search.state.untranslated",
  translated: "search.state.translated",
  fuzzy: "search.state.fuzzy",
};

const skipLabels: Record<BulkEditDto["skipped"][number]["reason"], MessageKey> = {
  changed: "search.skipped.changed",
  missing: "search.skipped.missing",
  invalid: "search.skipped.invalid",
  broken: "search.skipped.broken",
};

/** Splits `text` into plain and highlighted parts by UTF-16 `ranges`, shortened around the first match. */
export function highlight(text: string, ranges: readonly (readonly [number, number])[]): ReactNode[] {
  const first = ranges[0]?.[0] ?? 0;
  const start = text.length > SNIPPET && first > SNIPPET / 3 ? first - Math.floor(SNIPPET / 3) : 0;
  const end = Math.min(text.length, start + SNIPPET);
  const parts: ReactNode[] = [];
  let at = start;
  if (start > 0) parts.push("…");
  for (const [from, to] of ranges) {
    if (to <= at || from >= end) continue;
    const begin = Math.max(from, at);
    if (begin > at) parts.push(text.slice(at, begin));
    parts.push(<mark key={`${from}:${to}`}>{text.slice(begin, Math.min(to, end))}</mark>);
    at = Math.min(to, end);
  }
  if (at < end) parts.push(text.slice(at, end));
  if (end < text.length) parts.push("…");
  return parts;
}

function entryRef(hit: SearchEntryDto): EntryRefDto {
  return { path: hit.path, context: hit.context, expectedText: hit.translation, expectedFuzzy: hit.fuzzy };
}

function coordinateOf(hit: SearchHitDto): string {
  return hit.binding ? `${hit.binding.rowId}:${hit.binding.subrowId} · ${hit.binding.columnIndex}` : hit.context;
}

/** Project search: find in every file of `po/`, replace in translations after a preview, and translate found strings again. */
export function SearchPanel({ onRevealBinding, onWorkspaceChanged, onOpenTranslate, seed }: SearchPanelProps) {
  const { t } = useI18n();
  const { preferences } = usePreferences();
  // The query outlives the panel while the window lives, as the Git dock's view does.
  const [text, setText] = useStickyState("search.text", "");
  const [caseSensitive, setCaseSensitive] = useStickyState("search.case", false);
  const [wholeWord, setWholeWord] = useStickyState("search.word", false);
  const [regex, setRegex] = useStickyState("search.regex", false);
  const [replaceOpen, setReplaceOpen] = useStickyState("search.replaceOpen", false);
  const [replaceText, setReplaceText] = useStickyState("search.replace", "");
  const [preserveCase, setPreserveCase] = useStickyState("search.preserveCase", true);
  const [fields, setFields] = useStickyState<SearchField[]>("search.fields", ["translation", "source"]);
  const [filtersOpen, setFiltersOpen] = useStickyState("search.filtersOpen", false);
  const [pathsText, setPathsText] = useStickyState("search.paths", "");
  const [states, setStates] = useStickyState<SearchState[]>("search.states", []);
  const [check, setCheck] = useStickyState<SearchCheck>("search.check", "any");
  const [result, setResult] = useState<SearchResultDto | null>(null);
  const [searching, setSearching] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<{ text: string; edit: BulkEditDto | null } | null>(null);
  const [undoAvailable, setUndoAvailable] = useState(false);
  const [preview, setPreview] = useState<ReplaceChangeDto[] | null>(null);
  const [retranslate, setRetranslate] = useState<EntryRefDto[] | null>(null);
  const [issueGroup, setIssueGroup] = useStickyState<string | null>("search.issue", null);
  // Sheets whose state differs from the default, `allOpen`: open for a small
  // result and closed otherwise, decided when the query changes.
  const [toggled, setToggled] = useState<ReadonlySet<string>>(new Set());
  const [allOpen, setAllOpen] = useState<boolean | null>(null);
  const [fileHits, setFileHits] = useState<ReadonlyMap<string, SearchHitDto[] | "loading">>(new Map());
  const [summaryOpen, setSummaryOpen] = useState(false);
  // Sheets whose strings were cut when loaded: more than one search returns.
  const [cut, setCut] = useState<ReadonlySet<string>>(new Set());
  const [selected, setSelected] = useState<Chosen>(new Map());
  const searchRun = useRef(0);
  // The query of the shown result: a result of the same query, read again
  // after the files changed, keeps the open sheets and the chosen strings.
  const shownQuery = useRef<string | null>(null);
  const fileHitsRef = useRef(fileHits);
  fileHitsRef.current = fileHits;

  useEffect(() => {
    if (seed) {
      setText(seed.text);
      setRegex(false);
    }
    // Only a new seed applies; the setters are stable.
  }, [seed?.nonce]);

  const query = useMemo((): SearchQueryDto => ({
    text,
    kind: regex ? "regex" : wholeWord ? "word" : "text",
    caseSensitive,
    fields,
    paths: pathsText.split(/[,\s]+/).map((path) => path.trim()).filter(Boolean),
    contexts: [],
    states,
    check,
    issue: check === "any" ? null : issueGroup,
  }), [caseSensitive, check, fields, issueGroup, pathsText, regex, states, text, wholeWord]);
  const active = query.text !== "" || query.check !== "any" || query.states.length > 0;

  const runSearch = useCallback(async (current: SearchQueryDto) => {
    const run = ++searchRun.current;
    setSearching(true);
    setError(null);
    try {
      const found = await projectSearch(current);
      if (run !== searchRun.current) return;
      const key = JSON.stringify(current);
      if (key !== shownQuery.current) {
        shownQuery.current = key;
        setToggled(new Set());
        setAllOpen(found.total <= EXPANDED_UP_TO);
        setFileHits(new Map());
        setCut(new Set());
        setSelected(new Map());
        setResult(found);
        return;
      }
      // The same query again: the sheets read before are read again in one
      // search, and their old strings stay until then.
      setResult(found);
      const paths = [...fileHitsRef.current.entries()].filter(([, hits]) => hits !== "loading").map(([path]) => path);
      const stillFound = new Set(found.files.map((file) => file.path));
      const reread = paths.filter((path) => stillFound.has(path));
      if (reread.length === 0) {
        setFileHits(new Map());
        return;
      }
      const again = await projectSearch({ ...current, paths: reread });
      if (run !== searchRun.current) return;
      setFileHits(new Map(reread.map((path) => [path, again.hits.filter((hit) => hit.path === path)])));
    } catch (caught) {
      if (run === searchRun.current) {
        setResult(null);
        setError(normalizeCommandError(caught));
      }
    } finally {
      if (run === searchRun.current) setSearching(false);
    }
  }, []);

  useEffect(() => {
    if (!active) {
      searchRun.current += 1;
      setResult(null);
      setSearching(false);
      setError(null);
      shownQuery.current = null;
      return;
    }
    const timer = window.setTimeout(() => void runSearch(query), DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [active, query, runSearch]);

  // Live: when the project's files change (a save, a bulk edit, machine
  // translation, Git), the result is read again for the same query.
  const liveRef = useRef<() => void>(() => undefined);
  liveRef.current = () => { if (active && busy === null) void runSearch(query); };
  useEffect(() => {
    let timer: number | undefined;
    const subscription = listen("project://changed", () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => liveRef.current(), LIVE_MS);
    });
    return () => {
      window.clearTimeout(timer);
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);

  const refresh = useCallback(() => {
    onWorkspaceChanged?.();
    if (active) void runSearch(query);
  }, [active, onWorkspaceChanged, query, runSearch]);

  const describeEdit = useCallback((edit: BulkEditDto, done: MessageKey) => {
    setUndoAvailable(edit.undoAvailable);
    const textOf = edit.skipped.length > 0
      ? `${t(done, { count: edit.done })} ${t("search.skippedCount", { count: edit.skipped.length })}`
      : t(done, { count: edit.done });
    setNotice({ text: textOf, edit: edit.skipped.length > 0 ? edit : null });
  }, [t]);

  const act = useCallback(async (label: string, action: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setBusy(null);
    }
  }, []);

  const openPreview = (scope: Partial<SearchQueryDto>) => void act("preview", async () => {
    const changes = await projectReplacePreview({ ...query, ...scope }, { text: replaceText, preserveCase });
    if (changes.length === 0) setNotice({ text: t("search.nothingToReplace"), edit: null });
    else setPreview(changes);
  });

  const applyChanges = (changes: readonly ReplaceChangeDto[]) => {
    setPreview(null);
    void act("apply", async () => {
      const edit = await projectReplaceApply(changes.map((change) => ({ path: change.path, context: change.context, expectedText: change.before, expectedFuzzy: change.fuzzy, after: change.after })));
      describeEdit(edit, "search.replaced");
      refresh();
    });
  };

  const replaceOne = (hit: SearchHitDto) => void act("apply", async () => {
    const changes = await projectReplacePreview({ ...query, paths: [hit.path], contexts: [hit.context] }, { text: replaceText, preserveCase });
    const valid = changes.filter((change) => change.problems.length === 0);
    if (changes.length > 0 && valid.length === 0) {
      setError({ code: "translationInvalid", message: (changes[0]?.problems ?? []).map((issue) => describeIssue(issue, t)).join("; ") });
      return;
    }
    const edit = await projectReplaceApply(valid.map((change) => ({ path: change.path, context: change.context, expectedText: change.before, expectedFuzzy: change.fuzzy, after: change.after })));
    describeEdit(edit, "search.replaced");
    refresh();
  });

  const startRetranslate = (entries: EntryRefDto[]) => {
    setRetranslate(null);
    void act("retranslate", async () => {
      if (!preferences.translationModel) {
        setError({ code: "modelNotChosen", message: t("search.chooseModel") });
        return;
      }
      const edit = await projectRetranslate(entries, preferences.translationModel, preferences.translationEffort || null);
      describeEdit(edit, "search.retranslating");
      onWorkspaceChanged?.();
      onOpenTranslate?.();
      if (active) void runSearch(query);
    });
  };

  const exceptTermNow = async (entries: EntryRefDto[], term: string) => {
    const edit = await projectTermException(entries, term, true);
    describeEdit(edit, "search.excepted");
    refresh();
  };
  const exceptTerm = (entries: EntryRefDto[], term: string) => void act("exception", () => exceptTermNow(entries, term));

  const undo = () => void act("undo", async () => {
    const edit = await projectEditUndo();
    describeEdit(edit, "search.undone");
    setUndoAvailable(false);
    refresh();
  });

  const toggle = <T,>(list: readonly T[], value: T): T[] => (list.includes(value) ? list.filter((item) => item !== value) : [...list, value]);

  // Hits of the result by file; a file may have more strings than were sent.
  const hitsByPath = useMemo(() => {
    const byPath = new Map<string, SearchHitDto[]>();
    for (const hit of result?.hits ?? []) {
      const list = byPath.get(hit.path);
      if (list) list.push(hit);
      else byPath.set(hit.path, [hit]);
    }
    return byPath;
  }, [result]);
  const allLoaded = result !== null && result.total === result.hits.length;
  const groups = useMemo(() => groupBySheet(result?.files ?? []), [result]);
  const defaultOpen = allOpen ?? false;
  const isOpen = (key: string) => defaultOpen !== toggled.has(key);

  /** A file's strings when all are at hand, "loading", or null when they must be read. */
  const fileHitsOf = (file: SearchFileDto): SearchHitDto[] | "loading" | null => {
    const loaded = fileHits.get(file.path);
    if (loaded) return loaded;
    const sent = hitsByPath.get(file.path) ?? [];
    return sent.length >= file.count ? sent : null;
  };

  const hitsOf = (group: SheetGroup): SearchHitDto[] | "loading" | null => {
    const hits: SearchHitDto[] = [];
    let missing = false;
    for (const file of group.files) {
      const found = fileHitsOf(file);
      if (found === "loading") return "loading";
      if (found === null) missing = true;
      else hits.push(...found);
    }
    return missing ? null : hits;
  };

  const loadGroup = useCallback(async (group: SheetGroup) => {
    const paths = group.files.filter((file) => fileHitsOf(file) === null).map((file) => file.path);
    setFileHits((current) => { const next = new Map(current); for (const path of paths) next.set(path, "loading"); return next; });
    try {
      const found = await projectSearch({ ...query, paths });
      setFileHits((current) => {
        const next = new Map(current);
        for (const path of paths) next.set(path, found.hits.filter((hit) => hit.path === path));
        return next;
      });
      if (found.total > found.hits.length) setCut((current) => new Set(current).add(group.key));
    } catch (caught) {
      setFileHits((current) => { const next = new Map(current); for (const path of paths) next.delete(path); return next; });
      setError(normalizeCommandError(caught));
    }
    // fileHitsOf reads the state of this render, which made the callback.
  }, [query, fileHits, hitsByPath]);

  const toggleGroup = (group: SheetGroup) => {
    const opening = !isOpen(group.key);
    setToggled((current) => { const next = new Set(current); if (next.has(group.key)) next.delete(group.key); else next.add(group.key); return next; });
    if (opening && hitsOf(group) === null) void loadGroup(group);
  };

  useEffect(() => {
    if (!result) return;
    const shown: SearchHitDto[] = [...result.hits];
    const known = new Set<string>();
    for (const file of result.files) {
      const loaded = fileHits.get(file.path);
      if (Array.isArray(loaded)) {
        shown.push(...loaded);
        known.add(file.path);
      } else if ((hitsByPath.get(file.path)?.length ?? 0) >= file.count) {
        known.add(file.path);
      }
    }
    const found = new Set(result.files.map((file) => file.path));
    setSelected((current) => reconcileChosen(current, shown, (path) => known.has(path) || !found.has(path)));
  }, [result, fileHits, hitsByPath]);

  const setChosen = (strings: readonly SearchEntryDto[], on: boolean) => setSelected((current) => choose(current, strings, on));

  // Every string of the result, or of its files `paths`: those at hand when
  // all are, otherwise read again without the limit of a search.
  const everyString = async (paths: readonly string[] | null, atHand: readonly SearchEntryDto[] | null): Promise<SearchEntryDto[] | null> => {
    if (atHand) return [...atHand];
    const key = JSON.stringify(query);
    const strings = await projectSearchEntries(paths === null ? query : { ...query, paths: [...paths] });
    // A result of another query is shown by now: the strings are not its.
    return shownQuery.current === key ? strings : null;
  };

  const chooseEvery = (paths: readonly string[] | null, atHand: readonly SearchEntryDto[] | null, on: boolean) => {
    if (!on) {
      setSelected((current) => (paths === null ? new Map() : unchooseFiles(current, new Set(paths))));
      return;
    }
    if (atHand) {
      setChosen(atHand, true);
      return;
    }
    void act("choose", async () => {
      const strings = await everyString(paths, null);
      if (strings) setChosen(strings, true);
    });
  };

  const translatedHits = useMemo(() => (result?.hits ?? []).filter((hit) => hit.translation !== ""), [result]);
  // The chosen group is a term's: every string found can take an exception for it.
  const groupTerm = useMemo(() => {
    if (issueGroup === null) return null;
    const issue = result?.issues.find((count) => count.issue.group === issueGroup)?.issue;
    return issue ? exceptionTerm(issue) : null;
  }, [issueGroup, result]);
  const complete = result !== null && result.total === result.hits.length && !result.cancelled;
  const chosen = useMemo(() => [...selected.values()], [selected]);
  const chosenCounts = useMemo(() => chosenByPath(selected), [selected]);
  const chosenOf = (paths: readonly string[]) => paths.reduce((sum, path) => sum + (chosenCounts.get(path) ?? 0), 0);
  const chosenInResult = result ? chosenOf(result.files.map((file) => file.path)) : 0;
  const chosenTranslated = chosen.filter((hit) => hit.translation !== "");
  const chosenTerm = groupTerm ?? commonTerm(chosen);
  const disabled = busy !== null;
  // A cut result may have translations past the strings listed.
  const mayHaveTranslated = translatedHits.length > 0 || (result !== null && !complete);

  const translatedOfResult = async (): Promise<EntryRefDto[] | null> => {
    const strings = await everyString(null, complete && result ? result.hits : null);
    if (!strings) return null;
    const translated = strings.filter((string) => string.translation !== "").map(entryRef);
    if (translated.length === 0) setNotice({ text: t("search.nothingTranslated"), edit: null });
    return translated.length > 0 ? translated : null;
  };

  const retranslateAll = () => void act("choose", async () => {
    const translated = await translatedOfResult();
    if (translated) setRetranslate(translated);
  });

  const exceptAll = (term: string) => void act("exception", async () => {
    const translated = await translatedOfResult();
    if (translated) await exceptTermNow(translated, term);
  });

  return (
    <section className="tool-content search-tool" aria-label={t("tool.searchLabel")}>
      <div className="search-form">
        <div className="search-row">
          <IconButton className="search-expand" icon={replaceOpen ? "chevronDown" : "chevronRight"} label={t("search.toggleReplace")} onClick={() => setReplaceOpen(!replaceOpen)} />
          <div className="search-query">
            <input className="input" type="search" autoFocus value={text} placeholder={t("search.placeholder")} aria-label={t("search.placeholder")} spellCheck={false} onChange={(event) => setText(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void runSearch(query); }} />
            <div className="search-toggles">
              <IconButton icon="caseSensitive" size="xs" label={t("search.caseSensitive")} pressed={caseSensitive} onClick={() => setCaseSensitive(!caseSensitive)} />
              <IconButton icon="wholeWord" size="xs" label={t("search.wholeWord")} pressed={wholeWord && !regex} disabled={regex} onClick={() => setWholeWord(!wholeWord)} />
              <IconButton icon="regex" size="xs" label={t("search.regex")} pressed={regex} onClick={() => setRegex(!regex)} />
            </div>
          </div>
        </div>
        {replaceOpen ? (
          <div className="search-row">
            <span className="search-expand" />
            <div className="search-query">
              <input className="input" value={replaceText} placeholder={t("search.replacePlaceholder")} aria-label={t("search.replacePlaceholder")} spellCheck={false} onChange={(event) => setReplaceText(event.target.value)} />
              <div className="search-toggles">
                <IconButton icon="preserveCase" size="xs" label={t("search.preserveCase")} pressed={preserveCase} onClick={() => setPreserveCase(!preserveCase)} />
                <IconButton icon="replaceAll" size="xs" label={t("search.replaceAll")} disabled={disabled || !result || result.total === 0 || text === ""} onClick={() => openPreview({})} />
              </div>
            </div>
          </div>
        ) : null}
        <div className="search-chips" role="group" aria-label={t("search.fields")}>
          {(Object.keys(fieldLabels) as SearchField[]).map((field) => (
            <button key={field} type="button" className={fields.includes(field) ? "chip-toggle on" : "chip-toggle"} aria-pressed={fields.includes(field)} onClick={() => setFields(toggle(fields, field))}>{t(fieldLabels[field])}</button>
          ))}
          <button type="button" className={filtersOpen ? "link-button search-filters-toggle open" : "link-button search-filters-toggle"} onClick={() => setFiltersOpen(!filtersOpen)}>
            <UiIcon icon="listFilter" size="xs" />{t("search.filters")}{pathsText || states.length > 0 || check !== "any" ? " •" : ""}
          </button>
        </div>
        {filtersOpen ? (
          <div className="search-filters">
            <label className="search-filter">
              <span>{t("search.paths")}</span>
              <input className="input" value={pathsText} placeholder={t("search.pathsPlaceholder")} spellCheck={false} onChange={(event) => setPathsText(event.target.value)} />
            </label>
            <div className="search-chips" role="group" aria-label={t("search.states")}>
              {(Object.keys(stateLabels) as SearchState[]).map((state) => (
                <button key={state} type="button" className={states.includes(state) ? "chip-toggle on" : "chip-toggle"} aria-pressed={states.includes(state)} onClick={() => setStates(toggle(states, state))}>{t(stateLabels[state])}</button>
              ))}
            </div>
            <Select<SearchCheck>
              value={check}
              onChange={(value) => { setCheck(value); setIssueGroup(null); }}
              label={t("search.check")}
              options={[
                { value: "any", label: t("search.check.any") },
                { value: "problems", label: t("search.check.problems"), hint: t("search.check.problemsHint") },
                { value: "advice", label: t("search.check.advice"), hint: t("search.check.adviceHint") },
              ]}
            />
          </div>
        ) : null}
      </div>

      {error ? <div className="git-feedback error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{errorText(error, t)}</span></div> : null}
      {notice ? (
        <div className="git-feedback" role="status">
          <UiIcon icon="info" size="sm" />
          <span>
            {notice.text}
            {undoAvailable ? <> <button className="link-button" type="button" disabled={disabled} onClick={undo}>{t("search.undo")}</button></> : null}
          </span>
        </div>
      ) : null}
      {notice?.edit ? (
        <ul className="search-skipped">
          {notice.edit.skipped.slice(0, 50).map((skipped) => (
            <li key={`${skipped.path}|${skipped.context}`}>
              <button className="link-button" type="button" disabled={!skipped.binding} onClick={() => { if (skipped.binding) onRevealBinding?.(skipped.binding); }}>{skipped.binding ? `${skipped.binding.sheetName} ${skipped.binding.rowId}:${skipped.binding.subrowId}` : skipped.context}</button>
              <span className="muted"> — {t(skipLabels[skipped.reason])}{skipped.problems.length > 0 ? `: ${skipped.problems.map((issue) => describeIssue(issue, t)).join("; ")}` : ""}</span>
            </li>
          ))}
        </ul>
      ) : null}

      {!active ? (
        <div className="empty-state search-empty">
          <UiIcon icon="search" size="xl" />
          <p>{t("search.hint")}</p>
        </div>
      ) : result ? (
        <>
          <div className="search-summary">
            {result.total > 0 ? (
              <input
                type="checkbox"
                className="search-check"
                aria-label={t("search.chooseAll")}
                title={t("search.chooseAll")}
                disabled={disabled || result.cancelled}
                checked={chosenInResult >= result.total}
                ref={(element) => { if (element) element.indeterminate = chosenInResult > 0 && chosenInResult < result.total; }}
                onChange={(event) => chooseEvery(null, complete ? result.hits : null, event.target.checked)}
              />
            ) : null}
            <span className="muted">
              {searching ? <span className="spinner" /> : null}
              {result.total === 0 ? t("search.none") : t("search.summary", { strings: result.total, files: result.files.length })}
            </span>
            {result.files.length > 1 ? (
              <>
                <IconButton icon="chevronsUp" size="xs" label={t("search.collapseAll")} onClick={() => { setAllOpen(false); setToggled(new Set()); }} />
                <IconButton icon="chevronDown" size="xs" label={allLoaded ? t("search.expandAll") : t("search.expandAllUnavailable")} disabled={!allLoaded} onClick={() => { setAllOpen(true); setToggled(new Set()); }} />
              </>
            ) : null}
          </div>
          {check !== "any" && (result.issues.length > 0 || issueGroup !== null) ? (
            <div className="search-issues" role="group" aria-label={t("search.issues")}>
              {issueGroup !== null ? <button type="button" className="chip-toggle" onClick={() => setIssueGroup(null)}><UiIcon icon="x" size="xs" />{t("search.issueClear")}</button> : null}
              {(summaryOpen ? result.issues : result.issues.slice(0, SUMMARY_ISSUES)).map(({ issue, count }) => (
                <button key={issue.group} type="button" className={`chip-toggle${issue.advice ? " advice" : ""}${issue.group === issueGroup ? " on" : ""}`} aria-pressed={issue.group === issueGroup} title={`${describeIssue(issue, t)} — ${t("search.issueFilter")}`} onClick={() => setIssueGroup(issue.group === issueGroup ? null : issue.group)}>
                  {issueLabel(issue, t)}<span className="search-issue-count">{count}</span>
                </button>
              ))}
              {!summaryOpen && result.issues.length > SUMMARY_ISSUES ? <button type="button" className="link-button" onClick={() => setSummaryOpen(true)}>{t("search.moreIssues", { count: result.issues.length - SUMMARY_ISSUES })}</button> : null}
            </div>
          ) : null}
          {chosen.length > 0 ? (
            <div className="search-summary search-selection">
              <span>{t("search.chosen", { count: chosen.length })}</span>
              <button className="button button-ghost" type="button" disabled={disabled || chosenTranslated.length === 0} onClick={() => setRetranslate(chosenTranslated.map(entryRef))}>
                <UiIcon icon="sparkles" size="sm" />{t("search.retranslateChosen")}
              </button>
              <button className="button button-ghost" type="button" disabled={disabled || chosenTerm === null} title={chosenTerm === null ? t("search.exceptChosenNoTerm") : t("search.exceptAllTitle", { term: chosenTerm })} onClick={() => { if (chosenTerm !== null) exceptTerm(chosen.map(entryRef), chosenTerm); }}>
                <UiIcon icon="bookX" size="sm" />{chosenTerm === null ? t("search.exceptChosenNone") : t("search.exceptChosen", { term: chosenTerm })}
              </button>
              <button className="link-button" type="button" onClick={() => setSelected(new Map())}>{t("search.clearChosen")}</button>
            </div>
          ) : null}
          <div className="search-summary">
            {mayHaveTranslated ? (
              <button className="button button-ghost" type="button" disabled={disabled || result.cancelled} title={t("search.retranslateTitle")} onClick={retranslateAll}>
                <UiIcon icon="sparkles" size="sm" />{t("search.retranslateAll")}
              </button>
            ) : null}
            {groupTerm !== null && mayHaveTranslated ? (
              <button className="button button-ghost" type="button" disabled={disabled || result.cancelled} title={t("search.exceptAllTitle", { term: groupTerm })} onClick={() => exceptAll(groupTerm)}>
                <UiIcon icon="bookX" size="sm" />{t("search.exceptAll", { term: groupTerm })}
              </button>
            ) : null}
            {replaceOpen && result.total > 0 && text !== "" ? (
              <button className="button button-secondary" type="button" disabled={disabled} onClick={() => openPreview({})}>
                <UiIcon icon="replaceAll" size="sm" />{t("search.replaceAllPreview")}
              </button>
            ) : null}
          </div>
          <div className="search-results">
            {groups.map((group) => {
              const open = isOpen(group.key);
              const hits = open ? hitsOf(group) : null;
              const paths = group.files.map((file) => file.path);
              const chosenHere = chosenOf(paths);
              const folder = group.files.length > 1 ? group.files[0]?.path.split("/").slice(0, -1).join("/") : group.files[0]?.path;
              return (
                <div className="search-group" key={group.key}>
                  <div className="search-group-head">
                    <input
                      type="checkbox"
                      className="search-check"
                      aria-label={t("search.chooseSheet", { sheet: group.sheet })}
                      disabled={disabled}
                      checked={group.count > 0 && chosenHere >= group.count}
                      ref={(element) => { if (element) element.indeterminate = chosenHere > 0 && chosenHere < group.count; }}
                      onChange={(event) => {
                        const atHand = hitsOf(group);
                        chooseEvery(paths, Array.isArray(atHand) && atHand.length >= group.count ? atHand : null, event.target.checked);
                      }}
                    />
                    <button type="button" className="search-group-toggle" aria-expanded={open} onClick={() => toggleGroup(group)}>
                      <UiIcon icon={open ? "chevronDown" : "chevronRight"} size="xs" />
                      <span className="search-group-name" title={group.files.map((file) => file.path).join("\n")}>{group.sheet}</span>
                      {folder && folder.replace(/\.po$/, "") !== group.sheet ? <span className="muted search-group-path">{group.files.length > 1 ? t("search.files", { count: group.files.length }) : folder}</span> : null}
                      <span className="git-count">{group.count}</span>
                    </button>
                    {replaceOpen && text !== "" ? <IconButton icon="replaceAll" size="xs" label={t("search.replaceInSheet")} disabled={disabled} onClick={() => openPreview({ paths: group.files.map((file) => file.path) })} /> : null}
                  </div>
                  {open ? (
                    hits === "loading" || hits === null ? (
                      <div className="search-loading muted"><span className="spinner" />{t("search.loadingFile")}</div>
                    ) : (
                      <ul className="search-hits">
                        {hits.map((hit) => {
                          const translationRanges = hit.matches.find((found) => found.field === "translation")?.ranges ?? [];
                          const sourceMatch = hit.matches.find((found) => found.field === "source");
                          const other = hit.matches.find((found) => found.field === "note" || found.field === "context");
                          const findings = hit.findings.map((issue) => describeIssue(issue, t));
                          const advice = hit.findings[0]?.advice ?? false;
                          const term = hit.findings.map(exceptionTerm).find((found) => found !== null) ?? null;
                          const isChosen = selected.has(hitKey(hit));
                          return (
                            <li key={hitKey(hit)} className={isChosen ? "search-hit chosen" : "search-hit"}>
                              <input type="checkbox" className="search-check" aria-label={t("search.choose")} checked={isChosen} onChange={(event) => setChosen([hit], event.target.checked)} />
                              <button type="button" className="search-hit-main" disabled={!hit.binding} onClick={() => { if (hit.binding) onRevealBinding?.(hit.binding); }}>
                                <span className="search-hit-coordinate mono">{coordinateOf(hit)}{hit.fuzzy ? <span className="chip chip-warn">fuzzy</span> : null}</span>
                                <span className="search-hit-text">{hit.translation ? highlight(hit.translation, translationRanges) : <em className="muted">{t("search.untranslated")}</em>}</span>
                                {sourceMatch ? <span className="search-hit-source">{highlight(hit.source, sourceMatch.ranges)}</span> : null}
                                {other ? <span className="search-hit-source">{highlight(other.field === "note" ? hit.note ?? "" : hit.context, other.ranges)}</span> : null}
                                {findings.length > 0 ? (
                                  <span className={advice ? "search-hit-findings advice" : "search-hit-findings"} title={findings.join("\n")}>
                                    {findings[0]}{findings.length > 1 ? <span className="search-issue-count">{t("search.moreFindings", { count: findings.length - 1 })}</span> : null}
                                  </span>
                                ) : null}
                              </button>
                              <span className="search-hit-actions">
                                {replaceOpen && translationRanges.length > 0 ? <IconButton icon="replace" size="xs" label={t("search.replaceOne")} disabled={disabled} onClick={() => replaceOne(hit)} /> : null}
                                {term !== null ? <IconButton icon="bookX" size="xs" label={t("search.exceptOne", { term })} disabled={disabled} onClick={() => exceptTerm([entryRef(hit)], term)} /> : null}
                                {hit.translation ? <IconButton icon="sparkles" size="xs" label={t("search.retranslateOne")} disabled={disabled} onClick={() => setRetranslate([entryRef(hit)])} /> : null}
                              </span>
                            </li>
                          );
                        })}
                        {cut.has(group.key) ? <li className="search-loading muted">{t("search.sheetCut", { shown: hits.length, count: group.count })}</li> : null}
                      </ul>
                    )
                  ) : null}
                </div>
              );
            })}
          </div>
        </>
      ) : searching ? (
        <div className="panel-state"><span className="spinner" />{t(check === "any" ? "search.searching" : "search.checking")}</div>
      ) : null}

      <ReplacePreviewDialog changes={preview} onCancel={() => setPreview(null)} onApply={applyChanges} onRevealBinding={onRevealBinding} />
      <ConfirmDialog
        open={retranslate !== null}
        title={t("search.retranslateConfirmTitle")}
        message={t("search.retranslateConfirm", { count: retranslate?.length ?? 0 })}
        confirmLabel={t("search.retranslateConfirmAction")}
        cancelLabel={t("common.cancel")}
        onKeepEditing={() => setRetranslate(null)}
        onDiscard={() => { if (retranslate) startRetranslate(retranslate); }}
      />
    </section>
  );
}
