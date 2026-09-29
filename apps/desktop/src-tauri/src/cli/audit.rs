//! `audit`, `review`, and `flag`: deterministic checks across the project,
//! what waits for review, and marking strings for a person.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use aeria_core::{ReviewState, TranslationUnit};
use aeria_knowledge::rules::machine_phrasing;
use aeria_knowledge::{Domain, sheet_domain};
use serde::Serialize;
use serde_json::json;

use super::Output;
use super::project::{Address, Author, Project, current, matches_pattern, review_word};

/// Prefix of the note an agent's flag adds.
const FLAG_NOTE: &str = "[agent]";
/// An interface translation longer than this many times its source may not
/// fit where the game shows it.
const LONG_RATIO: f64 = 1.8;
/// Shorter interface translations are never reported as long.
const LONG_MIN_CHARS: usize = 24;

/// The project-wide checks of `audit`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Check {
    /// The same source translated differently.
    Inconsistent,
    /// A forbidden variant of a term.
    Forbidden,
    /// A translation that breaks the source's structure.
    Structure,
    /// Both genders written at once, such as `готов(а)`.
    BothGenders,
    /// The source varies with the player character's gender and the
    /// translation does not.
    Gender,
    /// A term whose translation does not seem to be used.
    Terms,
    /// Phrasing that reads machine-written.
    Phrasing,
    /// An interface translation much longer than its source.
    Long,
}

