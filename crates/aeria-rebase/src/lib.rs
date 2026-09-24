//! Deterministic source update planning.
//!
//! The planner moves managed translation units from the source facts they
//! were last bound to onto one verified HXS snapshot. It needs only the
//! workspace's persisted source facts, the new snapshot, and the new
//! snapshot's translation permission. The previous snapshot is not required:
//! a game update overwrites the installation that produced it.

#![forbid(unsafe_code)]

pub mod candidates;
mod row_key;

pub use row_key::{MIN_KEYED_ROWS, RowKeys};

use std::collections::{BTreeMap, BTreeSet};

use aeria_core::{
    DetachReason, Sha256Hash, SourceBinding, SourceFingerprint, SourceLayout, SourceStatus,
    TranslationUnit, TranslationUnitId, WorkspaceMetadata,
};
use aeria_hxs::{
    ColumnType, HxsError, HxsSnapshot, MAX_STRING_OCCURRENCE_PAGE_SIZE, SheetMetadata,
    SnapshotMetadata, StringOccurrenceCoordinate,
};
use thiserror::Error;

/// Errors raised when a deterministic source update plan cannot be built.
#[derive(Debug, Error)]
pub enum SourceUpdateError {
    /// The new snapshot has a different source language than the workspace.
    #[error(
        "new HXS source language does not match the workspace: expected {expected:?}, found {found:?}"
    )]
    SourceLanguageMismatch { expected: String, found: String },

    /// The new snapshot could not be read.
    #[error("could not read the new HXS source: {0}")]
    SourceRead(#[source] HxsError),

    /// The borrowed workspace view did not satisfy its own uniqueness rules.
    #[error("invalid workspace input: {message}")]
    InvalidWorkspace { message: String },
}

/// The stable identity facts of one HXS snapshot included in a plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshotIdentity {
    pub game_version: String,
    pub source_language: String,
    pub scope: String,
    pub content_id: String,
    pub snapshot_id: String,
}

impl SourceSnapshotIdentity {
    pub(crate) fn from_metadata(metadata: &SnapshotMetadata) -> Self {
        Self {
            game_version: metadata.game_version.clone(),
            source_language: metadata.source_language.clone(),
            scope: metadata.scope.clone(),
            content_id: metadata.content_id.clone(),
            snapshot_id: metadata.snapshot_id.clone(),
        }
    }
}

/// Deterministic evidence for a non-authoritative candidate suggestion.
///
/// Candidate suggestions are review assistance only. No variant establishes
/// source identity or changes a plan.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CandidateEvidence {
    /// The complete persisted fingerprint matches at another binding.
    CompleteFingerprint,
    /// Macro-text and the old optional raw-value hash matched.
    MacroAndRawValue,
    /// Macro-text and row technical hashes matched.
    MacroAndRowTechnical,
    /// The macro-text hash matched.
    ExactMacroText,
    /// Protected macro structure was equivalent while macro text differed.
    ProtectedStructureCompatible,
    /// Visible/translatable text similarity supplied the ranking evidence.
    VisibleTextSimilarity,
}

/// Diagnostic status for row technical context at the resolved occurrence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceContextStatus {
    /// The row technical hash is unchanged.
    Unchanged,
    /// The row technical hash changed; this does not change the outcome.
    Changed,
}

/// The deterministic result for one managed translation unit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UnitUpdateOutcome {
    /// The unit is bound to an occurrence with unchanged String content.
    Unchanged,
    /// The unit is bound to an occurrence with the same macro text whose raw
    /// source bytes changed. The translation is written against the macro
    /// text, so its review state is kept.
    EncodingChanged,
    /// The unit is bound to an occurrence whose macro text changed; its
    /// translation needs review.
    SourceChanged,
    /// The unit is preserved without a current source occurrence.
    Detached(DetachReason),
}

impl UnitUpdateOutcome {
    /// Returns whether the outcome binds the unit to an occurrence.
    #[must_use]
    pub const fn is_bound(self) -> bool {
        !matches!(self, Self::Detached(_))
    }
}

