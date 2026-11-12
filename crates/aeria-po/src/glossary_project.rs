//! One glossary identity across sheet sources and standalone terms.
//! Writes are optimistic transactions with a recoverable on-disk journal.

use crate::{Entry, PoFile, ProjectKnowledge, SheetTerm, Variant, fingerprint, list};
use aeria_knowledge::catalog::{
    CATALOG_PATH, CATALOG_VERSION, Catalog, Category, Sheet, Term, TermSource,
};
use aeria_knowledge::{Glossary, Knowledge, TermSheet};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Component, Path},
};

const JOURNAL: &str = ".aeria/glossary-transaction.json";

#[derive(Clone)]
struct SourceString {
    path: String,
    entry: Entry,
}

#[derive(Clone)]
struct Group {
    source: TermSource,
    strings: Vec<SourceString>,
}

#[derive(Debug)]
pub struct Resolved {
    revision: String,
    pub catalog: Catalog,
    pub term_ids: Vec<String>,
    pub knowledge: Knowledge,
    /// One optional source description per term, in catalog order.
    pub origins: Vec<Vec<SheetTerm>>,
    pub read: Vec<std::path::PathBuf>,
}

fn io(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn read_text(root: &Path, path: &str) -> Result<Option<String>, String> {
    match fs::read_to_string(root.join(path)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

fn group_key(source: &TermSource) -> (String, u32, String) {
    (
        source.sheet.clone(),
        source.column,
        source.source.to_lowercase(),
    )
}

fn groups(root: &Path, sheets: &[Sheet]) -> Result<(Vec<Group>, Vec<std::path::PathBuf>), String> {
    if sheets.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let declarations: HashSet<_> = sheets
        .iter()
        .map(|s| (s.sheet.as_str(), s.column))
        .collect();
    let mut found: HashMap<(String, u32, String), Group> = HashMap::new();
    let mut order = Vec::new();
    let mut read = Vec::new();
    // File order is deterministic; sheet declaration order is applied below.
    for path in list(root).map_err(io)? {
        if !sheets
            .iter()
            .any(|s| crate::sheet_terms::is_file_of(&path, &s.sheet))
        {
            continue;
        }
        let full = root.join("po").join(&path);
        read.push(full.parent().expect("PO parent").to_owned());
        read.push(full.clone());
        let text = fs::read_to_string(&full).map_err(io)?;
        let (file, problems) = PoFile::parse(&text);
        if let Some(problem) = problems.first() {
            return Err(format!("{path}:{}: {}", problem.line, problem.message));
        }
        for entry in file.entries {
            let Ok(identity) = crate::Identity::parse(&entry.context) else {
                continue;
            };
            let source = entry.source.trim();
            if !declarations.contains(&(identity.sheet.as_str(), identity.column))
                || source.contains(['<', '\n'])
                || !source.chars().any(char::is_alphabetic)
                || entry
                    .term_exceptions
                    .iter()
                    .any(|e| e.to_lowercase() == source.to_lowercase())
            {
                continue;
            }
            let reference = TermSource {
                sheet: identity.sheet,
                column: identity.column,
                source: source.to_owned(),
            };
            let key = group_key(&reference);
            if !found.contains_key(&key) {
                order.push(key.clone());
            }
            found
                .entry(key)
                .or_insert_with(|| Group {
                    source: reference,
                    strings: Vec::new(),
                })
                .strings
                .push(SourceString {
                    path: path.clone(),
                    entry,
                });
        }
    }
    let mut result = Vec::new();
    for sheet in sheets {
        for key in &order {
            if key.0 == sheet.sheet
                && key.1 == sheet.column
                && let Some(group) = found.remove(key)
            {
                result.push(group);
            }
        }
    }
    Ok((result, read))
}

fn eligible(string: &SourceString) -> bool {
    !string.entry.fuzzy && !string.entry.translation.trim().is_empty()
}

fn tally<'a>(strings: impl Iterator<Item = &'a SourceString>) -> Vec<(String, usize)> {
    let mut result: Vec<(String, usize)> = Vec::new();
    for string in strings.filter(|s| eligible(s)) {
        let translation = string.entry.translation.trim();
        if let Some((_, n)) = result.iter_mut().find(|(t, _)| t == translation) {
            *n += 1;
        } else {
            result.push((translation.to_owned(), 1));
        }
    }
    result.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    result
}

fn discover_terms(catalog: &mut Catalog, groups: &[Group]) -> Result<(), String> {
    let mut assigned: HashSet<_> = catalog
        .terms
        .iter()
        .flat_map(|t| t.sources.iter().map(group_key))
        .collect();
    // New game names take only the sheet's default category. Existing terms
    // retain their independent category and settings.
    for group in groups {
        if assigned.contains(&group_key(&group.source)) {
            continue;
        }
        let Some(first) = group.strings.iter().find(|s| eligible(s)) else {
            continue;
        };
        let sheet = catalog
            .sheets
            .iter()
            .find(|s| s.sheet == group.source.sheet && s.column == group.source.column)
            .expect("declared sheet");
        let category = sheet.category.clone();
        if let Some(term) = catalog.terms.iter_mut().find(|t| {
            t.category == category
                && std::iter::once(&t.term)
                    .chain(&t.forms)
                    .any(|n| n.to_lowercase() == group.source.source.to_lowercase())
        }) {
            if term.sources.is_empty() {
                if term.translation.as_deref()
                    != tally(group.strings.iter()).first().map(|v| v.0.as_str())
                {
                    return Err(format!(
                        "new sheet name {:?} conflicts with standalone term {:?}; choose one translation before linking this source",
                        group.source.source, term.term
                    ));
                }
                term.translation = None;
            }
            term.sources.push(group.source.clone());
        } else {
            let note: Vec<String> = group
                .strings
                .iter()
                .filter(|s| eligible(s))
                .flat_map(|s| s.entry.notes.iter().cloned())
                .collect();
            catalog.terms.push(Term {
                id: format!(
                    "t-{}",
                    fingerprint(&format!(
                        "{}\0{category}",
                        group.source.source.to_lowercase()
                    ))
                ),
                term: first.entry.source.trim().to_owned(),
                category: category.clone(),
                forms: group
                    .strings
                    .iter()
                    .filter(|s| eligible(s))
                    .map(|s| s.entry.source.trim().to_owned())
                    .filter(|s| s != first.entry.source.trim())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                note: (!note.is_empty()).then(|| note.join("\n")),
                match_case: true,
                sources: vec![group.source.clone()],
                translation: None,
                version_category: category,
            });
        }
        assigned.insert(group_key(&group.source));
    }
    Ok(())
}

fn resolved(root: &Path, mut catalog: Catalog) -> Result<Resolved, String> {
    catalog.validate()?;
    let revision = current_token(root, &catalog)?;
    let (groups, mut read) = groups(root, &catalog.sheets)?;
    discover_terms(&mut catalog, &groups)?;
    catalog.validate()?;
    let by_source: HashMap<_, _> = groups.iter().map(|g| (group_key(&g.source), g)).collect();
    let mut entries = Vec::new();
    let mut origins = Vec::new();
    let mut term_ids = Vec::new();
    for term in &catalog.terms {
        let category = catalog
            .categories
            .iter()
            .find(|c| c.id == term.category)
            .expect("validated category");
        let sources: Vec<_> = term
            .sources
            .iter()
            .filter_map(|s| by_source.get(&group_key(s)).copied())
            .collect();
        // An untranslated, removed or fuzzy source does not currently name
        // a term. Keep its settings in the catalog for future game updates,
        // without inventing an empty successful translation in the glossary.
        let variants = tally(sources.iter().flat_map(|g| g.strings.iter()));
        if !term.sources.is_empty() && variants.is_empty() {
            continue;
        }
        let translation = if term.sources.is_empty() {
            term.translation.clone().expect("standalone translation")
        } else {
            variants.first().map(|v| v.0.clone()).unwrap_or_default()
        };
        entries.push(term.entry(category, translation));
        term_ids.push(term.id.clone());
        origins.push(
            sources
                .iter()
                .filter_map(|g| {
                    let variants = tally(g.strings.iter());
                    let first = g.strings.iter().find(|s| eligible(s))?;
                    Some(SheetTerm {
                        sheet: g.source.sheet.clone(),
                        context: first.entry.context.clone(),
                        term: term.term.clone(),
                        translation: variants[0].0.clone(),
                        category: category.name.clone(),
                        forms: term.forms.clone(),
                        strings: variants.iter().map(|v| v.1).sum(),
                        variants: variants
                            .into_iter()
                            .map(|(translation, strings)| Variant {
                                translation,
                                strings,
                            })
                            .collect(),
                    })
                })
                .collect(),
        );
    }
    read.push(root.join(CATALOG_PATH));
    let (style, problems) =
        match aeria_knowledge::read_file(root, aeria_knowledge::KnowledgeFile::Style) {
            Ok(style) => (style, Vec::new()),
            Err(problem) => (None, vec![problem]),
        };
    let sheets = catalog
        .sheets
        .iter()
        .map(|s| TermSheet {
            sheet: s.sheet.clone(),
            column: s.column,
            category: catalog
                .categories
                .iter()
                .find(|c| c.id == s.category)
                .expect("category")
                .name
                .clone(),
            note: None,
        })
        .collect();
    if current_token(root, &catalog)? != revision {
        return Err("glossary sources changed while loading".to_owned());
    }
    Ok(Resolved {
        revision,
        catalog,
        term_ids,
        knowledge: Knowledge {
            style,
            terms: Glossary::new(entries),
            sheets,
            problems,
        },
        origins,
        read,
    })
}

/// Read the catalog and resolve its source translations.
///
/// # Errors
/// Returns an invalid catalog, unreadable source, or recovery conflict.
pub fn load(root: &Path) -> Result<Resolved, String> {
    locked(root, || {
        recover_locked(root)?;
        if crate::read_settings(root).map_err(io)?.format != crate::FORMAT {
            return Err("the glossary catalog requires project format aeria-po/3".to_owned());
        }
        if root.join("aeria-knowledge/terms.csv").exists()
            || root.join("aeria-knowledge/term-sheets.csv").exists()
        {
            return Err(
                "legacy glossary files exist alongside glossary.json; use one authoritative format"
                    .to_owned(),
            );
        }
        let text = aeria_knowledge::read_file(root, aeria_knowledge::KnowledgeFile::Catalog)?
            .ok_or("glossary catalog is missing")?;
        resolved(root, Catalog::parse(&text)?)
    })
}

#[must_use]
pub fn knowledge(root: &Path) -> ProjectKnowledge {
    match load(root) {
        Ok(loaded) => ProjectKnowledge {
            knowledge: loaded.knowledge,
            read: loaded.read,
            ..ProjectKnowledge::default()
        },
        Err(problem) => ProjectKnowledge {
            knowledge: Knowledge {
                problems: vec![format!("{CATALOG_PATH}: {problem}")],
                ..Knowledge::default()
            },
            read: vec![root.join(CATALOG_PATH)],
            ..ProjectKnowledge::default()
        },
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    path: String,
    before: Option<String>,
    after: Option<String>,
}

fn valid_path(path: &str) -> bool {
    (path == CATALOG_PATH
        || path == crate::SETTINGS_FILE
        || path.starts_with("po/")
        || path.starts_with("aeria-knowledge/legacy-v1/")
        || path == "aeria-knowledge/terms.csv"
        || path == "aeria-knowledge/term-sheets.csv")
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !path.contains('\\')
}

fn apply(root: &Path, changes: &[Change]) -> Result<(), String> {
    // Validate every target and expected version before the first mutation.
    for change in changes {
        if !valid_path(&change.path) {
            return Err("invalid glossary transaction path".to_owned());
        }
        let current = read_text(root, &change.path)?;
        if current != change.before && current != change.after {
            return Err(format!("{} changed during glossary save", change.path));
        }
    }
    for change in changes {
        if read_text(root, &change.path)? == change.after {
            continue;
        }
        let path = root.join(&change.path);
        if let Some(text) = &change.after {
            fs::create_dir_all(path.parent().expect("parent")).map_err(io)?;
            crate::write_atomically(&path, text).map_err(io)?;
        } else if path.exists() {
            fs::remove_file(&path).map_err(io)?;
        }
    }
    Ok(())
}

/// Complete an interrupted glossary transaction.
///
/// # Errors
/// Returns an invalid journal, concurrent file change, or filesystem error.
pub fn recover(root: &Path) -> Result<(), String> {
    locked(root, || recover_locked(root))
}

fn locked<T>(root: &Path, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    fs::create_dir_all(root.join(".aeria")).map_err(io)?;
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".aeria/glossary.lock"))
        .map_err(io)?;
    lock_file.lock().map_err(io)?;
    action()
}

fn recover_locked(root: &Path) -> Result<(), String> {
    let Some(text) = read_text(root, JOURNAL)? else {
        return Ok(());
    };
    let changes: Vec<Change> = serde_json::from_str(&text).map_err(io)?;
    apply(root, &changes)?;
    fs::remove_file(root.join(JOURNAL)).map_err(io)
}

fn commit(root: &Path, changes: &[Change]) -> Result<(), String> {
    locked(root, || commit_locked(root, changes))
}

fn commit_locked(root: &Path, changes: &[Change]) -> Result<(), String> {
    recover_locked(root)?;
    for change in changes {
        if read_text(root, &change.path)? != change.before {
            return Err(format!("{} changed before glossary save", change.path));
        }
    }
    fs::create_dir_all(root.join(".aeria")).map_err(io)?;
    crate::write_atomically(
        &root.join(JOURNAL),
        &serde_json::to_string(&changes).map_err(io)?,
    )
    .map_err(io)?;
    recover_locked(root)
}

fn legacy_categories(old: &Knowledge) -> Vec<Category> {
    let mut categories: Vec<Category> = Vec::new();
    for entry in &old.terms.entries {
        if !categories
            .iter()
            .any(|c| c.name.to_lowercase() == entry.category.to_lowercase())
        {
            categories.push(Category {
                id: format!("c-{}", fingerprint(&entry.category.to_lowercase())),
                name: entry.category.clone(),
                note: String::new(),
            });
        }
    }
    for sheet in &old.sheets {
        if !categories
            .iter()
            .any(|c| c.name.to_lowercase() == sheet.category.to_lowercase())
        {
            categories.push(Category {
                id: format!("c-{}", fingerprint(&sheet.category.to_lowercase())),
                name: sheet.category.clone(),
                note: String::new(),
            });
        }
        let category = categories
            .iter_mut()
            .find(|c| c.name.to_lowercase() == sheet.category.to_lowercase())
            .expect("category");
        if let Some(note) = &sheet.note
            && !category.note.lines().any(|line| line == note)
        {
            if !category.note.is_empty() {
                category.note.push('\n');
            }
            category.note.push_str(note);
        }
    }
    categories
}

fn legacy_category_id(categories: &[Category], name: &str) -> String {
    categories
        .iter()
        .find(|c| c.name.to_lowercase() == name.to_lowercase())
        .expect("collected category")
        .id
        .clone()
}

fn legacy_terms(
    old: &Knowledge,
    categories: &[Category],
    sheets: &[Sheet],
    groups: &[Group],
) -> Result<Vec<Term>, String> {
    let category_id = |name: &str| legacy_category_id(categories, name);
    let mut terms: Vec<Term> = old
        .terms
        .entries
        .iter()
        .map(|e| {
            let category = category_id(&e.category);
            Term {
                id: format!(
                    "t-{}",
                    fingerprint(&format!("{}\0{category}", e.term.to_lowercase()))
                ),
                term: e.term.clone(),
                category,
                forms: e.forms.clone(),
                note: e.note.clone(),
                match_case: e.match_case,
                sources: Vec::new(),
                translation: Some(e.translation.clone()),
                version_category: e.category.clone(),
            }
        })
        .collect();
    let mut holders = HashMap::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, term) in terms.iter().enumerate() {
        for form in std::iter::once(&term.term).chain(&term.forms) {
            holders.insert((term.category.clone(), form.to_lowercase()), index);
        }
        by_name
            .entry(term.term.to_lowercase())
            .or_default()
            .push(index);
    }
    for group in groups {
        let category = sheets
            .iter()
            .find(|s| s.sheet == group.source.sheet && s.column == group.source.column)
            .expect("sheet")
            .category
            .clone();
        let variants = tally(group.strings.iter());
        let translation = variants.first().map(|v| v.0.clone());
        let exact = holders
            .get(&(category, group.source.source.to_lowercase()))
            .copied();
        // Legacy sheet deduplication could combine equal translations across
        // categories. Preserve those source bindings on the same old term.
        if translation.is_none() {
            continue;
        }
        let holder = exact
            .or_else(|| {
                by_name
                    .get(&group.source.source.to_lowercase())
                    .into_iter()
                    .flatten()
                    .copied()
                    .find(|&index| {
                        terms[index].translation.as_deref().is_some_and(|s| {
                            translation
                                .as_ref()
                                .is_some_and(|v| s.to_lowercase() == v.to_lowercase())
                        })
                    })
            })
            .ok_or_else(|| format!("cannot preserve source {:?}", group.source))?;
        terms[holder].sources.push(group.source.clone());
    }
    Ok(terms)
}

