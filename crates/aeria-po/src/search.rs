//! Search and replace over the files of `po/`.
//!
//! A search reads the project's files, which are the only state, so it
//! always sees the current translations. Translations and sources match only
//! in their text: the content of macros (`<sheet Item $n1 0>`) never
//! matches, while text inside conditions (`<if $gn4>готова<else>готов</if>`)
//! does. A replacement changes only that text, so it never touches game data.
//! See `docs/architecture/search.md`.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use aeria_knowledge::Knowledge;
use aeria_se::{ExprKind, SyntaxKind, SyntaxNode};
use regex::{Captures, Regex, RegexBuilder};

use crate::check::{Issue, check_translation};
use crate::po::PoFile;
use crate::project::{PO_DIR, walk};

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
    /// Text or whole words, not a regular expression.
    literal: bool,
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
            literal: pattern.kind != MatchKind::Regex,
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

    /// Whether `text` has a match, as [`Matcher::find`] finds them.
    fn has(&self, text: &str) -> bool {
        if self.literal {
            // The text of a literal is never empty, nor are its matches.
            self.regex.is_match(text)
        } else {
            self.regex.find_iter(text).any(|found| !found.is_empty())
        }
    }

    /// The matches in the text of macro text: in its text ranges only.
    fn find_in_text(&self, text: Text<'_>) -> Vec<Range<usize>> {
        if !self.may_be_in_text(text.text) {
            return Vec::new();
        }
        text.ranges()
            .flat_map(|range| {
                self.find(&text.text[range.clone()])
                    .into_iter()
                    .map(move |found| found.start + range.start..found.end + range.start)
            })
            .collect()
    }

    /// Whether the text of macro text has a match, as
    /// [`Matcher::find_in_text`] finds them.
    fn is_in_text(&self, text: Text<'_>) -> bool {
        self.may_be_in_text(text.text) && text.ranges().any(|range| self.has(&text.text[range]))
    }

    /// False when the text of macro text certainly has no match. A text or
    /// word found in a text range is found in the whole text too, since a
    /// range ends at a macro's `<` or `>` or a quote: a whole text without
    /// one has none, and its macros need not be parsed.
    fn may_be_in_text(&self, macro_text: &str) -> bool {
        !self.literal || self.regex.is_match(macro_text)
    }
}

/// Whether macro text may be more than one text node: without a backslash,
/// `<`, or `{` it is one, as most of the game's strings are, and need not be
/// parsed.
fn has_syntax(macro_text: &str) -> bool {
    macro_text.contains(['\\', '<', '{'])
}

/// The byte ranges of macro text that are text a translator writes: its text
/// nodes, and those of translatable macro arguments such as condition
/// branches. Malformed text is one range.
#[must_use]
pub fn text_ranges(macro_text: &str) -> Vec<Range<usize>> {
    if !has_syntax(macro_text) {
        return if macro_text.is_empty() {
            Vec::new()
        } else {
            std::iter::once(0..macro_text.len()).collect()
        };
    }
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

/// The size and modification time of a file: its text is read again when
/// they change.
type Stamp = Option<(u64, SystemTime)>;

fn stamp_of(entry: &std::fs::DirEntry) -> Stamp {
    // The listing has the metadata of a file, but not of what a link points to.
    let metadata = match entry.file_type() {
        Ok(kind) if !kind.is_symlink() => entry.metadata().ok()?,
        _ => std::fs::metadata(entry.path()).ok()?,
    };
    Some((metadata.len(), metadata.modified().ok()?))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Byte offsets of a text in [`FileText::text`].
#[derive(Clone, Copy, Debug, Default)]
struct Span(u32, u32);

/// One entry of a file, as a search reads it.
#[derive(Clone, Copy, Debug)]
struct TextEntry {
    context: Span,
    source: Span,
    /// The text ranges of the source in [`FileText::ranges`], when it has
    /// macros or escapes; otherwise the whole source is one.
    source_ranges: Option<Span>,
    translation: Span,
    translation_ranges: Option<Span>,
    /// The translator's notes, a line each; empty without notes.
    note: Span,
    fuzzy: bool,
}

/// The entries of one file with their texts in one buffer: a project of the
/// whole game has close to a million.
struct FileText {
    stamp: Stamp,
    sheet: String,
    text: String,
    /// Text ranges of sources and translations with macros, from the start
    /// of their text: macros are parsed once, not by every search.
    ranges: Vec<Span>,
    entries: Vec<TextEntry>,
    /// The issues of the checks, when they were looked for.
    checked: Mutex<Option<Arc<Checked>>>,
}

/// The issues of the checks of a file's translated entries that have any,
/// by the index of the entry, for the knowledge they were found with.
struct Checked {
    knowledge: Arc<Knowledge>,
    target_language: String,
    issues: Vec<(u32, Vec<Issue>)>,
}

/// An entry's texts.
struct View<'a> {
    context: &'a str,
    source: Text<'a>,
    translation: Text<'a>,
    note: &'a str,
    fuzzy: bool,
}

impl View<'_> {
    fn state(&self) -> State {
        if self.fuzzy {
            State::Fuzzy
        } else if self.translation.text.is_empty() {
            State::Untranslated
        } else {
            State::Translated
        }
    }
}

