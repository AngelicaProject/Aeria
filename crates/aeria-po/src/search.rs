//! Search and replace over the files of `po/`.
//!
//! A search reads the project's files, which are the only state, so it
//! always sees the current translations. Translations and sources match only
//! in their text: the content of macros (`<sheet Item $n1 0>`) never
//! matches, while text inside conditions (`<if $gn4>готова<else>готов</if>`)
//! does. A replacement changes only that text, so it never touches game data.
//! See `docs/architecture/search.md`.

use std::ops::Range;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use aeria_knowledge::Knowledge;
use aeria_se::{ExprKind, SyntaxKind, SyntaxNode};
use regex::{Captures, Regex, RegexBuilder};

use crate::check::{Issue, check_translation};
use crate::po::{Entry, PoFile};
use crate::project::{PO_DIR, list};

/// Most hits a search returns; the rest are counted.
pub const MAX_HITS: usize = 2000;

/// How the search text matches.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MatchKind {
    /// The text anywhere.
    #[default]
    Text,
    /// The text as whole words.
    Word,
    /// A regular expression (`regex` crate syntax).
    Regex,
}

/// What to look for.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Pattern {
    pub text: String,
    pub kind: MatchKind,
    pub case_sensitive: bool,
}

/// Which parts of an entry a pattern is matched against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // the toggles of a search form
pub struct Fields {
    pub translation: bool,
    pub source: bool,
    pub note: bool,
    /// The `msgctxt`, such as `Addon:2025:0:0`.
    pub context: bool,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            translation: true,
            source: true,
            note: false,
            context: false,
        }
    }
}

/// The part of an entry a match is in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    Translation,
    Source,
    Note,
    Context,
}

/// The state of an entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Untranslated,
    Translated,
    /// The source changed since the translation was written.
    Fuzzy,
}

/// Which translations to keep by the checks a translation is saved with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CheckFilter {
    #[default]
    Any,
    /// Translations with a problem: they are not exported.
    Problems,
    /// Translations with advice: what may be wrong, such as a term whose
    /// translation does not seem to be used. Advice does not keep a
    /// translation from being exported.
    Advice,
}

/// A search over the project.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Query {
    /// `None` keeps every entry the filters keep.
    pub pattern: Option<Pattern>,
    pub fields: Fields,
    /// Files and folders relative to `po/` (`quest/`, `Addon`, `Addon/2000.po`);
    /// empty for all of `po/`.
    pub paths: Vec<String>,
    /// Only these `msgctxt`; empty for any.
    pub contexts: Vec<String>,
    /// Entry states to keep; empty for all.
    pub states: Vec<State>,
    pub check: CheckFilter,
    /// With a check filter, only entries with an issue of this group (see
    /// [`Issue::group`]).
    pub issue: Option<String>,
}

/// The matches in one field, as byte ranges of its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldMatch {
    pub field: Field,
    pub ranges: Vec<Range<usize>>,
}

/// An entry a search found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hit {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub source: String,
    pub translation: String,
    pub fuzzy: bool,
    pub note: Option<String>,
    /// Empty when the search has no pattern.
    pub matches: Vec<FieldMatch>,
    /// The problems, or terms not used, when the search filters by them.
    pub findings: Vec<Issue>,
}

/// The entries a search found in one file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileHits {
    /// The file, relative to `po/`.
    pub path: String,
    /// The sheet of the file's entries.
    pub sheet: String,
    pub count: usize,
}

/// How many entries have issues of one group, with one of them to show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueCount {
    /// See [`Issue::group`].
    pub group: String,
    pub issue: Issue,
    pub count: usize,
}

/// The result of a search.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Found {
    /// The first [`MAX_HITS`] hits in file order.
    pub hits: Vec<Hit>,
    /// Every entry found.
    pub total: usize,
    /// Every match found, in all fields.
    pub matches: usize,
    /// Every file with a hit, in file order.
    pub files: Vec<FileHits>,
    /// With a check filter, the issues of every entry found by group, most
    /// entries first.
    pub issues: Vec<IssueCount>,
    /// The search was cancelled; the counts cover the files read until then.
    pub cancelled: bool,
}