/// How a bound outcome's row was established.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RowContinuity {
    /// The row and subrow IDs are unchanged.
    SameRow,
    /// The sheet is keyed and the unit's row key was found at another row.
    RowKey {
        previous_row_id: u32,
        previous_subrow_id: u16,
    },
}

/// How a bound outcome's column was established.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ColumnContinuity {
    /// The column index was interpreted in an unchanged sheet schema. A bound
    /// Workspace Format v1 unit without recorded layout is treated the same
    /// way when the source content ID did not change.
    SameColumn,
    /// The sheet schema changed and the unit's column was mapped by a
    /// sheet-level column mapping.
    Mapped {
        previous_column: u32,
        evidence: ColumnMappingEvidence,
    },
}

/// How a bound outcome's occurrence was established.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Continuity {
    pub row: RowContinuity,
    pub column: ColumnContinuity,
}

impl Continuity {
    /// The previous binding, unchanged, in an unchanged schema generation.
    pub const SAME_BINDING: Self = Self {
        row: RowContinuity::SameRow,
        column: ColumnContinuity::SameColumn,
    };

    /// An unchanged row with a column established by a column mapping.
    #[must_use]
    pub const fn column_mapped(previous_column: u32, evidence: ColumnMappingEvidence) -> Self {
        Self {
            row: RowContinuity::SameRow,
            column: ColumnContinuity::Mapped {
                previous_column,
                evidence,
            },
        }
    }
}

/// Evidence for one sheet-level column mapping.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ColumnMappingEvidence {
    /// A strict majority of the column's managed units found their exact
    /// previous macro text in exactly one String column of their resolved
    /// row, and it was this column.
    ExactContent { supporting: usize, cast: usize },
    /// No managed unit of the column found exact content evidence, and the
    /// new schema has a String column at the same index and offset.
    UnchangedPosition,
}

/// The mapping of one previous String column of a schema-changed sheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColumnMapping {
    pub previous_column: u32,
    /// The mapped current column. `None` means the column is unresolved and
    /// its units are detached.
    pub column: Option<u32>,
    pub evidence: Option<ColumnMappingEvidence>,
}

/// A sheet whose managed units were bound in another schema generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetSchemaUpdate {
    pub sheet_name: String,
    /// The schema the units were bound in; `None` for Workspace Format v1
    /// units that did not record it.
    pub previous_schema_hash: Option<Sha256Hash>,
    /// The current schema; `None` when the sheet was removed or is
    /// unavailable.
    pub schema_hash: Option<Sha256Hash>,
    /// The sheet exists in the game catalog but the current source excludes
    /// it as unreadable or unsupported.
    pub unavailable: bool,
    /// Column mappings in ascending previous-column order.
    pub columns: Vec<ColumnMapping>,
}

/// The source facts proposed for a bound outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposedSource {
    pub binding: SourceBinding,
    pub fingerprint: SourceFingerprint,
    pub layout: SourceLayout,
    /// The row key of the new row when the new sheet is keyed.
    pub row_key: Option<Sha256Hash>,
}

/// One deterministic plan entry for an existing managed unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitUpdate {
    pub translation_unit_id: TranslationUnitId,
    pub previous_status: SourceStatus,
    pub previous_binding: SourceBinding,
    pub previous_fingerprint: SourceFingerprint,
    pub previous_layout: Option<SourceLayout>,
    pub previous_row_key: Option<Sha256Hash>,
    pub outcome: UnitUpdateOutcome,
    /// Present for bound outcomes.
    pub continuity: Option<Continuity>,
    /// Present for bound outcomes.
    pub proposed: Option<ProposedSource>,
    /// Present for bound outcomes.
    pub context_status: Option<SourceContextStatus>,
}

impl UnitUpdate {
    /// Returns whether applying this entry changes the persisted unit.
    #[must_use]
    pub fn changes_unit(&self) -> bool {
        match (&self.outcome, &self.proposed) {
            (UnitUpdateOutcome::Detached(reason), _) => {
                self.previous_status != SourceStatus::Detached(*reason)
            }
            (_, Some(proposed)) => {
                self.previous_status != SourceStatus::Bound
                    || proposed.binding != self.previous_binding
                    || proposed.fingerprint != self.previous_fingerprint
                    || Some(proposed.layout) != self.previous_layout
                    || proposed.row_key != self.previous_row_key
            }
            (_, None) => false,
        }
    }

