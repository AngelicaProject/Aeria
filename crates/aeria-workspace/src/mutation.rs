//! Transactional application-level translation mutations.

use aeria_core::{
    ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, SourceLayout, TranslationUnit,
    TranslationUnitId,
};
use aeria_rebase::{RowKeys, SourceUpdateError};
use thiserror::Error;

use crate::{ProjectSession, WorkspaceError, WorkspaceStoreError};

/// Errors raised while applying one transactional translation mutation.
#[derive(Debug, Error)]
pub enum TranslationMutationError {
    /// The requested target contains no non-whitespace content.
    #[error("translation target must not be empty or whitespace-only")]
    EmptyTarget,

    /// The exact source occurrence is not granted by the verified HSG.
    #[error(
        "source occurrence is not translatable according to source guidance: {source_binding:?}"
    )]
    SourceNotTranslatable { source_binding: SourceBinding },

    /// The requested domain mutation was invalid.
    #[error("translation workspace mutation failed: {0}")]
    Workspace(#[from] WorkspaceError),

    /// The canonical shard could not be persisted.
    #[error("translation workspace persistence failed: {0}")]
    Persistence(#[from] WorkspaceStoreError),

    /// An existing unit no longer describes the verified source occurrence
    /// owned by this session.
    #[error(
        "translation unit {translation_unit_id} at {source_binding:?} has stale source facts: persisted {persisted:?}, verified {verified:?}"
    )]
    SourceIntegrity {
        translation_unit_id: TranslationUnitId,
        source_binding: Box<SourceBinding>,
        persisted: Box<(SourceFingerprint, Option<SourceLayout>)>,
        verified: Box<(SourceFingerprint, SourceLayout)>,
    },
}

/// The state an assisted write expects its unit to be in, captured when the
/// translation was produced. `None` fields describe an untranslated string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistedExpectation {
    pub target: Option<String>,
    pub review_state: Option<ReviewState>,
}

/// Errors from an assisted write. Nothing is written when one is returned.
#[derive(Debug, Error)]
pub enum AssistedWriteError {
    /// The ordinary target checks or persistence failed.
    #[error(transparent)]
    Mutation(#[from] TranslationMutationError),

    /// The translation breaks the assisted structure policy.
    #[error("the translation breaks the source structure: {}", .messages.join("; "))]
    Structure { messages: Vec<String> },

    /// The unit changed after the translation was produced.
    #[error("the string changed after the translation was produced")]
    Conflict { current: AssistedExpectation },

    /// The unit is reviewed and the write was not approved to replace it.
    #[error("the string is reviewed; replacing it needs explicit approval")]
    Reviewed,
}

impl ProjectSession {
    /// Returns the verified source macro text of one translatable occurrence.
    ///
    /// # Errors
    ///
    /// Returns `SourceNotTranslatable` for an occurrence that HSG does not
    /// grant or that has no String cell.
    pub fn source_macro(
        &self,
        source_binding: &SourceBinding,
    ) -> Result<String, TranslationMutationError> {
        let not_translatable = || TranslationMutationError::SourceNotTranslatable {
            source_binding: source_binding.clone(),
        };
        if !self.source_package.guidance_index().is_translatable(
            source_binding.sheet_name(),
            source_binding.row_id(),
            source_binding.subrow_id(),
            source_binding.column_index(),
        ) {
            return Err(not_translatable());
        }
        self.source_package
            .source()
            .string_cell(
                source_binding.sheet_name(),
                source_binding.row_id(),
                source_binding.subrow_id(),
                source_binding.column_index(),
            )
            .map_err(|error| TranslationMutationError::Workspace(WorkspaceError::Hxs(error)))?
            .map(|cell| cell.macro_text)
            .ok_or_else(not_translatable)
    }