/// Errors of a search.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("the search text is not a valid regular expression: {0}")]
    Pattern(String),
    #[error("the search text is empty")]
    EmptyPattern,
    #[error("po/{path}: {message}")]
    Read { path: String, message: String },
}

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Matcher {
    regex: Regex,
    /// The pattern can be looked for in a file's raw text before the file
    /// is parsed: a literal without characters the PO format escapes.
    prefilter: bool,
}

impl Matcher {
    /// Compiles a pattern.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::EmptyPattern`] or [`SearchError::Pattern`].
    pub fn new(pattern: &Pattern) -> Result<Self, SearchError> {
        if pattern.text.is_empty() {
            return Err(SearchError::EmptyPattern);
        }
        let source = match pattern.kind {
            MatchKind::Text => regex::escape(&pattern.text),
            MatchKind::Word => format!(r"\b{}\b", regex::escape(&pattern.text)),
            MatchKind::Regex => pattern.text.clone(),
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!pattern.case_sensitive)
            .size_limit(1 << 22)
            .build()
            .map_err(|error| SearchError::Pattern(error.to_string()))?;
        Ok(Self {
            regex,
            prefilter: pattern.kind != MatchKind::Regex
                && !pattern.text.contains(['"', '\\', '\n', '\t']),
        })
    }

    /// The matches in `text`, as byte ranges.
    fn find(&self, text: &str) -> Vec<Range<usize>> {
        self.regex
            .find_iter(text)
            .map(|found| found.range())
            .filter(|range| !range.is_empty())
            .collect()
    }

    /// The matches in the text of macro text: in its text ranges only.
    fn find_in_text(&self, macro_text: &str) -> Vec<Range<usize>> {
        text_ranges(macro_text)
            .into_iter()
            .flat_map(|range| {
                self.find(&macro_text[range.clone()])
                    .into_iter()
                    .map(move |found| found.start + range.start..found.end + range.start)
            })
            .collect()
    }
}

/// The byte ranges of macro text that are text a translator writes: its text
/// nodes, and those of translatable macro arguments such as condition
/// branches. Malformed text is one range.
#[must_use]
pub fn text_ranges(macro_text: &str) -> Vec<Range<usize>> {
    let document = aeria_se::parse(macro_text);
    if !document.is_well_formed() {
        return if macro_text.is_empty() {
            Vec::new()
        } else {
            std::iter::once(0..macro_text.len()).collect()
        };
    }
    let mut ranges = Vec::new();
    collect_text(document.nodes(), &mut ranges);
    ranges
}

fn collect_text(nodes: &[SyntaxNode], ranges: &mut Vec<Range<usize>>) {
    for node in nodes {
        match &node.kind {
            SyntaxKind::Text(_) => ranges.push(node.span.start()..node.span.end()),
            SyntaxKind::Macro(syntax) => {
                let Some(spec) = syntax.spec else { continue };
                for (index, arg) in syntax.args.iter().enumerate() {
                    if let ExprKind::Str(inner) = &arg.kind
                        && spec.is_translatable_arg(index)
                    {
                        collect_text(inner, ranges);
                    }
                }
            }
            SyntaxKind::Raw(_) | SyntaxKind::Error => {}
        }
    }
}