    /// Returns whether this entry binds a previously detached unit.
    #[must_use]
    pub fn reattaches(&self) -> bool {
        !self.previous_status.is_bound() && self.proposed.is_some()
    }
}

/// Deterministic counts derived from the plan entries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceUpdateSummary {
    /// Bound units with unchanged source content.
    pub unchanged: usize,
    /// Bound units whose macro text is unchanged but whose raw source bytes
    /// changed; their review state is kept.
    pub encoding_changed: usize,
    /// Bound units whose macro text changed and now need review.
    pub source_changed: usize,
    /// Units detached after the update, including units that were already
    /// detached and remain so.
    pub detached: usize,
    /// Previously bound units that the update detaches.
    pub newly_detached: usize,
    /// Previously detached units that the update binds again.
    pub reattached: usize,
    /// Bound units whose column index changed through a column mapping.
    pub column_mapped: usize,
    /// Bound units whose row changed because their row key moved.
    pub row_moved: usize,
    /// Units whose persisted facts change when the plan is applied.
    pub changed_units: usize,
}

/// A pure, owned plan for moving managed units onto one verified snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceUpdatePlan {
    pub previous_content_id: String,
    pub source_snapshot: SourceSnapshotIdentity,
    pub sheet_schema_updates: Vec<SheetSchemaUpdate>,
    pub unit_entries: Vec<UnitUpdate>,
    pub summary: SourceUpdateSummary,
}

impl SourceUpdatePlan {
    /// Returns entries in ascending [`TranslationUnitId`] order.
    #[must_use]
    pub fn entries(&self) -> &[UnitUpdate] {
        &self.unit_entries
    }

    /// Returns whether the source content identity changes.
    #[must_use]
    pub fn changes_content_id(&self) -> bool {
        self.previous_content_id != self.source_snapshot.content_id
    }
}