type ChangedFiles = BTreeMap<String, (String, PoFile)>;

fn transfer_legacy_translations(
    root: &Path,
    target: &str,
    old: &Knowledge,
    terms: &mut [Term],
    groups: &[Group],
) -> Result<ChangedFiles, String> {
    let mut files: BTreeMap<String, (String, PoFile)> = BTreeMap::new();
    let by_source: HashMap<_, _> = groups.iter().map(|g| (group_key(&g.source), g)).collect();
    for (term, old_entry) in terms.iter_mut().zip(&old.terms.entries) {
        if term.sources.is_empty() {
            continue;
        }
        let strings: Vec<_> = term
            .sources
            .iter()
            .filter_map(|s| by_source.get(&group_key(s)))
            .flat_map(|g| g.strings.iter())
            .collect();
        // Only flatten source variants when needed to transfer an actual
        // project override. Equal existing sheet variants stay untouched.
        if tally(strings.iter().copied())[0].0 != old_entry.translation {
            for string in strings.into_iter().filter(|s| eligible(s)) {
                if string.entry.translation == old_entry.translation {
                    continue;
                }
                if !files.contains_key(&string.path) {
                    let text =
                        fs::read_to_string(root.join("po").join(&string.path)).map_err(io)?;
                    let file = PoFile::parse(&text).0;
                    files.insert(string.path.clone(), (text, file));
                }
                let entry = files
                    .get_mut(&string.path)
                    .expect("file")
                    .1
                    .entries
                    .iter_mut()
                    .find(|e| e.context == string.entry.context)
                    .ok_or("source changed during migration")?;
                if *entry != string.entry {
                    return Err("source changed during migration".to_owned());
                }
                if entry.is_reviewed() {
                    return Err(format!(
                        "migration conflict: reviewed source {} differs from the project's term",
                        entry.context
                    ));
                }
                let verdict = crate::check_translation(old, target, entry, &old_entry.translation);
                if !verdict.problems.is_empty() {
                    return Err(format!(
                        "migration refused invalid translation of {}",
                        entry.context
                    ));
                }
                entry.translation.clone_from(&old_entry.translation);
                // Keep the old tracking and review status: migration is not
                // a translation run or a new review.
            }
        }
        term.translation = None;
    }
    Ok(files)
}