/// Whether a file is chosen: `paths` name files (with or without `.po`) and
/// folders, relative to `po/`.
#[must_use]
pub fn path_selected(path: &str, paths: &[String]) -> bool {
    paths.is_empty()
        || paths.iter().any(|chosen| {
            let chosen = chosen.trim().trim_matches('/');
            chosen.is_empty()
                || path == chosen
                || path.strip_suffix(".po") == Some(chosen)
                || path
                    .strip_prefix(chosen)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

fn state_of(entry: &Entry) -> State {
    if entry.fuzzy {
        State::Fuzzy
    } else if entry.translation.is_empty() {
        State::Untranslated
    } else {
        State::Translated
    }
}

/// What the search of one file needs.
struct Search<'a> {
    query: &'a Query,
    matcher: Option<Matcher>,
    knowledge: &'a Knowledge,
    target_language: &'a str,
}

impl Search<'_> {
    /// The hit of an entry, if the query keeps it.
    fn hit(&self, path: &str, entry: &Entry) -> Option<Hit> {
        let query = self.query;
        if !query.contexts.is_empty() && !query.contexts.contains(&entry.context) {
            return None;
        }
        if !query.states.is_empty() && !query.states.contains(&state_of(entry)) {
            return None;
        }
        let note = (!entry.notes.is_empty()).then(|| entry.notes.join("\n"));
        let mut matches = Vec::new();
        if let Some(matcher) = &self.matcher {
            let fields = query.fields;
            let mut add = |field, ranges: Vec<Range<usize>>| {
                if !ranges.is_empty() {
                    matches.push(FieldMatch { field, ranges });
                }
            };
            if fields.translation {
                add(Field::Translation, matcher.find_in_text(&entry.translation));
            }
            if fields.source {
                add(Field::Source, matcher.find_in_text(&entry.source));
            }
            if fields.note
                && let Some(note) = &note
            {
                add(Field::Note, matcher.find(note));
            }
            if fields.context {
                add(Field::Context, matcher.find(&entry.context));
            }
            if matches.is_empty() {
                return None;
            }
        }
        let findings = match query.check {
            CheckFilter::Any => Vec::new(),
            CheckFilter::Problems | CheckFilter::Advice => {
                if entry.translation.is_empty() {
                    return None;
                }
                let problems = query.check == CheckFilter::Problems;
                let mut findings: Vec<Issue> = check_translation(
                    self.knowledge,
                    self.target_language,
                    &entry.source,
                    &entry.translation,
                    &entry.extracted,
                    &entry.term_exceptions,
                )
                .issues
                .into_iter()
                .filter(|issue| issue.is_problem() == problems)
                .collect();
                if let Some(group) = &query.issue {
                    if !findings.iter().any(|issue| issue.group() == *group) {
                        return None;
                    }
                    findings.sort_by_key(|issue| issue.group() != *group);
                }
                if findings.is_empty() {
                    return None;
                }
                findings
            }
        };
        Some(Hit {
            path: path.to_owned(),
            context: entry.context.clone(),
            source: entry.source.clone(),
            translation: entry.translation.clone(),
            fuzzy: entry.fuzzy,
            note,
            matches,
            findings,
        })
    }

    fn file(&self, root: &Path, path: &str) -> Result<Vec<Hit>, SearchError> {
        let full = root.join(PO_DIR).join(path);
        let text = std::fs::read_to_string(&full).map_err(|error| SearchError::Read {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        if let Some(matcher) = &self.matcher
            && matcher.prefilter
            && !matcher.regex.is_match(&text)
        {
            return Ok(Vec::new());
        }
        let file = PoFile::parse(&text).0;
        Ok(file
            .entries
            .iter()
            .filter_map(|entry| self.hit(path, entry))
            .collect())
    }
}

/// Searches the project's files, several at a time. Hits are in file order
/// (files sorted by path), then entry order. `cancel` stops the search
/// between files.
///
/// # Errors
///
/// Returns an error for an invalid pattern, or when `po/` or a file cannot
/// be read.
pub fn search(
    root: &Path,
    query: &Query,
    knowledge: &Knowledge,
    target_language: &str,
    cancel: &AtomicBool,
) -> Result<Found, SearchError> {
    let results = search_files(root, query, knowledge, target_language, cancel)?;
    Ok(summarize(results, cancel.load(Ordering::Relaxed)))
}

/// Every entry a search finds, in file order, without the limit of
/// [`MAX_HITS`]: what a bulk action on the whole result acts on. It cannot
/// be cancelled, so it never returns part of the result.
///
/// # Errors
///
/// As [`search`].
pub fn search_all(
    root: &Path,
    query: &Query,
    knowledge: &Knowledge,
    target_language: &str,
) -> Result<Vec<Hit>, SearchError> {
    let results = search_files(
        root,
        query,
        knowledge,
        target_language,
        &AtomicBool::new(false),
    )?;
    Ok(results.into_iter().flat_map(|(_, hits)| hits).collect())
}

/// The hits of every file with one, by the file's index in path order.
fn search_files(
    root: &Path,
    query: &Query,
    knowledge: &Knowledge,
    target_language: &str,
    cancel: &AtomicBool,
) -> Result<Vec<(usize, Vec<Hit>)>, SearchError> {
    let matcher = query.pattern.as_ref().map(Matcher::new).transpose()?;
    let search = Search {
        query,
        matcher,
        knowledge,
        target_language,
    };
    let mut paths = list(root).map_err(|error| SearchError::Read {
        path: String::new(),
        message: error.to_string(),
    })?;
    paths.retain(|path| path_selected(path, &query.paths));
    paths.sort();

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Vec<Hit>)>> = Mutex::new(Vec::new());
    let failure: Mutex<Option<SearchError>> = Mutex::new(None);
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(paths.len().max(1)) {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(index) else { return };
                    match search.file(root, path) {
                        Ok(hits) if hits.is_empty() => {}
                        Ok(hits) => results
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push((index, hits)),
                        Err(error) => {
                            let mut failure = failure
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            failure.get_or_insert(error);
                            cancel.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = failure
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        return Err(error);
    }
    let mut results = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    results.sort_by_key(|(index, _)| *index);
    Ok(results)
}

/// The result of a search from the hits of each file, in file order.
fn summarize(results: Vec<(usize, Vec<Hit>)>, cancelled: bool) -> Found {
    let mut found = Found {
        cancelled,
        ..Found::default()
    };
    let mut issues: std::collections::HashMap<String, IssueCount> =
        std::collections::HashMap::new();
    for (_, hits) in results {
        let Some(first) = hits.first() else { continue };
        found.files.push(FileHits {
            path: first.path.clone(),
            sheet: crate::identity::Identity::parse(&first.context)
                .map(|identity| identity.sheet)
                .unwrap_or_default(),
            count: hits.len(),
        });
        for hit in hits {
            found.total += 1;
            found.matches += hit
                .matches
                .iter()
                .map(|field| field.ranges.len())
                .sum::<usize>();
            let mut seen = std::collections::HashSet::new();
            for issue in &hit.findings {
                let group = issue.group();
                if seen.insert(group.clone()) {
                    issues
                        .entry(group.clone())
                        .or_insert_with(|| IssueCount {
                            group,
                            issue: issue.clone(),
                            count: 0,
                        })
                        .count += 1;
                }
            }
            if found.hits.len() < MAX_HITS {
                found.hits.push(hit);
            }
        }
    }
    found.issues = issues.into_values().collect();
    found
        .issues
        .sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.group.cmp(&b.group)));
    found
}

/// How found text is replaced.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Replacement {
    /// The new text. With a regular expression, `$1` and `${name}` insert
    /// groups; `$$` is a `$`.
    pub text: String,
    /// Keep the case of what is replaced: all capitals, or a capital first
    /// letter.
    pub preserve_case: bool,
}

/// Replaces the matches in the text of `macro_text` and returns the new
/// macro text, or `None` when nothing matched. Inserted text is escaped as
/// macro text, so it never becomes a macro.
#[must_use]
pub fn replace_in_text(
    matcher: &Matcher,
    kind: MatchKind,
    macro_text: &str,
    replacement: &Replacement,
) -> Option<String> {
    let mut out = String::with_capacity(macro_text.len());
    let mut copied = 0;
    let mut changed = false;
    for range in text_ranges(macro_text) {
        let text = &macro_text[range.clone()];
        out.push_str(&macro_text[copied..range.start]);
        let mut last = 0;
        for captures in matcher.regex.captures_iter(text) {
            let Some(whole) = captures.get(0).filter(|whole| !whole.is_empty()) else {
                continue;
            };
            out.push_str(&text[last..whole.start()]);
            let mut new = expand(kind, &captures, &replacement.text);
            if replacement.preserve_case {
                new = match_case(whole.as_str(), &new);
            }
            escape_into(&mut out, &new);
            last = whole.end();
            changed = true;
        }
        out.push_str(&text[last..]);
        copied = range.end;
    }
    out.push_str(&macro_text[copied..]);
    changed.then_some(out)
}

fn expand(kind: MatchKind, captures: &Captures<'_>, template: &str) -> String {
    if kind == MatchKind::Regex {
        let mut expanded = String::new();
        captures.expand(template, &mut expanded);
        expanded
    } else {
        template.to_owned()
    }
}

/// `new` in the case of `found`: all capitals when `found` has more than one
/// letter and all are capitals, a capital first letter when `found` starts
/// with one, and as written otherwise.
fn match_case(found: &str, new: &str) -> String {
    let letters: Vec<char> = found.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return new.to_uppercase();
    }
    if letters.first().is_some_and(|c| c.is_uppercase()) {
        let mut chars = new.chars();
        return chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        });
    }
    if letters.first().is_some_and(|c| c.is_lowercase()) {
        let mut chars = new.chars();
        return chars.next().map_or_else(String::new, |first| {
            first.to_lowercase().chain(chars).collect()
        });
    }
    new.to_owned()
}