/// Builds a deterministic source update plan.
///
/// `is_translatable` must answer from the permission index that belongs to
/// `snapshot`. Units are collected and sorted by durable ID, so callers may
/// provide them in any order. Nothing is mutated.
///
/// # Errors
///
/// Returns [`SourceUpdateError`] when the source language differs, the
/// snapshot cannot be read, or the unit view repeats an ID.
pub fn plan_source_update<'a, I, F>(
    workspace_metadata: &WorkspaceMetadata,
    units: I,
    snapshot: &HxsSnapshot,
    is_translatable: F,
) -> Result<SourceUpdatePlan, SourceUpdateError>
where
    I: IntoIterator<Item = &'a TranslationUnit>,
    F: Fn(&SourceBinding) -> bool,
{
    let metadata = snapshot.metadata();
    if metadata.source_language != workspace_metadata.source_language() {
        return Err(SourceUpdateError::SourceLanguageMismatch {
            expected: workspace_metadata.source_language().to_owned(),
            found: metadata.source_language,
        });
    }
    let same_content = metadata.content_id == workspace_metadata.source_content_id();

    let mut units: Vec<&TranslationUnit> = units.into_iter().collect();
    units.sort_unstable_by_key(|unit| unit.id());
    if let Some(pair) = units.windows(2).find(|pair| pair[0].id() == pair[1].id()) {
        return Err(SourceUpdateError::InvalidWorkspace {
            message: format!("duplicate TranslationUnitId {}", pair[0].id()),
        });
    }

    let sheets: BTreeMap<String, SheetMetadata> = snapshot
        .sheets()
        .into_iter()
        .map(|sheet| (sheet.name.clone(), sheet))
        .collect();
    let excluded_sheets: BTreeSet<String> = snapshot
        .excluded_sheets()
        .iter()
        .map(|sheet| sheet.name.clone())
        .collect();
    let mut by_sheet: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, unit) in units.iter().enumerate() {
        by_sheet
            .entry(unit.source_binding().sheet_name())
            .or_default()
            .push(index);
    }

    let mut resolved: BTreeMap<usize, Resolution> = BTreeMap::new();
    let mut sheet_schema_updates = Vec::new();
    for (sheet_name, members) in by_sheet {
        let members: Vec<(usize, &TranslationUnit)> = members
            .into_iter()
            .map(|index| (index, units[index]))
            .collect();
        let sheet_plan = match sheets.get(sheet_name) {
            Some(sheet) => {
                let current = CurrentSheet::read(snapshot, sheet)?;
                let row_keys = RowKeys::detect(
                    &current.name,
                    current.string_columns.keys().copied(),
                    &current.occurrences,
                    &is_translatable,
                );
                plan_sheet(&current, row_keys.as_ref(), &members, same_content)
            }
            None => plan_absent_sheet(sheet_name, &members, excluded_sheets.contains(sheet_name)),
        };
        resolved.extend(sheet_plan.resolutions);
        sheet_schema_updates.extend(sheet_plan.schema_updates);
    }

    let mut resolutions: Vec<Resolution> = resolved.into_values().collect();
    debug_assert_eq!(resolutions.len(), units.len());
    for resolution in &mut resolutions {
        if let Resolution::Bound(bound) = resolution
            && !is_translatable(&bound.proposed.binding)
        {
            *resolution = Resolution::Detached(DetachReason::NotTranslatable);
        }
    }
    resolve_binding_conflicts(&units, &mut resolutions);

    let unit_entries: Vec<UnitUpdate> = units
        .iter()
        .zip(resolutions)
        .map(|(unit, resolution)| resolution.into_entry(unit))
        .collect();
    let summary = summarize(&unit_entries);
    Ok(SourceUpdatePlan {
        previous_content_id: workspace_metadata.source_content_id().to_owned(),
        source_snapshot: SourceSnapshotIdentity::from_metadata(&metadata),
        sheet_schema_updates,
        unit_entries,
        summary,
    })
}

fn schema_of(unit: &TranslationUnit) -> Option<Sha256Hash> {
    unit.source_layout()
        .map(|layout| layout.sheet_schema_hash())
}

/// Resolutions for one sheet's units, keyed by their index in the sorted
/// unit view, and the schema generations that needed column mapping.
struct SheetPlan {
    resolutions: Vec<(usize, Resolution)>,
    schema_updates: Vec<SheetSchemaUpdate>,
}

/// Detaches every unit of a sheet that is absent from the current source.
/// An excluded sheet still exists in the game but could not be read, so it
/// is reported as unavailable rather than removed.
fn plan_absent_sheet(
    sheet_name: &str,
    members: &[(usize, &TranslationUnit)],
    unavailable: bool,
) -> SheetPlan {
    let previous_schemas: BTreeSet<Option<Sha256Hash>> =
        members.iter().map(|(_, unit)| schema_of(unit)).collect();
    let reason = if unavailable {
        DetachReason::SheetUnavailable
    } else {
        DetachReason::SheetRemoved
    };
    SheetPlan {
        resolutions: members
            .iter()
            .map(|(index, _)| (*index, Resolution::Detached(reason)))
            .collect(),
        schema_updates: previous_schemas
            .into_iter()
            .map(|previous_schema_hash| SheetSchemaUpdate {
                sheet_name: sheet_name.to_owned(),
                previous_schema_hash,
                schema_hash: None,
                unavailable,
                columns: Vec::new(),
            })
            .collect(),
    }
}

/// The row a unit resolves to before its column is known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowTarget {
    Row {
        row_id: u32,
        subrow_id: u16,
        continuity: RowContinuity,
    },
    /// The sheet is keyed by the unit's key, but the key no longer exists.
    KeyRemoved,
}

