//! Deterministic three-way merge of Workspace Format v1 unit shards by
//! translation-unit identity.
//!
//! Git merges JSONL shards line by line, so edits to *different* units that
//! happen to be adjacent in one shard conflict textually. This module merges
//! such shards per unit instead. Only real same-unit conflicts remain, and
//! they are never resolved by guessing: the caller must supply an explicit
//! resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use aeria_core::{TranslationUnit, TranslationUnitId};
use aeria_workspace::decode_unit_shard;

use crate::GitError;

/// Which side of a merge to keep for one conflicting unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictResolution {
    /// Keep the local version.
    Ours,
    /// Keep the incoming version.
    Theirs,
}

/// One translation unit changed differently on both sides of a merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitConflict {
    pub id: TranslationUnitId,
    pub base: Option<TranslationUnit>,
    pub ours: Option<TranslationUnit>,
    pub theirs: Option<TranslationUnit>,
}

/// The result of merging one shard.
#[derive(Debug, Default)]
pub(crate) struct ShardMerge {
    pub units: Vec<TranslationUnit>,
    pub conflicts: Vec<UnitConflict>,
}

/// Merges one shard from its base, local, and incoming versions. A conflict
/// with an entry in `resolutions` is resolved by taking that side.
pub(crate) fn merge_shard(
    path: &str,
    base: Option<&[u8]>,
    ours: Option<&[u8]>,
    theirs: Option<&[u8]>,
    resolutions: &BTreeMap<TranslationUnitId, ConflictResolution>,
) -> Result<ShardMerge, GitError> {
    let decode =
        |bytes: Option<&[u8]>| -> Result<BTreeMap<TranslationUnitId, TranslationUnit>, GitError> {
            Ok(match bytes {
                Some(bytes) => decode_unit_shard(bytes, Path::new(path))?
                    .into_iter()
                    .map(|unit| (unit.id(), unit))
                    .collect(),
                None => BTreeMap::new(),
            })
        };
    let mut base = decode(base)?;
    let mut ours = decode(ours)?;
    let mut theirs = decode(theirs)?;
    let ids: BTreeSet<TranslationUnitId> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .copied()
        .collect();

    let mut merged = ShardMerge::default();
    for id in ids {
        let (base, ours, theirs) = (base.remove(&id), ours.remove(&id), theirs.remove(&id));
        match merge_unit(base.as_ref(), ours.as_ref(), theirs.as_ref()) {
            Some(unit) => merged.units.extend(unit),
            None => match resolutions.get(&id) {
                Some(ConflictResolution::Ours) => merged.units.extend(ours),
                Some(ConflictResolution::Theirs) => merged.units.extend(theirs),
                None => merged.conflicts.push(UnitConflict {
                    id,
                    base,
                    ours,
                    theirs,
                }),
            },
        }
    }
    Ok(merged)
}

/// Merges one unit. Returns `None` for a conflict, otherwise the merged
/// version (`Some(None)` means the unit is absent).
///
/// Rules, applied in order:
///
/// 1. identical sides, or a side equal to the base, take the other side;
/// 2. adding or removing a unit on one side while the other side changed it
///    differently is a conflict;
/// 3. a source binding or fingerprint change on either side conflicts with
///    any other change on the other side (source updates are rebase work);
/// 4. target and review state merge as one pair: when only one side changed
///    the target, that side's target and review state win, because a new
///    target invalidates a review of the old one; when both changed the
///    target differently, or neither changed it but both changed the review
///    state differently, it is a conflict;
/// 5. the translator note merges independently with the same three-way rule.
#[allow(clippy::option_option)]
pub(crate) fn merge_unit(
    base: Option<&TranslationUnit>,
    ours: Option<&TranslationUnit>,
    theirs: Option<&TranslationUnit>,
) -> Option<Option<TranslationUnit>> {
    if ours == theirs || theirs == base {
        return Some(ours.cloned());
    }
    if ours == base {
        return Some(theirs.cloned());
    }
    let (base, ours, theirs) = (base?, ours?, theirs?);

    let source =
        |unit: &TranslationUnit| (unit.source_binding().clone(), *unit.source_fingerprint());
    if source(ours) != source(base) || source(theirs) != source(base) {
        return None;
    }

    let (target, review) = if ours.target_macro() == theirs.target_macro() {
        let review = three_way(
            &base.review_state(),
            &ours.review_state(),
            &theirs.review_state(),
        )?;
        (ours.target_macro(), review)
    } else if ours.target_macro() == base.target_macro() {
        (theirs.target_macro(), theirs.review_state())
    } else if theirs.target_macro() == base.target_macro() {
        (ours.target_macro(), ours.review_state())
    } else {
        return None;
    };
    let note = three_way(
        &base.translator_note(),
        &ours.translator_note(),
        &theirs.translator_note(),
    )?;

    let mut merged = ours.clone();
    merged.set_target_macro(target);
    merged.set_review_state(review);
    merged.set_translator_note(note.map(str::to_owned));
    Some(Some(merged))
}

