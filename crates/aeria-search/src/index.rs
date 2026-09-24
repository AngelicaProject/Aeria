//! The local full-text index of a source package's translatable strings.
//!
//! One SQLite file per source package holds every translatable String cell
//! with its macro text and plain text, and an FTS5 index over the plain text.
//! It is built from the verified HXS snapshot and HSG guidance, is never
//! project data, and can be deleted and rebuilt at any time.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use aeria_hsp::GuidanceIndex;
use aeria_hxs::{HxsError, HxsSnapshot, MAX_STRING_OCCURRENCE_PAGE_SIZE};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::text::{plain_text, similarity};

/// The index file layout version.
pub const INDEX_FORMAT: &str = "1";
/// Most hits one search returns.
pub const MAX_SEARCH_LIMIT: u32 = 200;
/// Candidates read from the full-text index before similarity ranking.
const SIMILAR_CANDIDATES: u32 = 300;
/// Least similarity for a translation-memory match.
pub const MIN_SIMILARITY: f64 = 0.5;
/// Words or trigrams of a text used to find similar strings.
const SIMILAR_TERMS: usize = 16;

/// Index errors.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search index storage failed: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("search index file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("the source could not be read: {0}")]
    Source(#[from] HxsError),
    #[error("building the search index was cancelled")]
    Cancelled,
}

/// How the index splits text into terms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tokenizer {
    /// Words, case- and diacritic-insensitive, matched by prefix.
    Words,
    /// Three-character substrings, for languages written without spaces.
    Trigram,
}

impl Tokenizer {
    /// The tokenizer for a source language tag: trigrams for Japanese,
    /// Chinese, and Korean, words otherwise.
    #[must_use]
    pub fn for_language(language: &str) -> Self {
        let primary = language.split(['-', '_']).next().unwrap_or_default();
        if ["ja", "zh", "ko"].contains(&primary.to_ascii_lowercase().as_str()) {
            Self::Trigram
        } else {
            Self::Words
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Words => "words",
            Self::Trigram => "trigram",
        }
    }

    const fn fts_option(self) -> &'static str {
        match self {
            Self::Words => "unicode61 remove_diacritics 2",
            Self::Trigram => "trigram",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "words" => Some(Self::Words),
            "trigram" => Some(Self::Trigram),
            _ => None,
        }
    }
}

/// One translatable source string found by a search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceHit {
    pub sheet: String,
    pub row: u32,
    pub subrow: u16,
    pub column: u32,
    /// The source macro text.
    pub source: String,
}

/// A page of search hits.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourcePage {
    pub hits: Vec<SourceHit>,
    /// Whether more hits follow at `offset + hits.len()`.
    pub more: bool,
}

/// A source string similar to a given one.
#[derive(Clone, Debug, PartialEq)]
pub struct SimilarSource {
    pub hit: SourceHit,
    /// Similarity of the plain texts, from [`MIN_SIMILARITY`] to 1.
    pub score: f64,
}

/// A source search.
#[derive(Clone, Copy, Debug)]
pub struct SourceQuery<'a> {
    /// Text to find in the strings' plain text.
    pub text: &'a str,
    /// Only strings of this sheet.
    pub sheet: Option<&'a str>,
    pub offset: u32,
    pub limit: u32,
}

/// A built source index.
#[derive(Clone, Debug)]
pub struct SourceIndex {
    path: PathBuf,
    tokenizer: Tokenizer,
}

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE cells (
    id INTEGER PRIMARY KEY,
    sheet TEXT NOT NULL,
    row_id INTEGER NOT NULL,
    subrow_id INTEGER NOT NULL,
    column_index INTEGER NOT NULL,
    macro TEXT NOT NULL,
    plain TEXT NOT NULL
);
";