/// Resolves every unit's row. Row keys are used for a sheet only when the
/// new sheet has a row key column and at least one unit's persisted key is
/// found in it; otherwise row IDs are kept, which is also the behavior for
/// units without a persisted key.
fn row_targets(
    row_keys: Option<&RowKeys>,
    members: &[(usize, &TranslationUnit)],
) -> BTreeMap<usize, RowTarget> {
    let keyed = row_keys.filter(|keys| {
        members.iter().any(|(_, unit)| {
            unit.source_row_key()
                .is_some_and(|key| keys.row_of(key).is_some())
        })
    });
    members
        .iter()
        .map(|&(index, unit)| {
            let binding = unit.source_binding();
            let previous = (binding.row_id(), binding.subrow_id());
            let target = match (keyed, unit.source_row_key()) {
                (Some(keys), Some(key)) => match keys.row_of(key) {
                    Some((row_id, subrow_id)) => RowTarget::Row {
                        row_id,
                        subrow_id,
                        continuity: if (row_id, subrow_id) == previous {
                            RowContinuity::SameRow
                        } else {
                            RowContinuity::RowKey {
                                previous_row_id: previous.0,
                                previous_subrow_id: previous.1,
                            }
                        },
                    },
                    None => RowTarget::KeyRemoved,
                },
                _ => RowTarget::Row {
                    row_id: previous.0,
                    subrow_id: previous.1,
                    continuity: RowContinuity::SameRow,
                },
            };
            (index, target)
        })
        .collect()
}

fn plan_sheet(
    current: &CurrentSheet,
    row_keys: Option<&RowKeys>,
    members: &[(usize, &TranslationUnit)],
    same_content: bool,
) -> SheetPlan {
    let rows = row_targets(row_keys, members);
    let mut resolutions = Vec::with_capacity(members.len());
    let mut generations: BTreeMap<Option<Sha256Hash>, Vec<(usize, &TranslationUnit)>> =
        BTreeMap::new();
    for &(index, unit) in members {
        let RowTarget::Row {
            row_id,
            subrow_id,
            continuity,
        } = rows[&index]
        else {
            resolutions.push((index, Resolution::Detached(DetachReason::RowRemoved)));
            continue;
        };
        let previous_schema = schema_of(unit);
        // Unchanged content proves an unchanged schema only for units that
        // are bound in the workspace's content. A detached unit keeps the
        // layout it was last bound in, which may be older.
        let bound_in_same_content = same_content && unit.is_bound();
        if previous_schema == Some(current.schema_hash)
            || (bound_in_same_content && previous_schema.is_none())
        {
            let target = Target {
                row_id,
                subrow_id,
                column: unit.source_binding().column_index(),
                continuity: Continuity {
                    row: continuity,
                    column: ColumnContinuity::SameColumn,
                },
            };
            resolutions.push((index, current.resolve(unit, target, row_keys)));
        } else {
            generations
                .entry(previous_schema)
                .or_default()
                .push((index, unit));
        }
    }

    let mut schema_updates = Vec::with_capacity(generations.len());
    for (previous_schema_hash, generation) in generations {
        let voters: Vec<(&TranslationUnit, (u32, u16))> = generation
            .iter()
            .filter_map(|(index, unit)| match rows[index] {
                RowTarget::Row {
                    row_id, subrow_id, ..
                } => Some((*unit, (row_id, subrow_id))),
                RowTarget::KeyRemoved => None,
            })
            .collect();
        let mappings = current.map_columns(&voters);
        for (index, unit) in generation {
            let RowTarget::Row {
                row_id,
                subrow_id,
                continuity,
            } = rows[&index]
            else {
                continue;
            };
            let previous_column = unit.source_binding().column_index();
            let resolution = match mappings.get(&previous_column) {
                Some(ColumnMapping {
                    column: Some(column),
                    evidence: Some(evidence),
                    ..
                }) => {
                    let target = Target {
                        row_id,
                        subrow_id,
                        column: *column,
                        continuity: Continuity {
                            row: continuity,
                            column: ColumnContinuity::Mapped {
                                previous_column,
                                evidence: *evidence,
                            },
                        },
                    };
                    current.resolve(unit, target, row_keys)
                }
                _ => Resolution::Detached(DetachReason::ColumnUnresolved),
            };
            resolutions.push((index, resolution));
        }
        schema_updates.push(SheetSchemaUpdate {
            sheet_name: current.name.clone(),
            previous_schema_hash,
            schema_hash: Some(current.schema_hash),
            unavailable: false,
            columns: mappings.into_values().collect(),
        });
    }
    SheetPlan {
        resolutions,
        schema_updates,
    }
}