    /// Returns the current target and review state of a bound unit, or the
    /// untranslated state.
    #[must_use]
    pub fn assisted_state(&self, source_binding: &SourceBinding) -> AssistedExpectation {
        self.workspace
            .unit_by_source_binding(source_binding)
            .map_or(
                AssistedExpectation {
                    target: None,
                    review_state: None,
                },
                |unit| AssistedExpectation {
                    target: Some(unit.target_macro().to_owned()),
                    review_state: Some(unit.review_state()),
                },
            )
    }

    /// Writes a target produced by assisted translation.
    ///
    /// Besides the ordinary [`Self::set_target`] checks, the target must
    /// satisfy the assisted structure policy against the verified source, and
    /// the unit must still be in the `expected` state (compare-and-set), so a
    /// translation never replaces work saved after it was produced. A
    /// reviewed unit is replaced only when `replace_reviewed` records the
    /// user's explicit approval. The written target is a draft.
    ///
    /// # Errors
    ///
    /// Returns a structure, conflict, reviewed, or ordinary mutation error;
    /// nothing is written in that case.
    pub fn set_assisted_target(
        &mut self,
        source_binding: &SourceBinding,
        target_macro: &str,
        expected: &AssistedExpectation,
        replace_reviewed: bool,
    ) -> Result<TranslationUnitId, AssistedWriteError> {
        if target_macro.trim().is_empty() {
            return Err(TranslationMutationError::EmptyTarget.into());
        }
        let source = self.source_macro(source_binding)?;
        aeria_se::check_assisted_structure(&source, target_macro).map_err(|errors| {
            AssistedWriteError::Structure {
                messages: errors.into_iter().map(|error| error.message).collect(),
            }
        })?;
        let current = self.assisted_state(source_binding);
        if &current != expected {
            return Err(AssistedWriteError::Conflict { current });
        }
        if current.review_state == Some(ReviewState::Reviewed) && !replace_reviewed {
            return Err(AssistedWriteError::Reviewed);
        }
        Ok(self.set_target(source_binding, target_macro)?)
    }
}

impl ProjectSession {
    /// Creates or updates the translation unit at one verified source binding.
    ///
    /// A missing unit is created with a stable ID derived from the verified
    /// HXS occurrence. An existing unit keeps its durable ID and uses the
    /// normal workspace target-edit semantics. Empty and whitespace-only
    /// targets are rejected before any workspace or persistence mutation.
    ///
    /// # Errors
    ///
    /// Returns a typed empty-target, workspace, persistence, or
    /// source-integrity error.
    pub fn set_target(
        &mut self,
        source_binding: &SourceBinding,
        target_macro: &str,
    ) -> Result<TranslationUnitId, TranslationMutationError> {
        if target_macro.trim().is_empty() {
            return Err(TranslationMutationError::EmptyTarget);
        }
        if !self.source_package.guidance_index().is_translatable(
            source_binding.sheet_name(),
            source_binding.row_id(),
            source_binding.subrow_id(),
            source_binding.column_index(),
        ) {
            return Err(TranslationMutationError::SourceNotTranslatable {
                source_binding: source_binding.clone(),
            });
        }
        let existing_id = self
            .workspace
            .unit_by_source_binding(source_binding)
            .map(TranslationUnit::id);

        let Some(id) = existing_id else {
            let row_key = self.row_key_for(source_binding)?;
            let id = self.workspace.create_unit_from_verified_source(
                self.source_package.source(),
                source_binding.clone(),
                target_macro,
                row_key,
            )?;
            if let Err(error) = self.store.persist_unit(&self.workspace, id) {
                debug_assert!(self.workspace.remove_unit(id).is_some());
                return Err(error.into());
            }
            return Ok(id);
        };

        self.verify_current_source(id)?;
        let Some(current) = self.workspace.unit(id) else {
            return Err(WorkspaceError::UnitNotFound { id }.into());
        };
        if current.target_macro() == target_macro {
            return Ok(id);
        }

        let previous = current.clone();
        self.workspace.update_target(id, target_macro)?;
        if let Err(error) = self.store.persist_unit(&self.workspace, id) {
            self.workspace.restore_unit(previous);
            return Err(error.into());
        }
        Ok(id)
    }

