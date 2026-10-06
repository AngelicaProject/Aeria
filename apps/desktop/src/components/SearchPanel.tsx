import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { DropdownMenu } from "radix-ui";
import { normalizeCommandError, projectEditUndo, projectReplaceApply, projectReplacePreview, projectRetranslate, projectSearch, projectSearchEntries, projectSearchPrepare, projectTermException } from "../ipc";
import type { BulkEditDto, CommandError, EntryRefDto, ReplaceChangeDto, SearchCheck, SearchEntryDto, SearchField, SearchFileDto, SearchHitDto, SearchQueryDto, SearchResultDto, SearchState, SourceBinding } from "../types";
import { describeIssue, errorText, exceptionTerm } from "../issueText";
import { choose, chosenByPath, commonTerm, groupBySheet, reconcileChosen, resultRows, sheetOpen, unchooseFiles, type Chosen, type GroupHits, type SheetGroup } from "../searchResults";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import { useNameSheets } from "../ui/NameSheetMark";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ReplacePreviewDialog } from "./ReplacePreviewDialog";
import { useStickyState } from "./GitShared";
import { SearchResultList, chooseKey, type SearchResultListHandle } from "./SearchResultList";
import { SearchIssuePicker } from "./SearchIssuePicker";
import { SearchScope, defaultFields, scopeNarrowed, type Scope } from "./SearchScope";


type SearchPanelProps = {
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
  /** A bulk edit wrote files: the editor reads them again. */
  onWorkspaceChanged?: (() => void) | undefined;
  /** Shows the machine translation dialog, which follows a running run. */
  onOpenTranslate?: (() => void) | undefined;
  /** Text to search for, set from outside (the palette's `#`); a new `nonce` applies it again. */
  seed?: { text: string; nonce: number } | undefined;
};

/**
 * How long typing pauses before a search runs. A search of the whole game
 * takes tens of milliseconds, so the result follows the typing.
 */
const DEBOUNCE_MS = 120;
/** Set once the person has chosen strings with Ctrl or Shift: the bar of chosen strings stops naming the keys. */
const KEYS_LEARNED = "aeria.search.chooseKeysLearned";

function keysLearned(): boolean {
  try {
    return localStorage.getItem(KEYS_LEARNED) === "1";
  } catch {
    return false;
  }
}

/** How long changes of the project's files settle before the result is read again. */
const LIVE_MS = 500;

const skipLabels: Record<BulkEditDto["skipped"][number]["reason"], MessageKey> = {
  changed: "search.skipped.changed",
  missing: "search.skipped.missing",
  invalid: "search.skipped.invalid",
  broken: "search.skipped.broken",
  reviewed: "search.skipped.reviewed",
};