/// Appends text as macro text: `\`, `<`, and `{` are escaped.
fn escape_into(out: &mut String, text: &str) {
    for character in text.chars() {
        if matches!(character, '\\' | '<' | '{') {
            out.push('\\');
        }
        out.push(character);
    }
}

/// A translation a replacement changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub source: String,
    pub before: String,
    pub after: String,
    pub fuzzy: bool,
    /// The problems of `after`: a change with problems is not written.
    pub problems: Vec<Issue>,
}

/// The changes a replacement makes to the translations `query` finds; the
/// query's fields are ignored, since only translations are replaced.
///
/// # Errors
///
/// Returns an error for an invalid pattern, a query without one, or when a
/// file cannot be read.
pub fn preview_replace(
    root: &Path,
    query: &Query,
    replacement: &Replacement,
    knowledge: &Knowledge,
    target_language: &str,
    cancel: &AtomicBool,
) -> Result<Vec<Change>, SearchError> {
    let pattern = query.pattern.as_ref().ok_or(SearchError::EmptyPattern)?;
    let matcher = Matcher::new(pattern)?;
    let translations_only = Query {
        fields: Fields {
            translation: true,
            source: false,
            note: false,
            context: false,
        },
        ..query.clone()
    };
    let mut paths = list(root).map_err(|error| SearchError::Read {
        path: String::new(),
        message: error.to_string(),
    })?;
    paths.retain(|path| path_selected(path, &translations_only.paths));
    paths.sort();
    let search = Search {
        query: &translations_only,
        matcher: Some(matcher.clone()),
        knowledge,
        target_language,
    };
    let mut changes = Vec::new();
    for path in &paths {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let full = root.join(PO_DIR).join(path);
        let text = std::fs::read_to_string(&full).map_err(|error| SearchError::Read {
            path: path.clone(),
            message: error.to_string(),
        })?;
        if matcher.prefilter && !matcher.regex.is_match(&text) {
            continue;
        }
        for entry in &PoFile::parse(&text).0.entries {
            if search.hit(path, entry).is_none() {
                continue;
            }
            let Some(after) =
                replace_in_text(&matcher, pattern.kind, &entry.translation, replacement)
            else {
                continue;
            };
            if after == entry.translation {
                continue;
            }
            let problems = if after.is_empty() {
                Vec::new()
            } else {
                check_translation(
                    knowledge,
                    target_language,
                    &entry.source,
                    &after,
                    &entry.extracted,
                    &entry.term_exceptions,
                )
                .issues
                .into_iter()
                .filter(Issue::is_problem)
                .collect()
            };
            changes.push(Change {
                path: path.clone(),
                context: entry.context.clone(),
                source: entry.source.clone(),
                before: entry.translation.clone(),
                after,
                fuzzy: entry.fuzzy,
                problems,
            });
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(text: &str, kind: MatchKind, case_sensitive: bool) -> Matcher {
        Matcher::new(&Pattern {
            text: text.to_owned(),
            kind,
            case_sensitive,
        })
        .expect("pattern")
    }

    fn replace(
        text: &str,
        kind: MatchKind,
        found_in: &str,
        with: &str,
        preserve_case: bool,
    ) -> Option<String> {
        replace_in_text(
            &matcher(text, kind, false),
            kind,
            found_in,
            &Replacement {
                text: with.to_owned(),
                preserve_case,
            },
        )
    }

    #[test]
    fn text_is_found_outside_macros_and_inside_conditions() {
        let text = "Повар <sheet Item $n1 0> <if $gn4>готова<else>готов</if>";
        let ranges = text_ranges(text);
        let joined: Vec<&str> = ranges.iter().map(|range| &text[range.clone()]).collect();
        assert!(joined.contains(&"Повар "));
        assert!(joined.contains(&"готова"));
        assert!(joined.contains(&"готов"));
        assert!(!joined.iter().any(|part| part.contains("sheet")));
        let found = matcher("item", MatchKind::Text, false).find_in_text(text);
        assert!(found.is_empty(), "macro arguments never match");
        assert_eq!(
            matcher("ГОТОВ", MatchKind::Word, false)
                .find_in_text(text)
                .len(),
            1
        );
        assert_eq!(
            matcher("готов", MatchKind::Text, false)
                .find_in_text(text)
                .len(),
            2
        );
    }

    #[test]
    fn replacement_keeps_macros_and_case_and_escapes_text() {
        assert_eq!(
            replace(
                "повар",
                MatchKind::Word,
                "Повар <sheet Item $n1 0> и повар",
                "кулинар",
                true
            )
            .as_deref(),
            Some("Кулинар <sheet Item $n1 0> и кулинар")
        );
        assert_eq!(
            replace("повар", MatchKind::Text, "ПОВАР", "кулинар", true).as_deref(),
            Some("КУЛИНАР")
        );
        assert_eq!(
            replace("Item", MatchKind::Text, "<sheet Item $n1 0>", "x", false),
            None,
            "game data is never replaced"
        );
        assert_eq!(
            replace("a", MatchKind::Text, "a", "<b>", false).as_deref(),
            Some(r"\<b>"),
            "inserted text cannot become a macro"
        );
        assert_eq!(
            replace(
                r"(заклинател)(\w*)",
                MatchKind::Regex,
                "Заклинателя нет",
                "чароде$2",
                false
            )
            .as_deref(),
            Some("чародея нет")
        );
    }

    #[test]
    fn paths_choose_files_and_folders() {
        let paths = vec!["quest/".to_owned(), "Addon".to_owned()];
        assert!(path_selected("quest/000/A.po", &paths));
        assert!(path_selected("Addon/2000.po", &paths));
        assert!(path_selected("Addon.po", &paths));
        assert!(!path_selected("AddonTransient.po", &paths));
        assert!(path_selected("Item.po", &[]));
    }

    #[test]
    fn an_invalid_regular_expression_is_reported() {
        assert!(matches!(
            Matcher::new(&Pattern {
                text: "(".to_owned(),
                kind: MatchKind::Regex,
                case_sensitive: false
            }),
            Err(SearchError::Pattern(_))
        ));
    }
}
