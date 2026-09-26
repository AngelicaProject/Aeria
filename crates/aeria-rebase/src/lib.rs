//! Deterministic source update planning.
//!
//! The planner moves translation units from the source facts they were last
//! bound to onto the installed game. It needs only the units' persisted facts
//! and the game: a patch has already replaced the previous game version. The
//! rules are specified in `docs/architecture/rebase-safety.md`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use aeria_core::{
    DetachReason, GameVersion, LayoutHash, SourceBinding, SourceFacts, SourceStatus,
    TranslationUnit, TranslationUnitId, WorkspaceMetadata,
};
use aeria_source::{GameSource, RowKeys, SheetLookup, SourceError, SourceSheet};
use thiserror::Error;

/// Errors raised when a source update cannot be planned.
#[derive(Debug, Error)]
pub enum SourceUpdateError {
    /// The game is opened in another language than the project's source.
    #[error("the game source language {found:?} differs from the project's {expected:?}")]
    SourceLanguageMismatch { expected: String, found: String },

    /// The game is older than the version the project describes.
    #[error("the game version {game} is older than the project's {project}")]
    GameOutdated {
        project: GameVersion,
        game: GameVersion,
    },

    /// The game could not be read.
    #[error("could not read the game: {0}")]
    SourceRead(#[from] SourceError),

    /// The units repeat an ID.
    #[error("invalid workspace input: {message}")]
    InvalidWorkspace { message: String },
}

/// The deterministic result for one translation unit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UnitUpdateOutcome {
    /// The unit is bound to a cell with the same text.
    Unchanged,
    /// The unit is bound to a cell whose text changed; its translation
    /// needs review.
    SourceChanged,
    /// The unit is preserved without a current cell.
    Detached(DetachReason),
}

impl UnitUpdateOutcome {
    /// Returns whether the outcome binds the unit to a cell.
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
    /// The column index was interpreted in an unchanged layout.
    SameColumn,
    /// The layout changed and the column was mapped by a sheet-level column
    /// mapping.
    Mapped {
        previous_column: u32,
        evidence: ColumnMappingEvidence,
    },
}

/// How a bound outcome's cell was established.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Continuity {
    pub row: RowContinuity,
    pub column: ColumnContinuity,
}

impl Continuity {
    /// The previous binding, unchanged, in an unchanged layout.
    pub const SAME_BINDING: Self = Self {
        row: RowContinuity::SameRow,
        column: ColumnContinuity::SameColumn,
    };
}

/// Evidence for one sheet-level column mapping.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ColumnMappingEvidence {
    /// A strict majority of the column's units found their exact previous
    /// text in exactly one String column of their resolved row, and it was
    /// this column.
    ExactContent { supporting: usize, cast: usize },
}

/// The mapping of one previous String column of a sheet whose layout
/// changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColumnMapping {
    pub previous_column: u32,
    /// The mapped current column; `None` when the column is unresolved and
    /// its units are detached.
    pub column: Option<u32>,
    pub evidence: Option<ColumnMappingEvidence>,
}

/// A sheet whose units were bound in another layout, or that is gone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetLayoutUpdate {
    pub sheet_name: String,
    /// The layout the units were bound in.
    pub previous_layout: LayoutHash,
    /// The current layout; `None` when the sheet was removed or cannot be
    /// read.
    pub layout: Option<LayoutHash>,
    /// The sheet is listed by the game but cannot be read.
    pub unavailable: bool,
    /// Column mappings in ascending previous-column order.
    pub columns: Vec<ColumnMapping>,
}

/// One plan entry for an existing unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitUpdate {
    pub translation_unit_id: TranslationUnitId,
    pub previous_status: SourceStatus,
    pub previous: SourceFacts,
    pub outcome: UnitUpdateOutcome,
    /// Present for bound outcomes.
    pub continuity: Option<Continuity>,
    /// The new source facts; present for bound outcomes.
    pub proposed: Option<SourceFacts>,
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
                self.previous_status != SourceStatus::Bound || *proposed != self.previous
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

/// Counts derived from the plan entries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceUpdateSummary {
    /// Bound units with unchanged text.
    pub unchanged: usize,
    /// Bound units whose text changed and now need review.
    pub source_changed: usize,
    /// Units detached after the update, including units that stay detached.
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