/// A source or translation with its text ranges (see [`text_ranges`]).
#[derive(Clone, Copy)]
struct Text<'a> {
    text: &'a str,
    /// `None` when the whole text is one range.
    ranges: Option<&'a [Span]>,
}

impl Text<'_> {
    fn ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        let whole = (self.ranges.is_none() && !self.text.is_empty()).then_some(0..self.text.len());
        whole.into_iter().chain(
            self.ranges
                .unwrap_or_default()
                .iter()
                .map(|span| span.0 as usize..span.1 as usize),
        )
    }
}

impl FileText {
    /// The text of a parsed file; `None` for a file too large for its
    /// offsets, more than 4 GB.
    fn of(stamp: Stamp, file: &PoFile) -> Option<Self> {
        let size = file
            .entries
            .iter()
            .map(|entry| {
                entry.context.len()
                    + entry.source.len()
                    + entry.translation.len()
                    + entry.notes.iter().map(|note| note.len() + 1).sum::<usize>()
            })
            .sum();
        let mut text = String::with_capacity(size);
        let mut ranges = Vec::new();
        let mut entries = Vec::with_capacity(file.entries.len());
        let offset = |text: &String| u32::try_from(text.len()).ok();
        for entry in &file.entries {
            let mut push = |part: &str| {
                let start = offset(&text)?;
                text.push_str(part);
                Some(Span(start, offset(&text)?))
            };
            let context = push(&entry.context)?;
            let source = push(&entry.source)?;
            let translation = push(&entry.translation)?;
            let note = push(&entry.notes.join("\n"))?;
            let mut ranges_of = |macro_text: &str| -> Option<Option<Span>> {
                if !has_syntax(macro_text) {
                    return Some(None);
                }
                let start = u32::try_from(ranges.len()).ok()?;
                for range in text_ranges(macro_text) {
                    ranges.push(Span(
                        u32::try_from(range.start).ok()?,
                        u32::try_from(range.end).ok()?,
                    ));
                }
                Some(Some(Span(start, u32::try_from(ranges.len()).ok()?)))
            };
            entries.push(TextEntry {
                context,
                source,
                source_ranges: ranges_of(&entry.source)?,
                translation,
                translation_ranges: ranges_of(&entry.translation)?,
                note,
                fuzzy: entry.fuzzy,
            });
        }
        Some(Self {
            stamp,
            sheet: file
                .entries
                .first()
                .and_then(|entry| crate::identity::Identity::parse(&entry.context).ok())
                .map(|identity| identity.sheet)
                .unwrap_or_default(),
            text,
            ranges,
            entries,
            checked: Mutex::new(None),
        })
    }

    fn get(&self, span: Span) -> &str {
        &self.text[span.0 as usize..span.1 as usize]
    }

    fn text(&self, span: Span, ranges: Option<Span>) -> Text<'_> {
        Text {
            text: self.get(span),
            ranges: ranges.map(|ranges| &self.ranges[ranges.0 as usize..ranges.1 as usize]),
        }
    }

    fn view(&self, entry: &TextEntry) -> View<'_> {
        View {
            context: self.get(entry.context),
            source: self.text(entry.source, entry.source_ranges),
            translation: self.text(entry.translation, entry.translation_ranges),
            note: self.get(entry.note),
            fuzzy: entry.fuzzy,
        }
    }

    /// The issues of the checks, when they were found with `knowledge`.
    fn checked(&self, knowledge: &Arc<Knowledge>, target_language: &str) -> Option<Arc<Checked>> {
        lock(&self.checked)
            .as_ref()
            .filter(|checked| {
                Arc::ptr_eq(&checked.knowledge, knowledge)
                    && checked.target_language == target_language
            })
            .cloned()
    }
}