impl Check {
    const ALL: [Self; 8] = [
        Self::Inconsistent,
        Self::Forbidden,
        Self::Structure,
        Self::BothGenders,
        Self::Gender,
        Self::Terms,
        Self::Phrasing,
        Self::Long,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Inconsistent => "inconsistent",
            Self::Forbidden => "forbidden",
            Self::Structure => "structure",
            Self::BothGenders => "both-genders",
            Self::Gender => "gender",
            Self::Terms => "terms",
            Self::Phrasing => "phrasing",
            Self::Long => "long",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Inconsistent => "the same source translated differently",
            Self::Forbidden => "a forbidden variant of a term",
            Self::Structure => "breaks the source's macros or structure",
            Self::BothGenders => "writes both genders at once",
            Self::Gender => {
                "the source varies with the player character's gender, the translation does not"
            }
            Self::Terms => "a term's translation does not seem to be used",
            Self::Phrasing => "reads machine-written",
            Self::Long => "an interface string much longer than its source",
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|check| check.name() == text.trim())
            .ok_or_else(|| {
                format!(
                    "{text:?} is not a check; the checks are {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

/// One finding.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Finding {
    address: String,
    detail: String,
    by: Author,
}

pub(crate) struct AuditOptions {
    pub pattern: Option<String>,
    pub checks: Vec<Check>,
    pub limit: usize,
}

/// A bound, translated unit of the scope with its source.
struct Translated<'a> {
    unit: &'a TranslationUnit,
    address: Address,
    source: String,
    author: Author,
}

fn address_of(unit: &TranslationUnit) -> Address {
    let binding = unit.source_binding();
    Address {
        sheet: binding.sheet_name().to_owned(),
        row: binding.row_id(),
        subrow: binding.subrow_id(),
        column: binding.column_index(),
    }
}

/// Russian forms that write both genders at once.
const BOTH_GENDERS: [&str; 6] = ["(а)", "(ла)", "(ая)", "(на)", "(ен)", "(ой)"];

fn plain_chars(text: &str) -> usize {
    aeria_se::parse(text).plain_text().chars().count()
}

#[allow(clippy::too_many_lines)] // one pass per check
pub(crate) fn audit(project: &Project, options: &AuditOptions, out: &mut Output) -> bool {
    let knowledge = project.knowledge();
    let written = project
        .ledger()
        .and_then(|ledger| ledger.all().ok())
        .unwrap_or_default();
    let target_language = project.target_language().unwrap_or_default();
    let russian = target_language.eq_ignore_ascii_case("ru");
    let wanted = |check: Check| options.checks.is_empty() || options.checks.contains(&check);
    let translated: Vec<Translated<'_>> = project
        .session
        .workspace()
        .units()
        .filter(|unit| unit.is_bound() && !unit.target_macro().trim().is_empty())
        .filter(|unit| {
            options
                .pattern
                .as_deref()
                .is_none_or(|pattern| matches_pattern(unit.source_binding().sheet_name(), pattern))
        })
        .map(|unit| {
            let address = address_of(unit);
            let author = if written.get(&address.to_string()).map(String::as_str)
                == Some(unit.target_macro())
            {
                Author::Agent
            } else {
                Author::Person
            };
            Translated {
                source: unit.source().text().to_owned(),
                unit,
                address,
                author,
            }
        })
        .collect();

    let mut findings: BTreeMap<Check, Vec<Finding>> = BTreeMap::new();
    let mut add = |check: Check, item: &Translated<'_>, detail: String| {
        findings.entry(check).or_default().push(Finding {
            address: item.address.to_string(),
            detail,
            by: item.author,
        });
    };
    if wanted(Check::Inconsistent) {
        let mut by_source: BTreeMap<&str, Vec<&Translated<'_>>> = BTreeMap::new();
        for item in &translated {
            by_source
                .entry(item.source.as_str())
                .or_default()
                .push(item);
        }
        for items in by_source.values() {
            let mut variants: BTreeMap<&str, usize> = BTreeMap::new();
            for item in items {
                *variants.entry(item.unit.target_macro()).or_default() += 1;
            }
            if variants.len() < 2 {
                continue;
            }
            let listed = variants
                .iter()
                .map(|(target, count)| format!("«{target}» ×{count}"))
                .collect::<Vec<_>>()
                .join(" / ");
            for item in items {
                add(
                    Check::Inconsistent,
                    item,
                    format!("{} → {listed}", item.source),
                );
            }
        }
    }
    for item in &translated {
        let target = item.unit.target_macro();
        if wanted(Check::Forbidden) {
            for finding in knowledge.terms.forbidden_in(&item.source, target) {
                add(Check::Forbidden, item, finding);
            }
        }
        if wanted(Check::Structure)
            && let Err(errors) = aeria_se::check_assisted_structure(&item.source, target)
        {
            add(
                Check::Structure,
                item,
                errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if wanted(Check::BothGenders)
            && russian
            && let Some(form) = BOTH_GENDERS
                .iter()
                .find(|form| target.contains(*form) && !item.source.contains(*form))
        {
            add(Check::BothGenders, item, format!("{form} in «{target}»"));
        }
        if wanted(Check::Gender) && item.source.contains("$gn4") && !target.contains("$gn4") {
            add(Check::Gender, item, format!("«{target}»"));
        }
        if wanted(Check::Terms) {
            for finding in knowledge.terms.missing_in(&item.source, target) {
                add(Check::Terms, item, finding);
            }
        }
        if wanted(Check::Phrasing) {
            let phrasing = machine_phrasing(&target_language, target);
            if !phrasing.is_empty() {
                add(
                    Check::Phrasing,
                    item,
                    format!("{}: «{target}»", phrasing.join(", ")),
                );
            }
        }
        if wanted(Check::Long) && sheet_domain(&item.address.sheet) == Domain::Interface {
            let (source_chars, target_chars) = (plain_chars(&item.source), plain_chars(target));
            #[allow(clippy::cast_precision_loss)] // string lengths
            if target_chars >= LONG_MIN_CHARS
                && target_chars as f64 > source_chars as f64 * LONG_RATIO
            {
                add(
                    Check::Long,
                    item,
                    format!("{source_chars} → {target_chars} characters: «{target}»"),
                );
            }
        }
    }

    let total: usize = findings.values().map(Vec::len).sum();
    if out.json {
        out.json_value(&json!({
            "translations": translated.len(),
            "findings": findings
                .iter()
                .map(|(check, items)| json!({
                    "check": check.name(),
                    "count": items.len(),
                    "items": items.iter().take(options.limit).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }));
        return total == 0;
    }
    let _ = writeln!(
        out.text,
        "audited {} translations{}: {total} findings",
        translated.len(),
        options
            .pattern
            .as_deref()
            .map(|pattern| format!(" of {pattern}"))
            .unwrap_or_default()
    );
    for (check, items) in &findings {
        let _ = writeln!(
            out.text,
            "\n## {} — {} ({})",
            check.name(),
            check.describe(),
            items.len()
        );
        for item in items.iter().take(options.limit) {
            let _ = writeln!(
                out.text,
                "@{} · {} · {}",
                item.address,
                match item.by {
                    Author::Agent => "agent's",
                    Author::Person => "person's (keep)",
                },
                item.detail
            );
        }
        if items.len() > options.limit {
            let _ = writeln!(
                out.text,
                "… {} more; narrow with a pattern or --check, or pass --limit",
                items.len() - options.limit
            );
        }
    }
    total == 0
}

pub(crate) struct ReviewOptions {
    pub pattern: Option<String>,
    pub limit: usize,
}

/// Translations waiting for review, and translations whose source is gone.
pub(crate) fn review(project: &Project, options: &ReviewOptions, out: &mut Output) {
    let ledger = project.ledger();
    let in_scope = |sheet: &str| {
        options
            .pattern
            .as_deref()
            .is_none_or(|pattern| matches_pattern(sheet, pattern))
    };
    let mut waiting: Vec<(Address, &TranslationUnit)> = project
        .session
        .workspace()
        .units()
        .filter(|unit| unit.is_bound() && unit.review_state() == ReviewState::NeedsReview)
        .filter(|unit| in_scope(unit.source_binding().sheet_name()))
        .map(|unit| (address_of(unit), unit))
        .collect();
    waiting.sort_by(|left, right| left.0.cmp(&right.0));
    let detached: Vec<&TranslationUnit> = project
        .session
        .detached_units()
        .filter(|unit| in_scope(unit.source_binding().sheet_name()))
        .collect();
    if out.json {
        out.json_value(&json!({
            "needsReview": waiting.len(),
            "items": waiting.iter().take(options.limit).map(|(address, unit)| json!({
                "address": address.to_string(),
                "source": unit.source().text(),
                "target": unit.target_macro(),
                "note": unit.translator_note(),
                "by": current(&project.session, ledger.as_ref(), address).map(|now| now.author),
            })).collect::<Vec<_>>(),
            "detached": detached.iter().map(|unit| json!({
                "id": unit.id().to_string(),
                "source": unit.source().text(),
                "target": unit.target_macro(),
            })).collect::<Vec<_>>(),
        }));
        return;
    }
    let target = project
        .target_language()
        .unwrap_or_else(|| "target".to_owned());
    let _ = writeln!(
        out.text,
        "{} translations need review; {} translations lost their source after a game update",
        waiting.len(),
        detached.len()
    );
    for (address, unit) in waiting.iter().take(options.limit) {
        let by = current(&project.session, ledger.as_ref(), address)
            .map_or(Author::Person, |now| now.author);
        let _ = write!(
            out.text,
            "\n@{address} · {} {}\n  {}: {}\n  {target}: {}",
            match by {
                Author::Agent => "agent's",
                Author::Person => "person's",
            },
            review_word(unit.review_state()),
            project.source_language(),
            unit.source().text(),
            unit.target_macro()
        );
        if let Some(note) = unit.translator_note() {
            let _ = write!(out.text, "\n  note: {note}");
        }
        out.text.push('\n');
    }
    if waiting.len() > options.limit {
        let _ = writeln!(
            out.text,
            "\n… {} more; narrow with a pattern or pass --limit",
            waiting.len() - options.limit
        );
    }
    if !detached.is_empty() {
        out.text.push_str(
            "\nTranslations whose source is gone are for a person to reattach or drop in Aeria.\n",
        );
    }
}

/// Marks translations as needing a person's review with a reason in their
/// note. Returns whether every address was flagged.
pub(crate) fn flag(
    project: &mut Project,
    addresses: &[Address],
    reason: &str,
    out: &mut Output,
) -> bool {
    let mut flagged = 0;
    let mut failed = 0;
    for address in addresses {
        let Some(id) = project
            .session
            .workspace()
            .unit_by_source_binding(&address.binding())
            .map(TranslationUnit::id)
        else {
            failed += 1;
            let _ = writeln!(
                out.text,
                "not flagged @{address}: it has no translation; tell the user instead"
            );
            continue;
        };
        let note = project
            .session
            .workspace()
            .unit(id)
            .and_then(TranslationUnit::translator_note)
            .map(str::to_owned);
        let line = format!("{FLAG_NOTE} {}", reason.trim());
        let note = match note {
            Some(note) if note.contains(&line) => note,
            Some(note) if !note.trim().is_empty() => format!("{}\n{line}", note.trim_end()),
            _ => line,
        };
        let result = project.session.set_note(id, Some(note)).and_then(|()| {
            project
                .session
                .set_review_state(id, ReviewState::NeedsReview)
        });
        match result {
            Ok(()) => {
                flagged += 1;
                let _ = writeln!(out.text, "flagged @{address}");
            }
            Err(error) => {
                failed += 1;
                let _ = writeln!(out.text, "not flagged @{address}: {error}");
            }
        }
    }
    if flagged > 0
        && let Some(files) = &project.files
        && let Err(error) = files.touch_stamp()
    {
        out.warn(&format!(
            "the strings were flagged, but an open Aeria window may show it only after it reopens the project: {error}"
        ));
    }
    let _ = writeln!(out.text, "flagged {flagged} · not flagged {failed}");
    failed == 0
}