fn three_way<T: PartialEq + Clone>(base: &T, ours: &T, theirs: &T) -> Option<T> {
    if ours == theirs || theirs == base {
        Some(ours.clone())
    } else if ours == base {
        Some(theirs.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use aeria_core::{ReviewState, Sha256Hash, SourceBinding, SourceFingerprint};

    use super::*;

    fn unit(target: &str, review: ReviewState, note: Option<&str>) -> TranslationUnit {
        let mut unit = TranslationUnit::new(
            TranslationUnitId::from_bytes([7; 32]),
            SourceBinding::new("Addon", 1, 0, 0),
            SourceFingerprint::new(
                Sha256Hash::from_bytes([1; 32]),
                None,
                Sha256Hash::from_bytes([2; 32]),
            ),
            target,
        );
        unit.set_review_state(review);
        unit.set_translator_note(note.map(str::to_owned));
        unit
    }

    use ReviewState::{Draft, NeedsReview, Reviewed};

    #[test]
    fn one_sided_changes_take_the_changed_side() {
        let base = unit("a", Draft, None);
        let changed = unit("b", Draft, None);
        assert_eq!(
            merge_unit(Some(&base), Some(&base), Some(&changed)),
            Some(Some(changed.clone()))
        );
        assert_eq!(
            merge_unit(Some(&base), Some(&changed), Some(&base)),
            Some(Some(changed.clone()))
        );
        assert_eq!(merge_unit(None, None, Some(&changed)), Some(Some(changed)));
        assert_eq!(merge_unit(Some(&base), Some(&base), None), Some(None));
    }

    #[test]
    fn a_new_target_wins_over_a_review_of_the_old_target() {
        let base = unit("a", Draft, None);
        let edited = unit("b", Draft, None);
        let reviewed = unit("a", Reviewed, None);
        assert_eq!(
            merge_unit(Some(&base), Some(&edited), Some(&reviewed)),
            Some(Some(edited.clone()))
        );
        assert_eq!(
            merge_unit(Some(&base), Some(&reviewed), Some(&edited)),
            Some(Some(edited))
        );
    }

    #[test]
    fn independent_note_and_target_changes_combine() {
        let base = unit("a", Draft, None);
        let edited = unit("b", Draft, None);
        let noted = unit("a", Draft, Some("context"));
        assert_eq!(
            merge_unit(Some(&base), Some(&edited), Some(&noted)),
            Some(Some(unit("b", Draft, Some("context"))))
        );
    }

    #[test]
    fn real_same_unit_conflicts_are_reported() {
        let base = unit("a", Draft, None);
        assert_eq!(
            merge_unit(
                Some(&base),
                Some(&unit("b", Draft, None)),
                Some(&unit("c", Draft, None))
            ),
            None
        );
        assert_eq!(
            merge_unit(
                Some(&base),
                Some(&unit("a", Reviewed, None)),
                Some(&unit("a", NeedsReview, None))
            ),
            None
        );
        assert_eq!(
            merge_unit(
                Some(&base),
                Some(&unit("a", Draft, Some("x"))),
                Some(&unit("a", Draft, Some("y")))
            ),
            None
        );
        assert_eq!(
            merge_unit(Some(&base), None, Some(&unit("b", Draft, None))),
            None
        );
        assert_eq!(
            merge_unit(
                None,
                Some(&unit("b", Draft, None)),
                Some(&unit("c", Draft, None))
            ),
            None
        );
    }
}