impl Checked {
    /// The issues of an entry.
    fn of(&self, index: usize) -> &[Issue] {
        u32::try_from(index)
            .ok()
            .and_then(|index| self.issues.binary_search_by_key(&index, |(at, _)| *at).ok())
            .map_or(&[], |at| self.issues[at].1.as_slice())
    }
}

/// A file's text with the issues of its checks.
type CheckedFile = (String, Arc<FileText>, Arc<Checked>);

/// What a search keeps of an entry.
struct Search<'a> {
    query: &'a Query,
    matcher: Option<Matcher>,
}

impl Search<'_> {
    /// Whether the query keeps an entry by its state, `msgctxt`, and pattern.
    fn keeps(&self, entry: &View<'_>) -> bool {
        let query = self.query;
        if !query.contexts.is_empty()
            && !query
                .contexts
                .iter()
                .any(|context| context == entry.context)
        {
            return false;
        }
        if !query.states.is_empty() && !query.states.contains(&entry.state()) {
            return false;
        }
        let Some(matcher) = &self.matcher else {
            return true;
        };
        let fields = query.fields;
        (fields.translation && matcher.is_in_text(entry.translation))
            || (fields.source && matcher.is_in_text(entry.source))
            || (fields.note && matcher.has(entry.note))
            || (fields.context && matcher.has(entry.context))
    }

    /// The matches of the pattern in an entry the query keeps.
    fn matches(&self, entry: &View<'_>) -> Vec<FieldMatch> {
        let Some(matcher) = &self.matcher else {
            return Vec::new();
        };
        let fields = self.query.fields;
        let mut found = Vec::new();
        let mut add = |chosen: bool, field, find: &dyn Fn() -> Vec<Range<usize>>| {
            if chosen {
                let ranges = find();
                if !ranges.is_empty() {
                    found.push(FieldMatch { field, ranges });
                }
            }
        };
        add(fields.translation, Field::Translation, &|| {
            matcher.find_in_text(entry.translation)
        });
        add(fields.source, Field::Source, &|| {
            matcher.find_in_text(entry.source)
        });
        add(fields.note, Field::Note, &|| matcher.find(entry.note));
        add(fields.context, Field::Context, &|| {
            matcher.find(entry.context)
        });
        found
    }

    /// The findings the check filter keeps of an entry's issues, or `None`
    /// when it does not keep the entry.
    fn findings(&self, entry: &View<'_>, issues: &[Issue]) -> Option<Vec<Issue>> {
        let query = self.query;
        let problems = match query.check {
            CheckFilter::Any => return Some(Vec::new()),
            CheckFilter::Problems => true,
            CheckFilter::Advice => false,
        };
        if entry.translation.text.is_empty() {
            return None;
        }
        let mut findings: Vec<Issue> = issues
            .iter()
            .filter(|issue| issue.is_problem() == problems)
            .cloned()
            .collect();
        if let Some(group) = &query.issue {
            if !findings.iter().any(|issue| issue.group() == *group) {
                return None;
            }
            findings.sort_by_key(|issue| issue.group() != *group);
        }
        (!findings.is_empty()).then_some(findings)
    }
}

/// Runs `work` for `0..count` on several threads, until `cancel` is set or
/// one fails. Returns the results by index, in order.
fn parallel<T: Send>(
    count: usize,
    cancel: &AtomicBool,
    work: impl Fn(usize) -> Result<T, SearchError> + Sync,
) -> Result<Vec<(usize, T)>, SearchError> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let results: Mutex<Vec<(usize, T)>> = Mutex::new(Vec::new());
    let failure: Mutex<Option<SearchError>> = Mutex::new(None);
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(count) {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        return;
                    }
                    match work(index) {
                        Ok(result) => lock(&results).push((index, result)),
                        Err(error) => {
                            lock(&failure).get_or_insert(error);
                            stop.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(error);
    }
    let mut results = results.into_inner().unwrap_or_else(PoisonError::into_inner);
    results.sort_by_key(|(index, _)| *index);
    Ok(results)
}