/// A pure, owned plan for moving units onto the installed game.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceUpdatePlan {
    pub previous_game_version: GameVersion,
    pub game_version: GameVersion,
    pub sheet_layout_updates: Vec<SheetLayoutUpdate>,
    pub unit_entries: Vec<UnitUpdate>,
    pub summary: SourceUpdateSummary,
}

impl SourceUpdatePlan {
    /// Returns entries in ascending [`TranslationUnitId`] order.
    #[must_use]
    pub fn entries(&self) -> &[UnitUpdate] {
        &self.unit_entries
    }

    /// Returns whether the project's game version changes.
    #[must_use]
    pub fn changes_game_version(&self) -> bool {
        self.previous_game_version != self.game_version
    }

    /// Returns whether applying the plan changes anything.
    #[must_use]
    pub fn changes_workspace(&self) -> bool {
        self.changes_game_version() || self.summary.changed_units > 0
    }
}

/// Builds a deterministic source update plan.
///
/// Units may be given in any order; the plan lists them by ID. Nothing is
/// mutated.
///
/// # Errors
///
/// Returns [`SourceUpdateError`] when the source language differs, the game
/// is older than the project, the game cannot be read, or an ID repeats.
pub fn plan_source_update<'a, I>(
    workspace_metadata: &WorkspaceMetadata,
    units: I,
    source: &GameSource,
) -> Result<SourceUpdatePlan, SourceUpdateError>
where
    I: IntoIterator<Item = &'a TranslationUnit>,
{
    if source.language().code() != workspace_metadata.source_language() {
        return Err(SourceUpdateError::SourceLanguageMismatch {
            expected: workspace_metadata.source_language().to_owned(),
            found: source.language().code().to_owned(),
        });
    }
    if source.version() < workspace_metadata.game_version() {
        return Err(SourceUpdateError::GameOutdated {
            project: workspace_metadata.game_version().clone(),
            game: source.version().clone(),
        });
    }

    let mut units: Vec<&TranslationUnit> = units.into_iter().collect();
    units.sort_unstable_by_key(|unit| unit.id());
    if let Some(pair) = units.windows(2).find(|pair| pair[0].id() == pair[1].id()) {
        return Err(SourceUpdateError::InvalidWorkspace {
            message: format!("the translation unit ID {} repeats", pair[0].id()),
        });
    }

    let mut by_sheet: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, unit) in units.iter().enumerate() {
        by_sheet
            .entry(unit.source_binding().sheet_name())
            .or_default()
            .push(index);
    }

    let mut resolved: BTreeMap<usize, Resolution> = BTreeMap::new();
    let mut sheet_layout_updates = Vec::new();
    for (sheet_name, members) in by_sheet {
        let members: Vec<(usize, &TranslationUnit)> = members
            .into_iter()
            .map(|index| (index, units[index]))
            .collect();
        let sheet_plan = match source.sheet(sheet_name)? {
            SheetLookup::Present(sheet) => plan_sheet(&sheet, &members),
            SheetLookup::Missing => plan_absent_sheet(sheet_name, &members, false),
            SheetLookup::Unavailable(_) => plan_absent_sheet(sheet_name, &members, true),
        };
        resolved.extend(sheet_plan.resolutions);
        sheet_layout_updates.extend(sheet_plan.layout_updates);
    }

    let mut resolutions: Vec<Resolution> = resolved.into_values().collect();
    debug_assert_eq!(resolutions.len(), units.len());
    resolve_binding_conflicts(&units, &mut resolutions);

    let unit_entries: Vec<UnitUpdate> = units
        .iter()
        .zip(resolutions)
        .map(|(unit, resolution)| resolution.into_entry(unit))
        .collect();
    let summary = summarize(&unit_entries);
    Ok(SourceUpdatePlan {
        previous_game_version: workspace_metadata.game_version().clone(),
        game_version: source.version().clone(),
        sheet_layout_updates,
        unit_entries,
        summary,
    })
}

/// Resolutions for one sheet's units, keyed by their index in the sorted
/// unit view, and the layouts that needed column mapping.
struct SheetPlan {
    resolutions: Vec<(usize, Resolution)>,
    layout_updates: Vec<SheetLayoutUpdate>,
}