fn migration_changes(root: &Path, files: ChangedFiles) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    for (path, (before, file)) in files {
        changes.push(Change {
            path: format!("aeria-knowledge/legacy-v1/po/{path}"),
            before: None,
            after: Some(before.clone()),
        });
        changes.push(Change {
            path: format!("po/{path}"),
            before: Some(before),
            after: Some(file.write()),
        });
    }
    for name in ["terms.csv", "term-sheets.csv"] {
        let path = format!("aeria-knowledge/{name}");
        if let Some(before) = read_text(root, &path)? {
            changes.push(Change {
                path: format!("aeria-knowledge/legacy-v1/{name}"),
                before: None,
                after: Some(before.clone()),
            });
            changes.push(Change {
                path,
                before: Some(before),
                after: None,
            });
        }
    }
    Ok(changes)
}

fn legacy_token(root: &Path, sheets: &[TermSheet]) -> Result<String, String> {
    let mut versions = BTreeMap::new();
    versions.insert(
        crate::SETTINGS_FILE.to_owned(),
        read_text(root, crate::SETTINGS_FILE)?.map(|text| fingerprint(&text)),
    );
    for name in ["terms.csv", "term-sheets.csv", "style.md"] {
        let path = format!("aeria-knowledge/{name}");
        versions.insert(
            path.clone(),
            read_text(root, &path)?.map(|text| fingerprint(&text)),
        );
    }
    for path in if sheets.is_empty() {
        Vec::new()
    } else {
        list(root).map_err(io)?
    } {
        if sheets
            .iter()
            .any(|s| crate::sheet_terms::is_file_of(&path, &s.sheet))
        {
            versions.insert(
                format!("po/{path}"),
                Some(fingerprint(
                    &fs::read_to_string(root.join("po").join(path)).map_err(io)?,
                )),
            );
        }
    }
    serde_json::to_string(&versions).map_err(io)
}