/// The entries a search keeps in one file, by index, with their findings.
struct Kept {
    path: String,
    file: Arc<FileText>,
    entries: Vec<(usize, Vec<Issue>)>,
}

impl Kept {
    fn hit(&self, search: &Search<'_>, index: usize, findings: Vec<Issue>) -> Hit {
        let entry = self.file.view(&self.file.entries[index]);
        Hit {
            path: self.path.clone(),
            context: entry.context.to_owned(),
            source: entry.source.text.to_owned(),
            translation: entry.translation.text.to_owned(),
            fuzzy: entry.fuzzy,
            note: (!entry.note.is_empty()).then(|| entry.note.to_owned()),
            matches: search.matches(&entry),
            findings,
        }
    }
}

/// The text of the project's files that searches read, kept in memory: a
/// search of the whole game then takes milliseconds instead of reading half
/// a gigabyte. Before every search the files are listed, and a file is read
/// again when its size or modification time changed, so a search always
/// sees the files as they are. The issues of the checks are kept the same
/// way, for the knowledge they were found with.
#[derive(Default)]
pub struct Corpus {
    files: Mutex<HashMap<String, Arc<FileText>>>,
    /// Held while files are read, so a search that starts during a read
    /// waits for it instead of reading the same files again.
    reading: Mutex<()>,
}

impl Corpus {
    /// Reads every file of `po/` that changed since it was last read, so the
    /// next search does not wait for it.
    ///
    /// # Errors
    ///
    /// Returns an error when `po/` or a file cannot be read.
    pub fn refresh(&self, root: &Path) -> Result<(), SearchError> {
        self.files(root, &[], &AtomicBool::new(false)).map(drop)
    }

    /// The text of the files `paths` chooses, in path order, each read again
    /// when it changed. A cancelled read leaves out the files not read.
    fn files(
        &self,
        root: &Path,
        paths: &[String],
        cancel: &AtomicBool,
    ) -> Result<Vec<(String, Arc<FileText>)>, SearchError> {
        let _reading = lock(&self.reading);
        let mut listed = Vec::new();
        walk(root, |path, entry| listed.push((path, stamp_of(entry)))).map_err(|error| {
            SearchError::Read {
                path: String::new(),
                message: error.to_string(),
            }
        })?;
        listed.sort_by(|a, b| a.0.cmp(&b.0));
        let chosen: Vec<(String, Stamp, Option<Arc<FileText>>)> = {
            let mut cached = lock(&self.files);
            cached.retain(|path, _| {
                listed
                    .binary_search_by(|(listed, _)| listed.as_str().cmp(path))
                    .is_ok()
            });
            listed
                .into_iter()
                .filter(|(path, _)| path_selected(path, paths))
                .map(|(path, stamp)| {
                    let fresh = cached
                        .get(&path)
                        .filter(|file| file.stamp == stamp)
                        .cloned();
                    (path, stamp, fresh)
                })
                .collect()
        };
        let stale: Vec<usize> = (0..chosen.len())
            .filter(|&index| chosen[index].2.is_none())
            .collect();
        let read = parallel(stale.len(), cancel, |at| {
            let (path, stamp, _) = &chosen[stale[at]];
            let text = read_file(root, path)?;
            FileText::of(*stamp, &PoFile::parse(&text).0)
                .map(Arc::new)
                .ok_or_else(|| too_large(path))
        })?;
        let mut read: HashMap<usize, Arc<FileText>> = read
            .into_iter()
            .map(|(at, file)| (stale[at], file))
            .collect();
        let mut cached = lock(&self.files);
        Ok(chosen
            .into_iter()
            .enumerate()
            .filter_map(|(index, (path, _, fresh))| {
                let file = fresh.or_else(|| {
                    let file = read.remove(&index)?;
                    cached.insert(path.clone(), Arc::clone(&file));
                    Some(file)
                })?;
                Some((path, file))
            })
            .collect())
    }