    /// Replaces the note on an existing translation unit.
    ///
    /// Clearing a note uses `None`. A note-only mutation does not change the
    /// review state, and an identical note does not rewrite the canonical
    /// shard.
    ///
    /// # Errors
    ///
    /// Returns a typed workspace, persistence, or source-integrity error.
    pub fn set_note(
        &mut self,
        translation_unit_id: TranslationUnitId,
        note: Option<String>,
    ) -> Result<(), TranslationMutationError> {
        self.verify_current_source(translation_unit_id)?;
        let Some(current) = self.workspace.unit(translation_unit_id) else {
            return Err(WorkspaceError::UnitNotFound {
                id: translation_unit_id,
            }
            .into());
        };
        if current.translator_note() == note.as_deref() {
            return Ok(());
        }

        let previous = current.clone();
        self.workspace.update_note(translation_unit_id, note)?;
        if let Err(error) = self
            .store
            .persist_unit(&self.workspace, translation_unit_id)
        {
            self.workspace.restore_unit(previous);
            return Err(error.into());
        }
        Ok(())
    }

    /// Applies an explicit review-state operation to an existing unit.
    ///
    /// An identical state does not rewrite the canonical shard.
    ///
    /// # Errors
    ///
    /// Returns a typed workspace, persistence, or source-integrity error.
    pub fn set_review_state(
        &mut self,
        translation_unit_id: TranslationUnitId,
        review_state: ReviewState,
    ) -> Result<(), TranslationMutationError> {
        self.verify_current_source(translation_unit_id)?;
        let Some(current) = self.workspace.unit(translation_unit_id) else {
            return Err(WorkspaceError::UnitNotFound {
                id: translation_unit_id,
            }
            .into());
        };
        if current.review_state() == review_state {
            return Ok(());
        }

        let previous = current.clone();
        self.workspace
            .update_review_state(translation_unit_id, review_state)?;
        if let Err(error) = self
            .store
            .persist_unit(&self.workspace, translation_unit_id)
        {
            self.workspace.restore_unit(previous);
            return Err(error.into());
        }
        Ok(())
    }

    /// Returns the row key of the binding's row when its sheet is keyed.
    fn row_key_for(
        &mut self,
        source_binding: &SourceBinding,
    ) -> Result<Option<Sha256Hash>, TranslationMutationError> {
        let sheet_name = source_binding.sheet_name();
        if !self.row_keys.contains_key(sheet_name) {
            let guidance = self.source_package.guidance_index();
            let keys = RowKeys::read(self.source_package.source(), sheet_name, |binding| {
                guidance.is_translatable(
                    binding.sheet_name(),
                    binding.row_id(),
                    binding.subrow_id(),
                    binding.column_index(),
                )
            })
            .map_err(|error| match error {
                SourceUpdateError::SourceRead(source) => WorkspaceError::Hxs(source),
                other => unreachable!("row key detection only reads the source: {other}"),
            })?;
            self.row_keys.insert(sheet_name.to_owned(), keys);
        }
        Ok(self.row_keys[sheet_name]
            .as_ref()
            .and_then(|keys| keys.key_of(source_binding.row_id(), source_binding.subrow_id())))
    }