/// Migrate with the exact old effective order, matching rules and version
/// categories. No term record is silently discarded as a duplicate.
///
/// # Errors
/// Returns malformed legacy data, an unsafe translation transfer, a reviewed
/// source conflict, concurrent change, or a filesystem error.
pub fn migrate(root: &Path, target: &str) -> Result<Resolved, String> {
    recover(root)?;
    if root.join(CATALOG_PATH).exists() {
        return load(root);
    }
    let declarations = Knowledge::load(root).sheets;
    let expected = legacy_token(root, &declarations)?;
    let settings_text =
        read_text(root, crate::SETTINGS_FILE)?.ok_or("project settings are missing")?;
    let mut settings = crate::read_settings(root).map_err(io)?;
    if settings.target_language != target {
        return Err("migration target differs from the project language".to_owned());
    }
    let old = crate::sheet_terms::legacy_project_knowledge(root);
    if !old.knowledge.problems.is_empty() {
        return Err(old.knowledge.problems.join("\n"));
    }
    let categories = legacy_categories(&old.knowledge);
    let sheets = old
        .knowledge
        .sheets
        .iter()
        .map(|s| Sheet {
            sheet: s.sheet.clone(),
            column: s.column,
            category: legacy_category_id(&categories, &s.category),
        })
        .collect::<Vec<_>>();
    let (groups, _) = groups(root, &sheets)?;
    let mut terms = legacy_terms(&old.knowledge, &categories, &sheets, &groups)?;
    let files = transfer_legacy_translations(root, target, &old.knowledge, &mut terms, &groups)?;
    let catalog = Catalog {
        format_version: CATALOG_VERSION,
        categories,
        sheets,
        terms,
    };
    let mut changes = migration_changes(root, files)?;
    changes.insert(
        0,
        Change {
            path: "aeria-knowledge/legacy-v1/aeria.json".to_owned(),
            before: None,
            after: Some(settings_text.clone()),
        },
    );
    crate::FORMAT.clone_into(&mut settings.format);
    changes.insert(
        1,
        Change {
            path: crate::SETTINGS_FILE.to_owned(),
            before: Some(settings_text),
            after: Some(serde_json::to_string_pretty(&settings).map_err(io)? + "\n"),
        },
    );
    changes.push(Change {
        path: CATALOG_PATH.to_owned(),
        before: None,
        after: Some(catalog.text()?),
    });
    if legacy_token(root, &declarations)? != expected {
        return Err("legacy glossary changed during migration".to_owned());
    }
    commit(root, &changes)?;
    let migrated = load(root)?;
    for (before, after) in old
        .knowledge
        .terms
        .entries
        .iter()
        .zip(&migrated.knowledge.terms.entries)
    {
        if crate::name_fingerprint(&old.knowledge.terms, before)
            != crate::name_fingerprint(&migrated.knowledge.terms, after)
        {
            return Err(format!(
                "migration changed the version of {:?}",
                before.term
            ));
        }
    }
    Ok(migrated)
}