    /// The files with the issues of their checks for `knowledge`, found for
    /// those that do not have them yet. A file is read again for them, since
    /// the checks need what a search does not keep. A file whose checks were
    /// cancelled is left out.
    fn checked(
        &self,
        root: &Path,
        files: Vec<(String, Arc<FileText>)>,
        knowledge: &Arc<Knowledge>,
        target_language: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<CheckedFile>, SearchError> {
        let mut checked: Vec<Option<Arc<Checked>>> = files
            .iter()
            .map(|(_, file)| file.checked(knowledge, target_language))
            .collect();
        let unchecked: Vec<usize> = (0..files.len())
            .filter(|&index| checked[index].is_none())
            .collect();
        let read = parallel(unchecked.len(), cancel, |at| {
            let (path, known) = &files[unchecked[at]];
            // The stamp before the read: a file changed during it is read
            // again by the next search.
            let stamp = std::fs::metadata(root.join(PO_DIR).join(path))
                .ok()
                .and_then(|metadata| Some((metadata.len(), metadata.modified().ok()?)));
            let file = PoFile::parse(&read_file(root, path)?).0;
            let mut issues = Vec::new();
            for (index, entry) in file.entries.iter().enumerate() {
                if entry.translation.is_empty() {
                    continue;
                }
                let found =
                    check_translation(knowledge, target_language, entry, &entry.translation).issues;
                if !found.is_empty() {
                    issues.push((u32::try_from(index).map_err(|_| too_large(path))?, found));
                }
            }
            let found = Arc::new(Checked {
                knowledge: Arc::clone(knowledge),
                target_language: target_language.to_owned(),
                issues,
            });
            // The issues belong to the text read with them: a file that
            // changed since its text was read takes the new text.
            let text = if stamp == known.stamp {
                None
            } else {
                Some(Arc::new(
                    FileText::of(stamp, &file).ok_or_else(|| too_large(path))?,
                ))
            };
            *lock(&text.as_ref().unwrap_or(known).checked) = Some(Arc::clone(&found));
            Ok((text, found))
        })?;
        let mut cached = lock(&self.files);
        let mut files: Vec<Option<(String, Arc<FileText>)>> = files.into_iter().map(Some).collect();
        for (at, (text, found)) in read {
            let index = unchecked[at];
            checked[index] = Some(found);
            if let (Some(text), Some((path, file))) = (text, &mut files[index]) {
                cached.insert(path.clone(), Arc::clone(&text));
                *file = text;
            }
        }
        Ok(files
            .into_iter()
            .zip(checked)
            .filter_map(|(file, checked)| {
                let (path, file) = file?;
                Some((path, file, checked?))
            })
            .collect())
    }

    /// The entries `search` keeps in each file that has any, in file order.
    fn find(
        &self,
        root: &Path,
        search: &Search<'_>,
        knowledge: &Arc<Knowledge>,
        target_language: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<Kept>, SearchError> {
        let files = self.files(root, &search.query.paths, cancel)?;
        let files: Vec<(String, Arc<FileText>, Option<Arc<Checked>>)> =
            if search.query.check == CheckFilter::Any {
                files
                    .into_iter()
                    .map(|(path, file)| (path, file, None))
                    .collect()
            } else {
                self.checked(root, files, knowledge, target_language, cancel)?
                    .into_iter()
                    .map(|(path, file, checked)| (path, file, Some(checked)))
                    .collect()
            };
        let kept = parallel(files.len(), cancel, |index| {
            let (path, file, checked) = &files[index];
            let entries: Vec<(usize, Vec<Issue>)> = file
                .entries
                .iter()
                .enumerate()
                .filter_map(|(at, entry)| {
                    let entry = file.view(entry);
                    if !search.keeps(&entry) {
                        return None;
                    }
                    let issues = checked.as_ref().map_or(&[][..], |checked| checked.of(at));
                    Some((at, search.findings(&entry, issues)?))
                })
                .collect();
            Ok((!entries.is_empty()).then(|| Kept {
                path: path.clone(),
                file: Arc::clone(file),
                entries,
            }))
        })?;
        Ok(kept.into_iter().filter_map(|(_, kept)| kept).collect())
    }

    /// Searches the project's files. Hits are in file order (files sorted by
    /// path), then entry order. `cancel` stops the search between files.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid pattern, or when `po/` or a file
    /// cannot be read.
    pub fn search(
        &self,
        root: &Path,
        query: &Query,
        knowledge: &Arc<Knowledge>,
        target_language: &str,
        cancel: &AtomicBool,
    ) -> Result<Found, SearchError> {
        let search = Search {
            query,
            matcher: query.pattern.as_ref().map(Matcher::new).transpose()?,
        };
        let kept = self.find(root, &search, knowledge, target_language, cancel)?;
        let mut found = Found {
            cancelled: cancel.load(Ordering::Relaxed),
            ..Found::default()
        };
        let mut issues: HashMap<String, IssueCount> = HashMap::new();
        for file in kept {
            found.files.push(FileHits {
                path: file.path.clone(),
                sheet: file.file.sheet.clone(),
                count: file.entries.len(),
            });
            for (index, findings) in &file.entries {
                found.total += 1;
                let mut seen = std::collections::HashSet::new();
                for issue in findings {
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
                    found.hits.push(file.hit(&search, *index, findings.clone()));
                }
            }
        }
        found.issues = issues.into_values().collect();
        found
            .issues
            .sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.group.cmp(&b.group)));
        Ok(found)
    }