/// Detaches every unit of a sheet that is missing or cannot be read.
fn plan_absent_sheet(
    sheet_name: &str,
    members: &[(usize, &TranslationUnit)],
    unavailable: bool,
) -> SheetPlan {
    let reason = if unavailable {
        DetachReason::SheetUnavailable
    } else {
        DetachReason::SheetRemoved
    };
    let previous_layouts: std::collections::BTreeSet<LayoutHash> = members
        .iter()
        .map(|(_, unit)| unit.source().layout())
        .collect();
    SheetPlan {
        resolutions: members
            .iter()
            .map(|(index, _)| (*index, Resolution::Detached(reason)))
            .collect(),
        layout_updates: previous_layouts
            .into_iter()
            .map(|previous_layout| SheetLayoutUpdate {
                sheet_name: sheet_name.to_owned(),
                previous_layout,
                layout: None,
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

/// Resolves every unit's row. Row keys are used when the sheet has a row
/// key column and at least one unit's key is found in it; otherwise row IDs
/// are kept, which is also the behavior for units without a key.
fn row_targets(
    row_keys: Option<&RowKeys>,
    members: &[(usize, &TranslationUnit)],
) -> BTreeMap<usize, RowTarget> {
    let keyed = row_keys.filter(|keys| {
        members.iter().any(|(_, unit)| {
            unit.source()
                .row_key()
                .is_some_and(|key| keys.row_of(key).is_some())
        })
    });
    members
        .iter()
        .map(|&(index, unit)| {
            let binding = unit.source_binding();
            let previous = (binding.row_id(), binding.subrow_id());
            let same_row = RowTarget::Row {
                row_id: previous.0,
                subrow_id: previous.1,
                continuity: RowContinuity::SameRow,
            };
            let target = match (keyed, unit.source().row_key()) {
                (Some(keys), Some(key)) => match keys.row_of(key) {
                    Some(row) if row == previous => same_row,
                    Some((row_id, subrow_id)) => RowTarget::Row {
                        row_id,
                        subrow_id,
                        continuity: RowContinuity::RowKey {
                            previous_row_id: previous.0,
                            previous_subrow_id: previous.1,
                        },
                    },
                    None => RowTarget::KeyRemoved,
                },
                _ => same_row,
            };
            (index, target)
        })
        .collect()
}

fn plan_sheet(sheet: &Arc<SourceSheet>, members: &[(usize, &TranslationUnit)]) -> SheetPlan {
    let current = CurrentSheet { sheet };
    let rows = row_targets(sheet.row_keys(), members);
    let mut resolutions = Vec::with_capacity(members.len());
    let mut generations: BTreeMap<LayoutHash, Vec<(usize, &TranslationUnit)>> = BTreeMap::new();
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
        if unit.source().layout() == sheet.layout() {
            let target = Target {
                row_id,
                subrow_id,
                column: unit.source_binding().column_index(),
                continuity: Continuity {
                    row: continuity,
                    column: ColumnContinuity::SameColumn,
                },
            };
            resolutions.push((index, current.resolve(unit, target)));
        } else {
            generations
                .entry(unit.source().layout())
                .or_default()
                .push((index, unit));
        }
    }

    let mut layout_updates = Vec::with_capacity(generations.len());
    for (previous_layout, generation) in generations {
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
                }) => current.resolve(
                    unit,
                    Target {
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
                    },
                ),
                _ => Resolution::Detached(DetachReason::ColumnUnresolved),
            };
            resolutions.push((index, resolution));
        }
        layout_updates.push(SheetLayoutUpdate {
            sheet_name: sheet.name().to_owned(),
            previous_layout,
            layout: Some(sheet.layout()),
            unavailable: false,
            columns: mappings.into_values().collect(),
        });
    }
    SheetPlan {
        resolutions,
        layout_updates,
    }
}

/// The cell a unit is resolved at and how it was established.
#[derive(Clone, Copy, Debug)]
struct Target {
    row_id: u32,
    subrow_id: u16,
    column: u32,
    continuity: Continuity,
}