function entryRef(hit: SearchEntryDto): EntryRefDto {
  return { path: hit.path, context: hit.context, expectedText: hit.translation, expectedFuzzy: hit.fuzzy };
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
  const [fields, setFields] = useStickyState<readonly SearchField[]>("search.fields", defaultFields);
  const [pathsText, setPathsText] = useStickyState("search.paths", "");
  // Only the sheets of names, which machine translation spreads through the project.
  const [nameSheetsOnly, setNameSheetsOnly] = useStickyState("search.nameSheets", false);
  const nameSheets = useNameSheets();
  const [states, setStates] = useStickyState<readonly SearchState[]>("search.states", []);
  const [check, setCheck] = useStickyState<SearchCheck>("search.check", "any");
  const [issueGroup, setIssueGroup] = useStickyState<string | null>("search.issue", null);
  const [result, setResult] = useState<SearchResultDto | null>(null);
  const [searching, setSearching] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<{ text: string; edit: BulkEditDto | null } | null>(null);
  const [undoAvailable, setUndoAvailable] = useState(false);
  const [preview, setPreview] = useState<ReplaceChangeDto[] | null>(null);
  const [retranslate, setRetranslate] = useState<EntryRefDto[] | null>(null);
  // Sheets open unless closed, when their strings are at hand; every sheet
  // is closed after "Collapse all". `toggled` holds the sheets the person
  // opened or closed against that.
  const [collapsed, setCollapsed] = useState(false);
  const [toggled, setToggled] = useState<ReadonlySet<string>>(new Set());
  const [fileHits, setFileHits] = useState<ReadonlyMap<string, SearchHitDto[] | "loading">>(new Map());
  // Sheets whose strings were cut when read: more than one search returns.
  const [cut, setCut] = useState<ReadonlySet<string>>(new Set());
  const [selected, setSelected] = useState<Chosen>(new Map());
  const [learned, setLearned] = useState(keysLearned);
  const learnKeys = useCallback(() => {
    if (learned) return;
    setLearned(true);
    try {
      localStorage.setItem(KEYS_LEARNED, "1");
    } catch {
      // Without storage the keys are named again in the next window.
    }
  }, [learned]);
  const searchRun = useRef(0);
  // The query of the shown result: a result of the same query, read again
  // after the files changed, keeps the open sheets and the chosen strings.
  const shownQuery = useRef<string | null>(null);
  const fileHitsRef = useRef(fileHits);
  fileHitsRef.current = fileHits;
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<SearchResultListHandle | null>(null);

  // The project's files are read in the background as the tool opens, so
  // the first search does not wait for them.
  useEffect(() => {
    void projectSearchPrepare().catch(() => undefined);
  }, []);

  useEffect(() => {
    if (seed) {
      setText(seed.text);
      setRegex(false);
      inputRef.current?.focus();
    }
    // Only a new seed applies; the setters are stable.
  }, [seed?.nonce]);

  const query = useMemo((): SearchQueryDto => ({
    text,
    kind: regex ? "regex" : wholeWord ? "word" : "text",
    caseSensitive,
    fields: [...fields],
    paths: nameSheetsOnly ? [...nameSheets] : pathsText.split(/[,\s]+/).map((path) => path.trim()).filter(Boolean),
    contexts: [],
    states: [...states],
    check,
    issue: check === "any" ? null : issueGroup,
  }), [caseSensitive, check, fields, issueGroup, nameSheets, nameSheetsOnly, pathsText, regex, states, text, wholeWord]);
  const active = query.text !== "" || query.check !== "any" || query.states.length > 0;

  const runSearch = useCallback(async (current: SearchQueryDto) => {
    const run = ++searchRun.current;
    setSearching(true);
    try {
      const found = await projectSearch(current);
      if (run !== searchRun.current) return;
      setError(null);
      const key = JSON.stringify(current);
      if (key !== shownQuery.current) {
        shownQuery.current = key;
        setToggled(new Set());
        setCollapsed(false);
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
        const failure = normalizeCommandError(caught);
        // A regular expression being typed is often incomplete: the last
        // result stays, dimmed, until the expression is whole again.
        if (failure.code !== "searchPattern") {
          shownQuery.current = null;
          setResult(null);
        }
        setError(failure);
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
  const groups = useMemo(() => groupBySheet(result?.files ?? []), [result]);

  /** A file's strings when all are at hand, "loading", or the strings sent of it. */
  const fileHitsOf = useCallback((file: SearchFileDto): { hits: readonly SearchHitDto[]; complete: boolean; loading: boolean } => {
    const loaded = fileHits.get(file.path);
    if (loaded === "loading") return { hits: hitsByPath.get(file.path) ?? [], complete: false, loading: true };
    if (loaded) return { hits: loaded, complete: true, loading: false };
    const sent = hitsByPath.get(file.path) ?? [];
    return { hits: sent, complete: sent.length >= file.count, loading: false };
  }, [fileHits, hitsByPath]);

  const hitsOf = useCallback((group: SheetGroup): GroupHits => {
    const hits: SearchHitDto[] = [];
    let complete = true;
    let loading = false;
    for (const file of group.files) {
      const found = fileHitsOf(file);
      hits.push(...found.hits);
      complete &&= found.complete;
      loading ||= found.loading;
    }
    return { hits, complete, loading, cut: cut.has(group.key) };
  }, [cut, fileHitsOf]);

  const isOpen = useCallback(
    (group: SheetGroup) => sheetOpen(group, (path) => hitsByPath.get(path)?.length ?? 0, collapsed, toggled),
    [collapsed, hitsByPath, toggled],
  );

  const loadGroup = useCallback(async (group: SheetGroup) => {
    const paths = group.files.filter((file) => !fileHitsOf(file).complete).map((file) => file.path);
    if (paths.length === 0) return;
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
  }, [query, fileHitsOf]);

  const toggleGroup = useCallback((group: SheetGroup) => {
    const opening = !isOpen(group);
    setToggled((current) => { const next = new Set(current); if (next.has(group.key)) next.delete(group.key); else next.add(group.key); return next; });
    if (opening && hitsOf(group).hits.length === 0) void loadGroup(group);
  }, [hitsOf, isOpen, loadGroup]);

  const rows = useMemo(() => resultRows(groups, isOpen, hitsOf), [groups, isOpen, hitsOf]);

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

  const setChosen = useCallback((strings: readonly SearchEntryDto[], on: boolean) => setSelected((current) => choose(current, strings, on)), []);

  // Every string of the result, or of its files `paths`: those at hand when
  // all are, otherwise read again without the limit of a search.
  const everyString = async (paths: readonly string[] | null, atHand: readonly SearchEntryDto[] | null): Promise<SearchEntryDto[] | null> => {
    if (atHand) return [...atHand];
    const key = JSON.stringify(query);
    const strings = await projectSearchEntries(paths === null ? query : { ...query, paths: [...paths] });
    // A result of another query is shown by now: the strings are not its.
    return shownQuery.current === key ? strings : null;
  };

  const complete = result !== null && result.total === result.hits.length && !result.cancelled;

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

  const chooseGroup = (group: SheetGroup, on: boolean) => {
    const atHand = hitsOf(group);
    chooseEvery(group.files.map((file) => file.path), atHand.complete ? atHand.hits : null, on);
  };

  const translatedHits = useMemo(() => (result?.hits ?? []).filter((hit) => hit.translation !== ""), [result]);
  // The chosen group is a term's: every string found can take an exception for it.
  const groupTerm = useMemo(() => {
    if (issueGroup === null) return null;
    const issue = result?.issues.find((count) => count.issue.group === issueGroup)?.issue;
    return issue ? exceptionTerm(issue) : null;
  }, [issueGroup, result]);
  const chosen = useMemo(() => [...selected.values()], [selected]);
  const chosenCounts = useMemo(() => chosenByPath(selected), [selected]);
  const chosenOf = useCallback((paths: readonly string[]) => paths.reduce((sum, path) => sum + (chosenCounts.get(path) ?? 0), 0), [chosenCounts]);
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

  // The issues to choose from are those of the result without an issue
  // chosen: with one chosen, the result counts only its strings.
  const queryWithoutIssue = JSON.stringify({ ...query, issue: null });
  const [allIssues, setAllIssues] = useState<{ key: string; issues: SearchResultDto["issues"] } | null>(null);
  useEffect(() => {
    if (result && query.issue === null && shownQuery.current === JSON.stringify(query)) setAllIssues({ key: queryWithoutIssue, issues: result.issues });
  }, [result]);
  const issueList = allIssues?.key === queryWithoutIssue ? allIssues.issues : result?.issues ?? [];

  const scope: Scope = { fields, pathsText, nameSheetsOnly, states, check };
  const changeScope = (change: Partial<Scope>) => {
    if (change.fields !== undefined) setFields(change.fields);
    if (change.pathsText !== undefined) setPathsText(change.pathsText);
    if (change.nameSheetsOnly !== undefined) setNameSheetsOnly(change.nameSheetsOnly);
    if (change.states !== undefined) setStates(change.states);
    if (change.check !== undefined) {
      setCheck(change.check);
      setIssueGroup(null);
    }
  };
  const clearScope = () => changeScope({ pathsText: "", nameSheetsOnly: false, states: [], check: "any" });
  const narrowed = scopeNarrowed(scope);

  const patternError = error?.code === "searchPattern" ? error : null;
  const otherError = patternError ? null : error;
  const replacing = replaceOpen && text !== "";
  const allClosed = rows.every((row) => row.kind !== "hit");

  return (
    <section className="tool-content search-tool" aria-label={t("tool.searchLabel")}>
      <div className="search-head">
        <div className={patternError ? "search-box invalid" : "search-box"}>
          <UiIcon icon="search" size="sm" />
          <input
            ref={inputRef}
            type="text"
            autoFocus
            value={text}
            placeholder={t("search.placeholder")}
            aria-label={t("search.placeholder")}
            aria-invalid={patternError !== null}
            spellCheck={false}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void runSearch(query);
              else if (event.key === "ArrowDown" && rows.length > 0) {
                event.preventDefault();
                listRef.current?.focus();
              } else if (event.key === "Escape" && text !== "") {
                event.preventDefault();
                setText("");
              }
            }}
          />
          {text !== "" ? <IconButton icon="x" size="xs" className="search-box-clear" label={t("search.clearText")} onClick={() => { setText(""); inputRef.current?.focus(); }} /> : null}
          <span className="search-box-toggles">
            <IconButton icon="caseSensitive" size="xs" label={t("search.caseSensitive")} pressed={caseSensitive} onClick={() => setCaseSensitive(!caseSensitive)} />
            <IconButton icon="wholeWord" size="xs" label={t("search.wholeWord")} pressed={wholeWord && !regex} disabled={regex} onClick={() => setWholeWord(!wholeWord)} />
            <IconButton icon="regex" size="xs" label={t("search.regex")} pressed={regex} onClick={() => setRegex(!regex)} />
          </span>
          <span className="search-box-divider" />
          <IconButton icon="replace" size="xs" label={t("search.toggleReplace")} pressed={replaceOpen} onClick={() => setReplaceOpen(!replaceOpen)} />
        </div>
        {patternError ? <p className="search-box-error" role="alert">{errorText(patternError, t)}</p> : null}
        {replaceOpen ? (
          <div className="search-box">
            <UiIcon icon="replace" size="sm" />
            <input value={replaceText} placeholder={t("search.replacePlaceholder")} aria-label={t("search.replacePlaceholder")} spellCheck={false} onChange={(event) => setReplaceText(event.target.value)} />
            <span className="search-box-toggles">
              <IconButton icon="preserveCase" size="xs" label={t("search.preserveCase")} pressed={preserveCase} onClick={() => setPreserveCase(!preserveCase)} />
            </span>
            <span className="search-box-divider" />
            <IconButton icon="replaceAll" size="xs" label={t("search.replaceAllPreview")} disabled={disabled || !result || result.total === 0 || text === ""} onClick={() => openPreview({})} />
          </div>
        ) : null}
        <SearchScope scope={scope} onChange={changeScope} onClear={clearScope}>
          {check !== "any" && (issueList.length > 0 || issueGroup !== null) ? <SearchIssuePicker issues={issueList} value={issueGroup} onChange={setIssueGroup} /> : null}
        </SearchScope>
      </div>

      {otherError ? <div className="git-feedback error search-error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{errorText(otherError, t)}</span></div> : null}

      {!active ? (
        <div className="search-start">
          <div className="search-quick">
            <span className="search-quick-label">{t("search.quick")}</span>
            <button type="button" className="scope-chip" onClick={() => changeScope({ states: ["untranslated"] })}>{t("search.state.untranslated")}</button>
            <button type="button" className="scope-chip" onClick={() => changeScope({ states: ["fuzzy"] })}>{t("search.state.fuzzy")}</button>
            <button type="button" className="scope-chip" onClick={() => changeScope({ check: "problems" })}>{t("search.check.problems")}</button>
            <button type="button" className="scope-chip" onClick={() => changeScope({ check: "advice" })}>{t("search.check.advice")}</button>
          </div>
        </div>
      ) : result ? (
        <div className={patternError ? "search-result stale" : "search-result"}>
          <div className="search-summary">
            <span className="search-count">
              {result.total === 0 ? t("search.none") : `${t("search.strings", { count: result.total })} · ${t("search.sheets", { count: groups.length })}`}
            </span>
            {searching && check !== "any" ? <span className="search-status">{t("search.checking")}</span> : null}
            <span className="search-summary-spacer" />
            {groups.length > 1 ? (
              <IconButton
                icon={allClosed ? "chevronDown" : "chevronsUp"}
                size="xs"
                label={allClosed ? t("search.expandAll") : t("search.collapseAll")}
                onClick={() => { setCollapsed(!allClosed); setToggled(new Set()); }}
              />
            ) : null}
            {result.total > 0 ? (
              <DropdownMenu.Root>
                <DropdownMenu.Trigger asChild>
                  <IconButton icon="ellipsis" size="xs" label={t("search.actions")} disabled={disabled} />
                </DropdownMenu.Trigger>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content className="menu-content" align="end" sideOffset={4} collisionPadding={8}>
                    <DropdownMenu.Label className="menu-label search-actions-label">{t("search.actionsAll", { count: result.total })}</DropdownMenu.Label>
                    <DropdownMenu.Item className="menu-item" disabled={result.cancelled} onSelect={() => chooseEvery(null, complete ? result.hits : null, true)}>
                      <span className="menu-item-label">{t("search.chooseAll")}</span>
                    </DropdownMenu.Item>
                    {mayHaveTranslated ? (
                      <DropdownMenu.Item className="menu-item" disabled={result.cancelled} title={t("search.retranslateTitle")} onSelect={retranslateAll}>
                        <span className="menu-item-label">{t("search.retranslateAll")}</span>
                      </DropdownMenu.Item>
                    ) : null}
                    {groupTerm !== null && mayHaveTranslated ? (
                      <DropdownMenu.Item className="menu-item" disabled={result.cancelled} title={t("search.exceptAllTitle", { term: groupTerm })} onSelect={() => exceptAll(groupTerm)}>
                        <span className="menu-item-label">{t("search.exceptAll", { term: groupTerm })}</span>
                      </DropdownMenu.Item>
                    ) : null}
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            ) : null}
            {searching ? <span className="search-progress" aria-hidden="true" /> : null}
          </div>
          {result.total === 0 ? (
            narrowed ? (
              <div className="search-nothing">
                <button type="button" className="link-button" onClick={clearScope}>{t("search.clearFilters")}</button>
              </div>
            ) : null
          ) : (
            <SearchResultList
              // A new query starts the list from the top.
              key={shownQuery.current ?? ""}
              ref={listRef}
              rows={rows}
              chosen={selected}
              chosenOf={chosenOf}
              nameSheets={nameSheets}
              replacing={replacing}
              disabled={disabled}
              onToggleGroup={toggleGroup}
              onShowAll={(group) => void loadGroup(group)}
              onReveal={(hit) => { if (hit.binding) onRevealBinding?.(hit.binding); }}
              onChoose={setChosen}
              onChooseGroup={chooseGroup}
              onReplaceOne={replaceOne}
              onReplaceGroup={(group) => openPreview({ paths: group.files.map((file) => file.path) })}
              onExcept={(hit, term) => exceptTerm([entryRef(hit)], term)}
              onRetranslate={(hit) => setRetranslate([entryRef(hit)])}
              onExit={() => inputRef.current?.focus()}
              onKeysUsed={learnKeys}
            />
          )}
        </div>
      ) : searching ? (
        <div className="panel-state"><span className="spinner" />{t(check === "any" ? "search.searching" : "search.checking")}</div>
      ) : null}

      {notice ? (
        <div className="search-message" role="status">
          <div className="git-feedback">
            <UiIcon icon="info" size="sm" />
            <span className="search-message-text">{notice.text}</span>
            {undoAvailable ? <button className="link-button" type="button" disabled={disabled} onClick={undo}>{t("search.undo")}</button> : null}
            <IconButton icon="x" size="xs" label={t("search.dismiss")} onClick={() => setNotice(null)} />
          </div>
          {notice.edit ? (
            <ul className="search-skipped">
              {notice.edit.skipped.slice(0, 50).map((skipped) => (
                <li key={`${skipped.path}|${skipped.context}`}>
                  <button className="link-button" type="button" disabled={!skipped.binding} onClick={() => { if (skipped.binding) onRevealBinding?.(skipped.binding); }}>{skipped.binding ? `${skipped.binding.sheetName} ${skipped.binding.rowId}:${skipped.binding.subrowId}` : skipped.context}</button>
                  <span className="muted"> — {t(skipLabels[skipped.reason])}{skipped.problems.length > 0 ? `: ${skipped.problems.map((issue) => describeIssue(issue, t)).join("; ")}` : ""}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}

      {chosen.length > 0 ? (
        <div className="search-footer" role="toolbar" aria-label={t("search.chosen", { count: chosen.length })}>
          <IconButton icon="x" size="xs" label={t("search.clearChosen")} onClick={() => setSelected(new Map())} />
          <span className="search-footer-count">{t("search.chosen", { count: chosen.length })}</span>
          <button className="button button-ghost" type="button" disabled={disabled || chosenTranslated.length === 0} onClick={() => setRetranslate(chosenTranslated.map(entryRef))}>
            <UiIcon icon="sparkles" size="sm" />{t("search.retranslateChosen")}
          </button>
          <button className="button button-ghost" type="button" disabled={disabled || chosenTerm === null} title={chosenTerm === null ? t("search.exceptChosenNoTerm") : t("search.exceptAllTitle", { term: chosenTerm })} onClick={() => { if (chosenTerm !== null) exceptTerm(chosen.map(entryRef), chosenTerm); }}>
            <UiIcon icon="bookX" size="sm" />{chosenTerm === null ? t("search.exceptChosenNone") : t("search.exceptChosen", { term: chosenTerm })}
          </button>
          {learned ? null : (
            <span className="search-footer-keys">
              <span><kbd>{chooseKey}</kbd>{t("search.keys.add")}</span>
              <span><kbd>Shift</kbd>{t("search.keys.range")}</span>
            </span>
          )}
        </div>
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