    fn verify_current_source(
        &self,
        translation_unit_id: TranslationUnitId,
    ) -> Result<(), TranslationMutationError> {
        let unit =
            self.workspace
                .unit(translation_unit_id)
                .ok_or(WorkspaceError::UnitNotFound {
                    id: translation_unit_id,
                })?;
        if !unit.is_bound() {
            return Err(WorkspaceError::DetachedUnit {
                id: translation_unit_id,
            }
            .into());
        }
        let source_binding = unit.source_binding().clone();
        let (fingerprint, layout) =
            crate::verified_source(self.source_package.source(), &source_binding)?;
        if unit.source_fingerprint() != &fingerprint || unit.source_layout() != Some(layout) {
            return Err(TranslationMutationError::SourceIntegrity {
                translation_unit_id,
                source_binding: Box::new(source_binding),
                persisted: Box::new((*unit.source_fingerprint(), unit.source_layout())),
                verified: Box::new((fingerprint, layout)),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(dead_code, unused_imports)]
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    use aeria_core::SourceBinding;
    use aeria_hxs::HxsSnapshot;
    use rusqlite::{Connection, params};
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::{fail_next_publication_for_test, persistence_test_lock};

    const SYNTHETIC_SCHEMA: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../aeria-hxs/tests/fixtures/synthetic_v1.sql"
    ));
    const APPLICATION_ID: i64 = 0x4841_544c;

    struct Fixture {
        _directory: TempDir,
        path: PathBuf,
    }

    #[test]
    fn existing_unit_persistence_failure_rolls_back_the_live_session() {
        let _test_lock = persistence_test_lock();
        let fixture = write_fixture();
        let repository = tempfile::tempdir().expect("temporary repository");
        let binding = SourceBinding::new("Synthetic", 42, 0, 0);
        let mut session = ProjectSession::initialize(
            repository.path(),
            &fixture.path,
            repository.path().join("cache"),
            "fr",
        )
        .expect("init");
        let id = session
            .set_target(&binding, "Bonjour")
            .expect("initial target");
        let before_files = managed_files(repository.path());

        fail_next_publication_for_test();
        let error = session
            .set_target(&binding, "Salut")
            .expect_err("publication must fail");
        assert!(matches!(
            error,
            TranslationMutationError::Persistence(
                crate::WorkspaceStoreError::AtomicPublication { .. }
            )
        ));
        assert_eq!(
            session.workspace().unit(id).expect("unit").target_macro(),
            "Bonjour"
        );
        assert_eq!(before_files, managed_files(repository.path()));

        session
            .set_target(&binding, "Salut")
            .expect("subsequent update succeeds");
        assert_eq!(
            session.workspace().unit(id).expect("unit").target_macro(),
            "Salut"
        );
    }

    #[test]
    fn reload_workspace_adopts_external_changes_and_keeps_state_on_failure() {
        let _test_lock = persistence_test_lock();
        let fixture = write_fixture();
        let repository = tempfile::tempdir().expect("temporary repository");
        let binding = SourceBinding::new("Synthetic", 42, 0, 0);
        let mut writer = ProjectSession::initialize(
            repository.path(),
            &fixture.path,
            repository.path().join("cache"),
            "fr",
        )
        .expect("init");
        let mut reader = ProjectSession::open(
            repository.path(),
            &fixture.path,
            repository.path().join("cache-reader"),
        )
        .expect("open");

        let id = writer.set_target(&binding, "Bonjour").expect("target");
        assert!(reader.workspace().unit(id).is_none());
        reader.reload_workspace().expect("reload");
        assert_eq!(
            reader.workspace().unit(id).expect("unit").target_macro(),
            "Bonjour"
        );
        reader
            .set_target(&binding, "Salut")
            .expect("reloaded session can mutate");

        let shard = repository.path().join(crate::unit_shard_path(id));
        fs::write(
            &shard,
            "not json
",
        )
        .expect("corrupt shard");
        assert!(matches!(
            reader.reload_workspace(),
            Err(crate::ProjectSessionError::Store { .. })
        ));
        assert_eq!(
            reader.workspace().unit(id).expect("unit").target_macro(),
            "Salut"
        );
    }

    #[test]
    fn first_unit_persistence_failure_rolls_back_creation() {
        let _test_lock = persistence_test_lock();
        let fixture = write_fixture();
        let repository = tempfile::tempdir().expect("temporary repository");
        let binding = SourceBinding::new("Synthetic", 42, 0, 0);
        let mut session = ProjectSession::initialize(
            repository.path(),
            &fixture.path,
            repository.path().join("cache"),
            "fr",
        )
        .expect("init");
        let before_files = managed_files(repository.path());

        fail_next_publication_for_test();
        let error = session
            .set_target(&binding, "Bonjour")
            .expect_err("publication must fail");
        assert!(matches!(
            error,
            TranslationMutationError::Persistence(
                crate::WorkspaceStoreError::AtomicPublication { .. }
            )
        ));
        assert_eq!(session.workspace().units().count(), 0);
        assert_eq!(before_files, managed_files(repository.path()));

        let id = session
            .set_target(&binding, "Bonjour")
            .expect("subsequent creation succeeds");
        assert_eq!(
            session.workspace().unit(id).expect("unit").target_macro(),
            "Bonjour"
        );
    }

    fn managed_files(repository_root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let aeria_root = repository_root.join(".aeria");
        let mut files = BTreeMap::new();
        let manifest = aeria_root.join("manifest.json");
        files.insert(manifest.clone(), fs::read(manifest).expect("manifest"));
        let units = aeria_root.join("units");
        if units.is_dir() {
            for entry in fs::read_dir(units).expect("units directory") {
                let entry = entry.expect("unit entry");
                let path = entry.path();
                if path.is_file() {
                    files.insert(path.clone(), fs::read(path).expect("unit shard"));
                }
            }
        }
        files
    }

    #[allow(clippy::too_many_lines)]
    fn write_fixture() -> Fixture {
        Fixture {
            _directory: tempfile::tempdir().expect("fixture directory"),
            path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../aeria-hsp/tests/fixtures/synthetic.hsp"),
        }
    }

    #[allow(dead_code)]
    fn write_legacy_fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("fixture.hxs");
        let connection = Connection::open(&path).expect("fixture database");
        connection
            .execute_batch(SYNTHETIC_SCHEMA)
            .expect("fixture schema");
        connection
            .execute_batch(&format!(
                "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
            ))
            .expect("fixture identity");

        let macro_hash = macro_hash("one");
        let technical_hash = row_technical_hash("Synthetic", 42, 0);
        let string_hash = row_string_hash("Synthetic", 42, 0, 0, &macro_hash);
        let row_hash = row_hash("Synthetic", 42, 0, &technical_hash, &string_hash);
        let schema_hash = schema_hash("Synthetic");
        let sheet_technical_hash = sheet_rows_hash(
            "HARMONIA-HXS-V1-SHEET-TECHNICAL",
            "Synthetic",
            &[(42, 0, technical_hash)],
        );
        let sheet_string_hash = sheet_rows_hash(
            "HARMONIA-HXS-V1-SHEET-STRINGS",
            "Synthetic",
            &[(42, 0, string_hash)],
        );
        let content_hash = digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-V1-SHEET");
            framed_text(hasher, "Synthetic");
            hasher.update(0_u32.to_le_bytes());
            hasher.update(schema_hash);
            hasher.update(sheet_technical_hash);
            hasher.update(sheet_string_hash);
        });
        let content_id = format!(
            "sha256:{}",
            hex(&digest(|hasher| {
                hasher.update(b"HARMONIA-HXS-CONTENT-v1");
                framed_text(hasher, "en");
                framed_text(hasher, "Synthetic");
                framed_text(hasher, "en");
                hasher.update(schema_hash);
                hasher.update(content_hash);
            }))
        );
        let snapshot_id = format!(
            "sha256:{}",
            hex(&digest(|hasher| {
                hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
                framed_text(hasher, "test-game");
                framed_text(hasher, "en");
                framed_text(hasher, &content_id);
            }))
        );