    /// Every entry a search finds, in file order, without the limit of
    /// [`MAX_HITS`]: what a bulk action on the whole result acts on. It
    /// cannot be cancelled, so it never returns part of the result.
    ///
    /// # Errors
    ///
    /// As [`Corpus::search`].
    pub fn search_all(
        &self,
        root: &Path,
        query: &Query,
        knowledge: &Arc<Knowledge>,
        target_language: &str,
    ) -> Result<Vec<Hit>, SearchError> {
        let search = Search {
            query,
            matcher: query.pattern.as_ref().map(Matcher::new).transpose()?,
        };
        let kept = self.find(
            root,
            &search,
            knowledge,
            target_language,
            &AtomicBool::new(false),
        )?;
        let mut hits = Vec::new();
        for file in &kept {
            for (index, findings) in &file.entries {
                hits.push(file.hit(&search, *index, findings.clone()));
            }
        }
        Ok(hits)
    }
}

fn read_file(root: &Path, path: &str) -> Result<String, SearchError> {
    std::fs::read_to_string(root.join(PO_DIR).join(path)).map_err(|error| SearchError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

fn too_large(path: &str) -> SearchError {
    SearchError::Read {
        path: path.to_owned(),
        message: "the file is larger than 4 GB".to_owned(),
    }
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

impl Corpus {
    /// The changes a replacement makes to the translations `query` finds;
    /// the query's fields are ignored, since only translations are replaced.
    /// The files with a translation found are read again, so every change is
    /// made from the translation on disk.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid pattern, a query without one, or when
    /// a file cannot be read.
    pub fn preview_replace(
        &self,
        root: &Path,
        query: &Query,
        replacement: &Replacement,
        knowledge: &Arc<Knowledge>,
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
        let search = Search {
            query: &translations_only,
            matcher: Some(matcher.clone()),
        };
        let kept = self.find(root, &search, knowledge, target_language, cancel)?;
        let mut changes = Vec::new();
        for file in kept {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let found: std::collections::HashSet<&str> = file
                .entries
                .iter()
                .map(|(index, _)| file.file.get(file.file.entries[*index].context))
                .collect();
            for entry in PoFile::parse(&read_file(root, &file.path)?).0.entries {
                if !found.contains(entry.context.as_str()) {
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
                    check_translation(knowledge, target_language, &entry, &after)
                        .issues
                        .into_iter()
                        .filter(Issue::is_problem)
                        .collect()
                };
                changes.push(Change {
                    path: file.path.clone(),
                    context: entry.context,
                    source: entry.source,
                    before: entry.translation,
                    after,
                    fuzzy: entry.fuzzy,
                    problems,
                });
            }
        }
        Ok(changes)
    }
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
        let spans: Vec<Span> = ranges
            .iter()
            .map(|range| {
                Span(
                    u32::try_from(range.start).expect("short"),
                    u32::try_from(range.end).expect("short"),
                )
            })
            .collect();
        let text = Text {
            text,
            ranges: Some(&spans),
        };
        let item = matcher("item", MatchKind::Text, false);
        assert!(
            item.find_in_text(text).is_empty(),
            "macro arguments never match"
        );
        assert!(!item.is_in_text(text));
        let word = matcher("ГОТОВ", MatchKind::Word, false);
        assert_eq!(word.find_in_text(text).len(), 1);
        assert!(word.is_in_text(text));
        assert_eq!(
            matcher("готов", MatchKind::Text, false)
                .find_in_text(text)
                .len(),
            2
        );
        let inflected = matcher(r"гот\w+", MatchKind::Regex, false);
        assert_eq!(inflected.find_in_text(text).len(), 2);
        assert!(!matcher(r"\d", MatchKind::Regex, false).is_in_text(text));
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