#[derive(Clone, Debug)]
struct BoundResolution {
    proposed: SourceFacts,
    outcome: UnitUpdateOutcome,
    continuity: Continuity,
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
            previous: unit.source().clone(),
            outcome: UnitUpdateOutcome::Unchanged,
            continuity: None,
            proposed: None,
        };
        match self {
            Self::Bound(bound) => {
                let bound = *bound;
                entry.outcome = bound.outcome;
                entry.continuity = Some(bound.continuity);
                entry.proposed = Some(bound.proposed);
            }
            Self::Detached(reason) => entry.outcome = UnitUpdateOutcome::Detached(reason),
        }
        entry
    }
}

struct CurrentSheet<'a> {
    sheet: &'a SourceSheet,
}

impl CurrentSheet<'_> {
    fn resolve(&self, unit: &TranslationUnit, target: Target) -> Resolution {
        if !self
            .sheet
            .columns()
            .iter()
            .any(|column| column.index == target.column)
        {
            return Resolution::Detached(DetachReason::CellRemoved);
        }
        let Some(cell) = self
            .sheet
            .cell(target.row_id, target.subrow_id, target.column)
        else {
            return Resolution::Detached(DetachReason::RowRemoved);
        };
        if !cell.translatable {
            return Resolution::Detached(DetachReason::NotTranslatable);
        }
        let text = cell.text();
        let outcome = if text == unit.source().text() {
            UnitUpdateOutcome::Unchanged
        } else {
            UnitUpdateOutcome::SourceChanged
        };
        let row_key = self
            .sheet
            .row_keys()
            .and_then(|keys| keys.key_of(target.row_id, target.subrow_id))
            .map(str::to_owned);
        Resolution::Bound(Box::new(BoundResolution {
            proposed: SourceFacts::new(
                SourceBinding::new(
                    self.sheet.name(),
                    target.row_id,
                    target.subrow_id,
                    target.column,
                ),
                self.sheet.layout(),
                text,
                row_key,
            ),
            outcome,
            continuity: target.continuity,
        }))
    }

    /// Maps every previous column used by one layout's units.
    ///
    /// Each unit is given with the row it resolves to. A unit votes for a
    /// current column only when its exact previous text occurs in exactly
    /// one String column of that row. A previous column maps to the column
    /// that holds a strict majority of the votes cast by its units. Two
    /// previous columns mapping to one current column are both unresolved.
    fn map_columns(
        &self,
        units: &[(&TranslationUnit, (u32, u16))],
    ) -> BTreeMap<u32, ColumnMapping> {
        let mut votes: BTreeMap<u32, BTreeMap<u32, usize>> = BTreeMap::new();
        for &(unit, (row_id, subrow_id)) in units {
            let previous_column = unit.source_binding().column_index();
            let column_votes = votes.entry(previous_column).or_default();
            let Some(row) = self.sheet.row(row_id, subrow_id) else {
                continue;
            };
            let mut matches = self
                .sheet
                .cells(row)
                .filter(|cell| cell.text() == unit.source().text())
                .map(|cell| cell.column);
            if let (Some(column), None) = (matches.next(), matches.next()) {
                *column_votes.entry(column).or_default() += 1;
            }
        }

        let mut mappings: BTreeMap<u32, ColumnMapping> = votes
            .iter()
            .map(|(&previous_column, column_votes)| {
                let cast: usize = column_votes.values().sum();
                let leader = column_votes
                    .iter()
                    .max_by(|left, right| left.1.cmp(right.1).then(right.0.cmp(left.0)));
                let mapping = match leader {
                    Some((&column, &supporting)) if supporting * 2 > cast => ColumnMapping {
                        previous_column,
                        column: Some(column),
                        evidence: Some(ColumnMappingEvidence::ExactContent { supporting, cast }),
                    },
                    _ => unresolved(previous_column),
                };
                (previous_column, mapping)
            })
            .collect();

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
/// Preference order: a bound unit that keeps its exact binding and layout,
/// then unchanged text, then a previously bound unit, then the smallest ID.
fn resolve_binding_conflicts(units: &[&TranslationUnit], resolutions: &mut [Resolution]) {
    let mut claims: BTreeMap<SourceBinding, Vec<usize>> = BTreeMap::new();
    for (index, resolution) in resolutions.iter().enumerate() {
        if let Resolution::Bound(bound) = resolution {
            claims
                .entry(bound.proposed.binding().clone())
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
                && unit.source().layout() == bound.proposed.layout();
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
                && proposed.binding().column_index() != entry.previous.binding().column_index()
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