/// The immutable revision captured with the resolved catalog and sources.
/// Saves compare it with current files rather than rebinding an old view.
#[must_use]
pub fn token(loaded: &Resolved) -> String {
    loaded.revision.clone()
}

fn current_token(root: &Path, catalog: &Catalog) -> Result<String, String> {
    let mut files = BTreeMap::new();
    files.insert(
        crate::SETTINGS_FILE.to_owned(),
        fingerprint(&read_text(root, crate::SETTINGS_FILE)?.ok_or("project settings are missing")?),
    );
    files.insert(
        CATALOG_PATH.to_owned(),
        fingerprint(&read_text(root, CATALOG_PATH)?.ok_or("missing catalog")?),
    );
    for path in if catalog.sheets.is_empty() {
        Vec::new()
    } else {
        list(root).map_err(io)?
    } {
        if !catalog
            .sheets
            .iter()
            .any(|s| crate::sheet_terms::is_file_of(&path, &s.sheet))
        {
            continue;
        }
        files.insert(
            format!("po/{path}"),
            fingerprint(&fs::read_to_string(root.join("po").join(&path)).map_err(io)?),
        );
    }
    serde_json::to_string(&files).map_err(io)
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditedTerm {
    pub id: Option<String>,
    pub term: String,
    pub translation: String,
    pub forms: Vec<String>,
    pub note: Option<String>,
    pub match_case: bool,
    pub category: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermMerge {
    pub from: String,
    pub into: String,
}

fn merge_bindings(catalog: &mut Catalog, merges: &[TermMerge]) -> Result<HashSet<String>, String> {
    let destinations: HashMap<_, _> = merges
        .iter()
        .map(|m| (m.from.as_str(), m.into.as_str()))
        .collect();
    for merge in merges {
        let mut seen = HashSet::new();
        let mut current = merge.from.as_str();
        while let Some(next) = destinations.get(current) {
            if !seen.insert(current) {
                return Err("cyclic term merge".to_owned());
            }
            current = next;
        }
    }
    let mut merged_ids = HashSet::new();
    for merge in merges {
        if merge.from == merge.into || !merged_ids.insert(merge.from.clone()) {
            return Err("invalid repeated term merge".to_owned());
        }
        let source = catalog
            .terms
            .iter()
            .find(|t| t.id == merge.from)
            .cloned()
            .ok_or("unknown merged term")?;
        let destination = catalog
            .terms
            .iter_mut()
            .find(|t| t.id == merge.into)
            .ok_or("unknown destination term")?;
        if destination.term.to_lowercase() != source.term.to_lowercase() {
            return Err("only meanings of the same name can be merged".to_owned());
        }
        destination.sources.extend(source.sources);
        if !destination.sources.is_empty() {
            destination.translation = None;
        }
    }
    Ok(merged_ids)
}

fn apply_term_edit(
    edit: EditedTerm,
    catalog: &Catalog,
    old_by_id: &HashMap<String, Term>,
) -> Result<Option<Term>, String> {
    let category = catalog
        .categories
        .iter()
        .find(|c| c.name.trim().to_lowercase() == edit.category.trim().to_lowercase())
        .ok_or_else(|| format!("unknown category {:?}", edit.category))?;
    if edit.translation.trim().is_empty() {
        return Err("term translation must not be empty".to_owned());
    }
    let mut term = if let Some(id) = &edit.id {
        old_by_id
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown term ID {id}"))?
    } else {
        Term {
            id: format!(
                "t-{}",
                fingerprint(&format!(
                    "{}\0{}",
                    edit.term.trim().to_lowercase(),
                    category.id
                ))
            ),
            term: String::new(),
            category: category.id.clone(),
            forms: Vec::new(),
            note: None,
            match_case: false,
            sources: Vec::new(),
            translation: None,
            version_category: category.id.clone(),
        }
    };
    // Removing a sheet declaration removes its derived terms. It never
    // turns sheet translations into separately stored project copies.
    let backed = !term.sources.is_empty();
    term.sources.retain(|s| {
        catalog
            .sheets
            .iter()
            .any(|sheet| sheet.sheet == s.sheet && sheet.column == s.column)
    });
    if backed && term.sources.is_empty() {
        return Ok(None);
    }
    edit.term.trim().clone_into(&mut term.term);
    term.category.clone_from(&category.id);
    term.forms = edit
        .forms
        .into_iter()
        .map(|f| f.trim().to_owned())
        .filter(|f| !f.is_empty())
        .collect();
    term.note = edit
        .note
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty());
    term.match_case = edit.match_case;
    if term.sources.is_empty() {
        term.translation = Some(edit.translation.trim().to_owned());
    } else {
        term.translation = None;
    }
    Ok(Some(term))
}

fn retain_unedited_sources(
    before: &Resolved,
    catalog: &Catalog,
    terms: &mut Vec<Term>,
    old_entry_by_id: &HashMap<&str, &aeria_knowledge::GlossaryEntry>,
    merged_ids: &HashSet<String>,
) -> Result<(), String> {
    let present_ids: HashSet<_> = terms.iter().map(|t| t.id.clone()).collect();
    // A source term cannot be deleted by deleting its catalog metadata.
    // Removing its declared sheet is the explicit way to remove that scope.
    for old in &before.catalog.terms {
        if !old_entry_by_id.contains_key(old.id.as_str()) {
            let mut retained = old.clone();
            retained.sources.retain(|s| {
                catalog
                    .sheets
                    .iter()
                    .any(|sheet| sheet.sheet == s.sheet && sheet.column == s.column)
            });
            if !retained.sources.is_empty() {
                if !catalog.categories.iter().any(|c| c.id == old.category) {
                    return Err(format!(
                        "category contains retained settings for {:?}",
                        old.term
                    ));
                }
                terms.push(retained);
            }
            continue;
        }
        if !old.sources.is_empty()
            && !merged_ids.contains(&old.id)
            && !present_ids.contains(&old.id)
            && old.sources.iter().any(|s| {
                catalog
                    .sheets
                    .iter()
                    .any(|sheet| sheet.sheet == s.sheet && sheet.column == s.column)
            })
        {
            return Err(format!(
                "sheet term {:?} cannot be deleted; edit its source or remove the sheet declaration",
                old.term
            ));
        }
    }
    Ok(())
}

fn update_catalog(
    before: &Resolved,
    edits: Vec<EditedTerm>,
    categories: Option<Vec<Category>>,
    sheets: Option<Vec<TermSheet>>,
    merges: &[TermMerge],
) -> Result<(Catalog, HashMap<String, String>), String> {
    let mut catalog = before.catalog.clone();
    if let Some(categories) = categories {
        catalog.categories = categories;
    }
    let merged_ids = merge_bindings(&mut catalog, merges)?;
    if let Some(sheets) = sheets {
        catalog.sheets = sheets
            .into_iter()
            .map(|s| {
                let category = catalog
                    .categories
                    .iter()
                    .find(|c| c.name.to_lowercase() == s.category.to_lowercase())
                    .ok_or_else(|| format!("unknown category {:?}", s.category))?;
                Ok(Sheet {
                    sheet: s.sheet,
                    column: s.column,
                    category: category.id.clone(),
                })
            })
            .collect::<Result<_, String>>()?;
    }
    let old_by_id = catalog
        .terms
        .iter()
        .map(|t| (t.id.clone(), t.clone()))
        .collect();
    let old_entry_by_id: HashMap<_, _> = before
        .term_ids
        .iter()
        .zip(&before.knowledge.terms.entries)
        .map(|(id, e)| (id.as_str(), e))
        .collect();
    let mut terms = Vec::new();
    let mut translations = HashMap::new();
    for edit in edits {
        let translation = edit.translation.trim().to_owned();
        let Some(term) = apply_term_edit(edit, &catalog, &old_by_id)? else {
            continue;
        };
        if !term.sources.is_empty()
            && (merges.iter().any(|m| m.into == term.id)
                || old_entry_by_id
                    .get(term.id.as_str())
                    .is_some_and(|e| e.translation != translation))
        {
            translations.insert(term.id.clone(), translation);
        }
        terms.push(term);
    }
    retain_unedited_sources(before, &catalog, &mut terms, &old_entry_by_id, &merged_ids)?;
    let positions: HashMap<_, _> = before
        .catalog
        .terms
        .iter()
        .enumerate()
        .map(|(index, t)| (t.id.as_str(), index))
        .collect();
    terms.sort_by_key(|t| {
        positions
            .get(t.id.as_str())
            .copied()
            .unwrap_or(before.catalog.terms.len())
    });
    catalog.terms = terms;
    catalog.validate()?;
    let retained: HashSet<_> = catalog
        .terms
        .iter()
        .flat_map(|t| t.sources.iter().map(group_key))
        .collect();
    for source in before.catalog.terms.iter().flat_map(|t| &t.sources) {
        if catalog
            .sheets
            .iter()
            .any(|s| s.sheet == source.sheet && s.column == source.column)
            && !retained.contains(&group_key(source))
        {
            return Err("a term merge would discard a source binding".to_owned());
        }
    }
    for merge in merges {
        if !catalog.terms.iter().any(|t| t.id == merge.into) && !merged_ids.contains(&merge.into) {
            return Err("missing retained definition of a merge".to_owned());
        }
    }
    Ok((catalog, translations))
}

fn edit_source_files(
    root: &Path,
    target: &str,
    before: &Resolved,
    catalog: &Catalog,
    translations: &HashMap<String, String>,
) -> Result<(ChangedFiles, HashSet<String>), String> {
    let (source_groups, _) = groups(root, &catalog.sheets)?;
    let mut files: ChangedFiles = BTreeMap::new();
    let mut edited_contexts = HashSet::new();
    let by_source: HashMap<_, _> = source_groups
        .iter()
        .map(|g| (group_key(&g.source), g))
        .collect();
    for term in &catalog.terms {
        let Some(translation) = translations.get(&term.id) else {
            continue;
        };
        for string in term
            .sources
            .iter()
            .filter_map(|s| by_source.get(&group_key(s)))
            .flat_map(|g| &g.strings)
        {
            let verdict =
                crate::check_translation(&before.knowledge, target, &string.entry, translation);
            if !verdict.problems.is_empty() {
                return Err(format!("invalid translation for {}", string.entry.context));
            }
            if !files.contains_key(&string.path) {
                let text = fs::read_to_string(root.join("po").join(&string.path)).map_err(io)?;
                let file = PoFile::parse(&text).0;
                files.insert(string.path.clone(), (text, file));
            }
            let entry = files
                .get_mut(&string.path)
                .expect("file")
                .1
                .entries
                .iter_mut()
                .find(|e| e.context == string.entry.context)
                .ok_or("source changed during save")?;
            if *entry != string.entry {
                return Err("source changed during save".to_owned());
            }
            edited_contexts.insert(entry.context.clone());
            let reviewed = entry.is_reviewed();
            entry.translation.clone_from(translation);
            entry.fuzzy = false;
            entry.previous = None;
            if reviewed {
                entry.reviewed = Some(fingerprint(translation));
            }
        }
    }
    Ok((files, edited_contexts))
}

fn rebase_records(
    root: &Path,
    before: &Resolved,
    after: &Resolved,
    files: &mut ChangedFiles,
) -> Result<(), String> {
    // Collapsing equal meanings changes the legacy ordered name hash, but
    // not what a translation followed. Rebase only records with an exactly
    // known OLD fingerprint and the same set of translations and own notes.
    // Genuine term edits and already-outdated records are never acknowledged.
    let mut relocations = HashMap::new();
    let after_by_name: HashMap<_, _> = after
        .knowledge
        .terms
        .entries
        .iter()
        .map(|e| (e.term.to_lowercase(), e))
        .collect();
    for old in &before.knowledge.terms.entries {
        let Some(new) = after_by_name.get(&old.term.to_lowercase()).copied() else {
            continue;
        };
        let signatures = |glossary: &Glossary, entry: &aeria_knowledge::GlossaryEntry| {
            glossary
                .senses(entry)
                .into_iter()
                .map(|e| (e.translation.clone(), e.note.clone().unwrap_or_default()))
                .collect::<std::collections::BTreeSet<_>>()
        };
        let old_print = crate::name_fingerprint(&before.knowledge.terms, old);
        let new_print = crate::name_fingerprint(&after.knowledge.terms, new);
        if old_print != new_print
            && signatures(&before.knowledge.terms, old) == signatures(&after.knowledge.terms, new)
        {
            relocations.insert((old.term.to_lowercase(), old_print), new_print);
        }
    }
    if !relocations.is_empty() {
        for path in list(root).map_err(io)? {
            let present = files.contains_key(&path);
            let (original, mut file) = if present {
                files.remove(&path).expect("file")
            } else {
                let original = fs::read_to_string(root.join("po").join(&path)).map_err(io)?;
                let (file, problems) = PoFile::parse(&original);
                if !problems.is_empty() {
                    return Err(format!("cannot rebase term records in malformed {path}"));
                }
                (original, file)
            };
            let mut changed = present;
            for entry in file.entries.iter_mut().chain(&mut file.obsolete) {
                for (name, print) in &mut entry.written_with {
                    if let Some(new) = relocations.get(&(name.to_lowercase(), print.clone())) {
                        print.clone_from(new);
                        changed = true;
                    }
                }
            }
            if changed {
                files.insert(path, (original, file));
            }
        }
    }
    Ok(())
}

/// Save the whole glossary document, including category management and
/// source translations. Category IDs and term IDs survive renames/moves.
///
/// # Errors
/// Returns an identity collision, invalid translation, concurrent source
/// change, invalid category/merge, or a filesystem/recovery error.
pub fn save(
    root: &Path,
    target: &str,
    expected: &str,
    edits: Vec<EditedTerm>,
    categories: Option<Vec<Category>>,
    sheets: Option<Vec<TermSheet>>,
    merges: &[TermMerge],
) -> Result<Resolved, String> {
    let before = load(root)?;
    if before.revision != expected {
        return Err("glossary or its source strings changed after loading".to_owned());
    }
    let old_catalog_text = read_text(root, CATALOG_PATH)?;
    let (catalog, translations) = update_catalog(&before, edits, categories, sheets, merges)?;
    let (mut files, edited_contexts) =
        edit_source_files(root, target, &before, &catalog, &translations)?;
    // Build the resulting glossary with explicit edits overlaid, before any
    // write, so each directly edited source records the NEW term version.
    let mut after = resolved(root, catalog.clone())?;
    for (id, entry) in after
        .term_ids
        .iter()
        .zip(&mut after.knowledge.terms.entries)
    {
        if let Some(translation) = translations.get(id) {
            entry.translation.clone_from(translation);
        }
    }
    for (_, file) in files.values_mut() {
        for entry in &mut file.entries {
            if edited_contexts.contains(&entry.context) {
                entry.record_terms(&after.knowledge.terms);
            }
        }
    }
    rebase_records(root, &before, &after, &mut files)?;
    // Recheck every source version after preparation; concurrent edits are
    // refused rather than overwritten by the multi-file transaction.
    if current_token(root, &before.catalog)? != expected {
        return Err("glossary changed during save".to_owned());
    }
    let mut changes: Vec<_> = files
        .into_iter()
        .map(|(path, (before, file))| Change {
            path: format!("po/{path}"),
            before: Some(before),
            after: Some(file.write()),
        })
        .collect();
    changes.push(Change {
        path: CATALOG_PATH.to_owned(),
        before: old_catalog_text,
        after: Some(after.catalog.text()?),
    });
    commit(root, &changes)?;
    load(root)
}