/// The occurrence a unit is resolved at and how it was established.
#[derive(Clone, Copy, Debug)]
struct Target {
    row_id: u32,
    subrow_id: u16,
    column: u32,
    continuity: Continuity,
}

#[derive(Clone, Debug)]
struct BoundResolution {
    proposed: ProposedSource,
    outcome: UnitUpdateOutcome,
    continuity: Continuity,
    context_status: SourceContextStatus,
}

#[derive(Clone, Debug)]
enum Resolution {
    Bound(Box<BoundResolution>),
    Detached(DetachReason),
}

impl Resolution {
    fn into_entry(self, unit: &TranslationUnit) -> UnitUpdate {
        let mut entry = UnitUpdate {
            translation_unit_id: unit.id(),
            previous_status: unit.source_status(),
            previous_binding: unit.source_binding().clone(),
            previous_fingerprint: *unit.source_fingerprint(),
            previous_layout: unit.source_layout(),
            previous_row_key: unit.source_row_key(),
            outcome: UnitUpdateOutcome::Unchanged,
            continuity: None,
            proposed: None,
            context_status: None,
        };
        match self {
            Self::Bound(bound) => {
                let bound = *bound;
                entry.outcome = bound.outcome;
                entry.continuity = Some(bound.continuity);
                entry.context_status = Some(bound.context_status);
                entry.proposed = Some(bound.proposed);
            }
            Self::Detached(reason) => entry.outcome = UnitUpdateOutcome::Detached(reason),
        }
        entry
    }
}

/// Hash-only String occurrences of one current sheet.
pub(crate) struct CurrentSheet {
    pub(crate) name: String,
    schema_hash: Sha256Hash,
    /// String column index to column offset.
    pub(crate) string_columns: BTreeMap<u32, u32>,
    pub(crate) occurrences: BTreeMap<(u32, u16, u32), SourceFingerprint>,
}