        connection
            .execute(
                "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, 'Synthetic', 0, 'en', 1, 1, ?1, ?2, ?3, ?4)",
                params![schema_hash.as_slice(), sheet_technical_hash.as_slice(), sheet_string_hash.as_slice(), content_hash.as_slice()],
            )
            .expect("sheet");
        connection
            .execute(
                "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, 0, 0, 1)",
                [],
            )
            .expect("column");
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (1, 42, 0, ?1, ?2, ?3, ?4)",
                params![Vec::<u8>::new(), row_hash.as_slice(), technical_hash.as_slice(), string_hash.as_slice()],
            )
            .expect("row");
        connection
            .execute(
                "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, 42, 0, 0, 'one', NULL, ?1, NULL)",
                params![macro_hash.as_slice()],
            )
            .expect("String cell");
        connection
            .execute(
                "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, 'test-game', 'en', 'full', ?1, ?2, 'test', '7.7.0', 1, 1, 1)",
                params![content_id, snapshot_id],
            )
            .expect("metadata");

        HxsSnapshot::open(&path).expect("fixture verifies");
        Fixture {
            _directory: directory,
            path,
        }
    }

    fn macro_hash(value: &str) -> [u8; 32] {
        digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-V1-MACRO");
            framed_text(hasher, value);
        })
    }

    fn schema_hash(sheet_name: &str) -> [u8; 32] {
        digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-V1-SCHEMA");
            framed_text(hasher, sheet_name);
            hasher.update(0_u32.to_le_bytes());
            hasher.update(0_u32.to_le_bytes());
            hasher.update(0_u32.to_le_bytes());
            hasher.update(1_u32.to_le_bytes());
        })
    }

    fn row_technical_hash(sheet_name: &str, row_id: u32, subrow_id: u16) -> [u8; 32] {
        digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-V1-ROW-TECHNICAL");
            row_identity(hasher, sheet_name, row_id, subrow_id);
        })
    }

    fn row_string_hash(
        sheet_name: &str,
        row_id: u32,
        subrow_id: u16,
        column_index: u32,
        macro_hash: &[u8; 32],
    ) -> [u8; 32] {
        digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-V1-ROW-STRINGS");
            row_identity(hasher, sheet_name, row_id, subrow_id);
            hasher.update(column_index.to_le_bytes());
            hasher.update(macro_hash);
            hasher.update([0]);
        })
    }

    fn row_hash(
        sheet_name: &str,
        row_id: u32,
        subrow_id: u16,
        technical_hash: &[u8; 32],
        string_hash: &[u8; 32],
    ) -> [u8; 32] {
        digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-V1-ROW");
            row_identity(hasher, sheet_name, row_id, subrow_id);
            hasher.update(technical_hash);
            hasher.update(string_hash);
        })
    }

    fn sheet_rows_hash(domain: &str, sheet_name: &str, rows: &[(u32, u16, [u8; 32])]) -> [u8; 32] {
        digest(|hasher| {
            hasher.update(domain.as_bytes());
            framed_text(hasher, sheet_name);
            for (row_id, subrow_id, row_hash) in rows {
                hasher.update(row_id.to_le_bytes());
                hasher.update(u32::from(*subrow_id).to_le_bytes());
                hasher.update(row_hash);
            }
        })
    }

    fn row_identity(hasher: &mut Sha256, sheet_name: &str, row_id: u32, subrow_id: u16) {
        framed_text(hasher, sheet_name);
        hasher.update(row_id.to_le_bytes());
        hasher.update(u32::from(subrow_id).to_le_bytes());
    }

    fn framed_text(hasher: &mut Sha256, value: &str) {
        hasher.update(
            u32::try_from(value.len())
                .expect("fixture text fits framing")
                .to_le_bytes(),
        );
        hasher.update(value.as_bytes());
    }

    fn digest(update: impl FnOnce(&mut Sha256)) -> [u8; 32] {
        let mut hasher = Sha256::new();
        update(&mut hasher);
        hasher.finalize().into()
    }

    fn hex(bytes: &[u8; 32]) -> String {
        let mut result = String::with_capacity(64);
        for byte in bytes {
            write!(&mut result, "{byte:02x}").expect("String writes cannot fail");
        }
        result
    }
}
