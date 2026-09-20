//! Transactional application-level translation mutations.

use aeria_core::{
    ReviewState, SourceBinding, SourceFingerprint, TranslationUnit, TranslationUnitId,
};
use thiserror::Error;

use crate::{ProjectSession, WorkspaceError, WorkspaceStoreError};

/// Errors raised while applying one transactional translation mutation.
#[derive(Debug, Error)]
pub enum TranslationMutationError {
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
        "translation unit {translation_unit_id} at {source_binding:?} has a stale source fingerprint: persisted {persisted:?}, verified {verified:?}"
    )]
    SourceIntegrity {
        translation_unit_id: TranslationUnitId,
        source_binding: Box<SourceBinding>,
        persisted: Box<SourceFingerprint>,
        verified: Box<SourceFingerprint>,
    },
}

impl ProjectSession {
    /// Creates or updates the translation unit at one verified source binding.
    ///
    /// A missing unit is created with a stable ID derived from the verified
    /// HXS occurrence. An existing unit keeps its durable ID and uses the
    /// normal workspace target-edit semantics. An explicitly empty target is
    /// a real sparse unit, not an instruction to remove one.
    ///
    /// # Errors
    ///
    /// Returns a typed workspace, persistence, or source-integrity error.
    pub fn set_target(
        &mut self,
        source_binding: &SourceBinding,
        target_macro: &str,
    ) -> Result<TranslationUnitId, TranslationMutationError> {
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
            let id = self.workspace.create_unit_from_hxs(
                self.source_package.source(),
                source_binding.sheet_name(),
                source_binding.row_id(),
                source_binding.subrow_id(),
                source_binding.column_index(),
                target_macro,
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
        let source_binding = unit.source_binding().clone();
        let verified = crate::verified_fingerprint(self.source_package.source(), &source_binding)?;
        if unit.source_fingerprint() != &verified {
            return Err(TranslationMutationError::SourceIntegrity {
                translation_unit_id,
                source_binding: Box::new(source_binding),
                persisted: Box::new(*unit.source_fingerprint()),
                verified: Box::new(verified),
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