impl CurrentSheet {
    pub(crate) fn read(
        snapshot: &HxsSnapshot,
        sheet: &SheetMetadata,
    ) -> Result<Self, SourceUpdateError> {
        let string_columns = sheet
            .columns
            .iter()
            .filter(|column| column.column_type == ColumnType::String)
            .map(|column| (column.index, column.offset))
            .collect();
        let mut occurrences = BTreeMap::new();
        let mut after: Option<StringOccurrenceCoordinate> = None;
        loop {
            let page = snapshot
                .page_string_occurrences(
                    &sheet.name,
                    after.as_ref(),
                    MAX_STRING_OCCURRENCE_PAGE_SIZE,
                )
                .map_err(SourceUpdateError::SourceRead)?;
            for occurrence in page.occurrences {
                let coordinate = &occurrence.coordinate;
                occurrences.insert(
                    (
                        coordinate.row_id,
                        coordinate.subrow_id,
                        coordinate.column_index,
                    ),
                    SourceFingerprint::new(
                        Sha256Hash::from_bytes(*occurrence.macro_text_hash.as_bytes()),
                        occurrence
                            .raw_value_hash
                            .as_ref()
                            .map(|hash| Sha256Hash::from_bytes(*hash.as_bytes())),
                        Sha256Hash::from_bytes(*occurrence.row_technical_hash.as_bytes()),
                    ),
                );
            }
            match page.next_after {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        Ok(Self {
            name: sheet.name.clone(),
            schema_hash: Sha256Hash::from_bytes(*sheet.hashes.schema.as_bytes()),
            string_columns,
            occurrences,
        })
    }

    fn row_occurrences(
        &self,
        row_id: u32,
        subrow_id: u16,
    ) -> impl Iterator<Item = (u32, &SourceFingerprint)> {
        self.occurrences
            .range((row_id, subrow_id, 0)..=(row_id, subrow_id, u32::MAX))
            .map(|(&(_, _, column), fingerprint)| (column, fingerprint))
    }

    fn resolve(
        &self,
        unit: &TranslationUnit,
        target: Target,
        row_keys: Option<&RowKeys>,
    ) -> Resolution {
        let Some(&column_offset) = self.string_columns.get(&target.column) else {
            return Resolution::Detached(DetachReason::CellRemoved);
        };
        let Some(fingerprint) =
            self.occurrences
                .get(&(target.row_id, target.subrow_id, target.column))
        else {
            return Resolution::Detached(DetachReason::RowRemoved);
        };
        let previous = unit.source_fingerprint();
        let outcome = if previous.macro_text_hash() != fingerprint.macro_text_hash() {
            UnitUpdateOutcome::SourceChanged
        } else if previous.raw_value_hash() != fingerprint.raw_value_hash() {
            UnitUpdateOutcome::EncodingChanged
        } else {
            UnitUpdateOutcome::Unchanged
        };
        let context_status = if previous.row_technical_hash() == fingerprint.row_technical_hash() {
            SourceContextStatus::Unchanged
        } else {
            SourceContextStatus::Changed
        };
        Resolution::Bound(Box::new(BoundResolution {
            proposed: ProposedSource {
                binding: SourceBinding::new(
                    self.name.clone(),
                    target.row_id,
                    target.subrow_id,
                    target.column,
                ),
                fingerprint: *fingerprint,
                layout: SourceLayout::new(self.schema_hash, column_offset),
                row_key: row_keys.and_then(|keys| keys.key_of(target.row_id, target.subrow_id)),
            },
            outcome,
            continuity: target.continuity,
            context_status,
        }))
    }

    /// Maps every previous column used by one schema generation's units.
    ///
    /// Each unit is given with the row it resolves to. A unit casts a vote
    /// for a current column only when its exact previous macro text occurs
    /// in exactly one String column of that row. A previous column maps to
    /// the column that holds a strict majority of the votes cast by its
    /// units. A column without any cast vote maps to the same index only
    /// when the current schema has a String column at the same index and
    /// offset. Two previous columns mapping to one current column are both
    /// unresolved.
    fn map_columns(
        &self,
        units: &[(&TranslationUnit, (u32, u16))],
    ) -> BTreeMap<u32, ColumnMapping> {
        let mut votes: BTreeMap<u32, BTreeMap<u32, usize>> = BTreeMap::new();
        let mut offsets: BTreeMap<u32, BTreeSet<Option<u32>>> = BTreeMap::new();
        for &(unit, (row_id, subrow_id)) in units {
            let previous_column = unit.source_binding().column_index();
            votes.entry(previous_column).or_default();
            offsets
                .entry(previous_column)
                .or_default()
                .insert(unit.source_layout().map(|layout| layout.column_offset()));
            let macro_text_hash = unit.source_fingerprint().macro_text_hash();
            let mut matches = self
                .row_occurrences(row_id, subrow_id)
                .filter(|(_, fingerprint)| fingerprint.macro_text_hash() == macro_text_hash)
                .map(|(column, _)| column);
            if let (Some(column), None) = (matches.next(), matches.next()) {
                *votes
                    .entry(previous_column)
                    .or_default()
                    .entry(column)
                    .or_default() += 1;
            }
        }

        let mut mappings: BTreeMap<u32, ColumnMapping> = BTreeMap::new();
        for (previous_column, column_votes) in &votes {
            let cast: usize = column_votes.values().sum();
            let leader = column_votes
                .iter()
                .max_by(|left, right| left.1.cmp(right.1).then(right.0.cmp(left.0)));
            let mapping = if let Some((&column, &supporting)) = leader {
                if supporting * 2 > cast {
                    ColumnMapping {
                        previous_column: *previous_column,
                        column: Some(column),
                        evidence: Some(ColumnMappingEvidence::ExactContent { supporting, cast }),
                    }
                } else {
                    unresolved(*previous_column)
                }
            } else {
                let unchanged_position = offsets.get(previous_column).is_some_and(|offsets| {
                    offsets.len() == 1
                        && offsets
                            .iter()
                            .next()
                            .copied()
                            .flatten()
                            .is_some_and(|offset| {
                                self.string_columns.get(previous_column) == Some(&offset)
                            })
                });
                if unchanged_position {
                    ColumnMapping {
                        previous_column: *previous_column,
                        column: Some(*previous_column),
                        evidence: Some(ColumnMappingEvidence::UnchangedPosition),
                    }
                } else {
                    unresolved(*previous_column)
                }
            };
            mappings.insert(*previous_column, mapping);
        }

        let mut claims: BTreeMap<u32, usize> = BTreeMap::new();
        for mapping in mappings.values() {
            if let Some(column) = mapping.column {
                *claims.entry(column).or_default() += 1;
            }
        }
        for mapping in mappings.values_mut() {
            if mapping.column.is_some_and(|column| claims[&column] > 1) {
                *mapping = unresolved(mapping.previous_column);
            }
        }
        mappings
    }
}

const fn unresolved(previous_column: u32) -> ColumnMapping {
    ColumnMapping {
        previous_column,
        column: None,
        evidence: None,
    }
}

/// Keeps exactly one unit per resolved binding and detaches the others.
///
/// Preference order: a bound unit that keeps its exact binding in an
/// unchanged schema generation, then unchanged macro text, then a previously
/// bound unit, then the smallest durable ID.
fn resolve_binding_conflicts(units: &[&TranslationUnit], resolutions: &mut [Resolution]) {
    let mut claims: BTreeMap<SourceBinding, Vec<usize>> = BTreeMap::new();
    for (index, resolution) in resolutions.iter().enumerate() {
        if let Resolution::Bound(bound) = resolution {
            claims
                .entry(bound.proposed.binding.clone())
                .or_default()
                .push(index);
        }
    }
    for (binding, claimants) in claims {
        if claimants.len() < 2 {
            continue;
        }
        let rank = |index: usize| {
            let unit = units[index];
            let Resolution::Bound(bound) = &resolutions[index] else {
                unreachable!("claimants are bound resolutions");
            };
            let keeps_binding = unit.is_bound()
                && unit.source_binding() == &binding
                && unit.source_layout() == Some(bound.proposed.layout);
            (
                !keeps_binding,
                bound.outcome == UnitUpdateOutcome::SourceChanged,
                !unit.is_bound(),
                unit.id(),
            )
        };
        let winner = *claimants
            .iter()
            .min_by_key(|&&index| rank(index))
            .expect("conflicts have claimants");
        for index in claimants {
            if index != winner {
                resolutions[index] = Resolution::Detached(DetachReason::BindingConflict);
            }
        }
    }
}

fn summarize(entries: &[UnitUpdate]) -> SourceUpdateSummary {
    let mut summary = SourceUpdateSummary::default();
    for entry in entries {
        match entry.outcome {
            UnitUpdateOutcome::Unchanged => summary.unchanged += 1,
            UnitUpdateOutcome::EncodingChanged => summary.encoding_changed += 1,
            UnitUpdateOutcome::SourceChanged => summary.source_changed += 1,
            UnitUpdateOutcome::Detached(_) => {
                summary.detached += 1;
                if entry.previous_status.is_bound() {
                    summary.newly_detached += 1;
                }
            }
        }
        if entry.reattaches() {
            summary.reattached += 1;
        }
        if let (Some(continuity), Some(proposed)) = (entry.continuity, entry.proposed.as_ref()) {
            if matches!(continuity.column, ColumnContinuity::Mapped { .. })
                && proposed.binding.column_index() != entry.previous_binding.column_index()
            {
                summary.column_mapped += 1;
            }
            if matches!(continuity.row, RowContinuity::RowKey { .. }) {
                summary.row_moved += 1;
            }
        }
        if entry.changes_unit() {
            summary.changed_units += 1;
        }
    }
    summary
}
