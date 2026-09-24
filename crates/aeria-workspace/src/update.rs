//! Atomic application of deterministic source updates.
//!
//! `aeria-rebase` owns the planning rules. This module turns a plan into the
//! next workspace state and hands it to the store for publication. It never
//! removes a unit or changes a target, note, or translation-unit ID.

use std::collections::{BTreeMap, BTreeSet};

use aeria_core::{TranslationUnit, TranslationUnitId, WorkspaceMetadata};
use aeria_rebase::{SourceUpdatePlan, UnitUpdateOutcome};

use crate::{Workspace, WorkspaceError};

/// The plan behind a source update together with the stored format it was
/// applied to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceUpdateReport {
    /// The deterministic plan that was applied or previewed.
    pub plan: SourceUpdatePlan,
    /// The Workspace Format version read before the update. It is older than
    /// the current version when the update also migrates the format.
    pub previous_format_version: u8,
}

/// Builds the post-update workspace and the shards whose bytes change.
///
/// `rewrite_all` requests every existing shard, which a format migration
/// requires even for units whose source facts are unchanged.
pub(crate) fn apply_plan(
    metadata: &WorkspaceMetadata,
    units: &BTreeMap<TranslationUnitId, TranslationUnit>,
    plan: &SourceUpdatePlan,
    rewrite_all: bool,
) -> Result<(Workspace, BTreeSet<u8>), WorkspaceError> {
    let metadata = metadata.with_source_content_id(plan.source_snapshot.content_id.clone())?;
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
                proposed.binding.clone(),
                proposed.fingerprint,
                proposed.layout,
                proposed.row_key,
                outcome == UnitUpdateOutcome::SourceChanged,
            ),
            (_, None) => unreachable!("bound plan outcomes always carry proposed source facts"),
        }
        shards.insert(id.as_bytes()[0]);
    }
    if rewrite_all {
        shards.extend(updated.keys().map(|id| id.as_bytes()[0]));
    }
    let workspace = Workspace::from_loaded(metadata, updated)?;
    Ok((workspace, shards))
}
