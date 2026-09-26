//! Atomic application of deterministic source updates.
//!
//! `aeria-rebase` owns the planning rules. This module turns a plan into the
//! next workspace state and hands it to the store for publication. It never
//! removes a unit or changes a target, note, or translation-unit ID.

use std::collections::{BTreeMap, BTreeSet};

use aeria_core::{TranslationUnit, TranslationUnitId, WorkspaceMetadata};
use aeria_rebase::{SourceUpdatePlan, UnitUpdateOutcome};

use crate::{Workspace, WorkspaceError};

/// The plan behind a source update that was applied or previewed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceUpdateReport {
    /// The deterministic plan.
    pub plan: SourceUpdatePlan,
}

/// Builds the post-update workspace and the shards whose bytes change.
pub(crate) fn apply_plan(
    metadata: &WorkspaceMetadata,
    units: &BTreeMap<TranslationUnitId, TranslationUnit>,
    plan: &SourceUpdatePlan,
) -> Result<(Workspace, BTreeSet<u8>), WorkspaceError> {
    let metadata = metadata.with_game_version(plan.game_version.clone());
    let mut updated = units.clone();
    let mut shards = BTreeSet::new();
    for entry in plan.entries() {
        if !entry.changes_unit() {
            continue;
        }
        let id = entry.translation_unit_id;
        let unit = updated
            .get_mut(&id)
            .ok_or(WorkspaceError::UnitNotFound { id })?;
        match (entry.outcome, &entry.proposed) {
            (UnitUpdateOutcome::Detached(reason), _) => unit.detach(reason),
            (outcome, Some(proposed)) => unit.bind_after_source_update(
                proposed.clone(),
                outcome == UnitUpdateOutcome::SourceChanged,
            ),
            (_, None) => unreachable!("bound plan outcomes always carry proposed source facts"),
        }
        shards.insert(id.as_bytes()[0]);
    }
    let workspace = Workspace::from_loaded(metadata, updated)?;
    Ok((workspace, shards))
}