fn read_meta(connection: &Connection, key: &str) -> Result<Option<String>, SearchError> {
    Ok(connection
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

/// Quotes a term for an FTS5 query.
fn fts_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Escapes `%`, `_`, and `\` for a `LIKE … ESCAPE '\'` pattern.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::from("%");
    for character in text.chars() {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

impl SourceIndex {
    /// Opens a complete index built for `package_id`, or returns `None` when
    /// the file is missing, incomplete, of another format, or for another
    /// package.
    ///
    /// # Errors
    ///
    /// Returns a storage error for an unreadable file.
    pub fn open(path: impl Into<PathBuf>, package_id: &str) -> Result<Option<Self>, SearchError> {
        let path = path.into();
        if !path.is_file() {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let valid = read_meta(&connection, "format")?.as_deref() == Some(INDEX_FORMAT)
            && read_meta(&connection, "package")?.as_deref() == Some(package_id);
        let tokenizer = read_meta(&connection, "tokenizer")?
            .as_deref()
            .and_then(Tokenizer::parse);
        Ok(match (valid, tokenizer) {
            (true, Some(tokenizer)) => Some(Self { path, tokenizer }),
            _ => None,
        })
    }

    /// Builds the index of every translatable, non-empty String cell and
    /// publishes it at `path` only when complete. `keep_going` is asked
    /// between pages; returning `false` cancels the build.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Cancelled`], a source read error, or a storage
    /// error. A failed build leaves no index behind.
    pub fn build(
        path: impl Into<PathBuf>,
        package_id: &str,
        tokenizer: Tokenizer,
        source: &HxsSnapshot,
        guidance: &GuidanceIndex,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Self, SearchError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = partial_path(&path);
        let _ = std::fs::remove_file(&partial);
        let result = Self::write(
            &partial, package_id, tokenizer, source, guidance, keep_going,
        )
        .and_then(|()| Ok(std::fs::rename(&partial, &path)?));
        if let Err(error) = result {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
        Ok(Self { path, tokenizer })
    }

    fn write(
        path: &Path,
        package_id: &str,
        tokenizer: Tokenizer,
        source: &HxsSnapshot,
        guidance: &GuidanceIndex,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<(), SearchError> {
        let mut connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;")?;
        connection.execute_batch(SCHEMA)?;
        connection.execute_batch(&format!(
            "CREATE VIRTUAL TABLE cells_fts USING fts5(plain, content='cells', content_rowid='id', tokenize='{}');",
            tokenizer.fts_option()
        ))?;
        let transaction = connection.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO cells (sheet, row_id, subrow_id, column_index, macro, plain) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for sheet in source.sheets() {
                if guidance.translatable_cell_count(&sheet.name) == 0 {
                    continue;
                }
                let mut after = None;
                loop {
                    if !keep_going() {
                        return Err(SearchError::Cancelled);
                    }
                    let page = source.page_string_occurrence_records(
                        &sheet.name,
                        after.as_ref(),
                        MAX_STRING_OCCURRENCE_PAGE_SIZE,
                    )?;
                    for record in &page.occurrences {
                        let at = &record.fingerprint.coordinate;
                        if record.macro_text.is_empty()
                            || !guidance.is_translatable(
                                &at.sheet_name,
                                at.row_id,
                                at.subrow_id,
                                at.column_index,
                            )
                        {
                            continue;
                        }
                        insert.execute(params![
                            at.sheet_name,
                            at.row_id,
                            at.subrow_id,
                            at.column_index,
                            record.macro_text,
                            plain_text(&record.macro_text),
                        ])?;
                    }
                    match page.next_after {
                        Some(next) => after = Some(next),
                        None => break,
                    }
                }
            }
        }
        transaction.execute_batch(
            "INSERT INTO cells_fts(cells_fts) VALUES ('rebuild');
             INSERT INTO cells_fts(cells_fts) VALUES ('optimize');
             CREATE INDEX cells_by_location ON cells (sheet, row_id, subrow_id, column_index);",
        )?;
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES ('format', ?1), ('package', ?2), ('tokenizer', ?3)",
            params![INDEX_FORMAT, package_id, tokenizer.name()],
        )?;
        transaction.commit()?;
        drop(connection);
        Ok(())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> Result<Connection, SearchError> {
        Ok(Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?)
    }

    /// The FTS5 query for a search, or `None` when the text needs a
    /// substring scan: it has no words, or fewer than three characters for
    /// trigrams.
    fn match_query(&self, text: &str) -> Option<String> {
        match self.tokenizer {
            Tokenizer::Words => {
                let terms: Vec<String> = words(text)
                    .iter()
                    .map(|word| format!("{}*", fts_term(word)))
                    .collect();
                (!terms.is_empty()).then(|| terms.join(" "))
            }
            Tokenizer::Trigram => (text.chars().count() >= 3).then(|| fts_term(text)),
        }
    }

    /// Finds translatable strings whose plain text matches the query: every
    /// word as a prefix, or the exact text for trigram indexes. The best
    /// matches come first.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn search(&self, query: &SourceQuery<'_>) -> Result<SourcePage, SearchError> {
        let text = query.text.trim();
        if text.is_empty() {
            return Ok(SourcePage::default());
        }
        let limit = query.limit.clamp(1, MAX_SEARCH_LIMIT);
        let connection = self.connect()?;
        let (sql, pattern) = self.match_query(text).map_or_else(
            || {
                (
                    "SELECT sheet, row_id, subrow_id, column_index, macro FROM cells
                     WHERE plain LIKE ?1 ESCAPE '\\' AND (?2 IS NULL OR sheet = ?2)
                     ORDER BY id LIMIT ?3 OFFSET ?4",
                    like_pattern(text),
                )
            },
            |fts| {
                (
                    "SELECT c.sheet, c.row_id, c.subrow_id, c.column_index, c.macro
                     FROM cells_fts JOIN cells c ON c.id = cells_fts.rowid
                     WHERE cells_fts MATCH ?1 AND (?2 IS NULL OR c.sheet = ?2)
                     ORDER BY bm25(cells_fts), c.id LIMIT ?3 OFFSET ?4",
                    fts,
                )
            },
        );
        let mut statement = connection.prepare(sql)?;
        let mut hits = statement
            .query_map(
                params![pattern, query.sheet, limit + 1, query.offset],
                |row| {
                    Ok(SourceHit {
                        sheet: row.get(0)?,
                        row: row.get(1)?,
                        subrow: row.get(2)?,
                        column: row.get(3)?,
                        source: row.get(4)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = hits.len() > limit as usize;
        hits.truncate(limit as usize);
        Ok(SourcePage { hits, more })
    }

    /// Terms that find candidates similar to a plain text: its longest
    /// distinct words, or evenly spread trigrams.
    fn similar_terms(&self, plain: &str) -> Vec<String> {
        match self.tokenizer {
            Tokenizer::Words => {
                let mut distinct: Vec<String> = words(plain)
                    .into_iter()
                    .filter(|word| word.chars().count() >= 2)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                distinct.sort_by(|left, right| {
                    right
                        .chars()
                        .count()
                        .cmp(&left.chars().count())
                        .then(left.cmp(right))
                });
                distinct.truncate(SIMILAR_TERMS);
                distinct
            }
            Tokenizer::Trigram => {
                let characters: Vec<char> = plain.chars().collect();
                let trigrams: Vec<String> = characters
                    .windows(3)
                    .filter(|window| !window.iter().any(|character| character.is_whitespace()))
                    .map(|window| window.iter().collect::<String>())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let step = trigrams.len().div_ceil(SIMILAR_TERMS).max(1);
                trigrams.into_iter().step_by(step).collect()
            }
        }
    }

    /// Translatable strings similar to a source string, most similar first,
    /// excluding the given location. Candidates come from the full-text index
    /// and are ranked by [`similarity`] of their plain text.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn similar(
        &self,
        source: &str,
        exclude: Option<(&str, u32, u16, u32)>,
        limit: usize,
    ) -> Result<Vec<SimilarSource>, SearchError> {
        let plain = plain_text(source);
        let terms = self.similar_terms(&plain);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let query = terms
            .iter()
            .map(|term| fts_term(term))
            .collect::<Vec<_>>()
            .join(" OR ");
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT c.sheet, c.row_id, c.subrow_id, c.column_index, c.macro, c.plain
             FROM cells_fts JOIN cells c ON c.id = cells_fts.rowid
             WHERE cells_fts MATCH ?1 ORDER BY bm25(cells_fts), c.id LIMIT ?2",
        )?;
        let mut similar = statement
            .query_map(params![query, SIMILAR_CANDIDATES], |row| {
                Ok((
                    SourceHit {
                        sheet: row.get(0)?,
                        row: row.get(1)?,
                        subrow: row.get(2)?,
                        column: row.get(3)?,
                        source: row.get(4)?,
                    },
                    row.get::<_, String>(5)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter(|(hit, _)| {
                exclude.is_none_or(|(sheet, row, subrow, column)| {
                    (hit.sheet.as_str(), hit.row, hit.subrow, hit.column)
                        != (sheet, row, subrow, column)
                })
            })
            .map(|(hit, candidate)| SimilarSource {
                score: similarity(&plain, &candidate),
                hit,
            })
            .filter(|similar| similar.score >= MIN_SIMILARITY)
            .collect::<Vec<_>>();
        similar.sort_by(|left, right| {
            right.score.total_cmp(&left.score).then_with(|| {
                (
                    &left.hit.sheet,
                    left.hit.row,
                    left.hit.subrow,
                    left.hit.column,
                )
                    .cmp(&(
                        &right.hit.sheet,
                        right.hit.row,
                        right.hit.subrow,
                        right.hit.column,
                    ))
            })
        });
        similar.truncate(limit);
        Ok(similar)
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    path.with_file_name(name)
}
