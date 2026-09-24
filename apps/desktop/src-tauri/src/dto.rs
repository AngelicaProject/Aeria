use std::path::Path;

use aeria_core::{
    DetachReason, ReviewState, SourceBinding, SourceStatus, TranslationUnit, TranslationUnitId,
};
use aeria_projects::RegistryEntry;
use aeria_rebase::SheetSchemaUpdate;
use aeria_workspace::{
    ProjectSession, SheetTranslationProgress, SourceUpdateReport, TranslationCellView,
    TranslationContextCellView, TranslationOverlayView, TranslationRowCursor, TranslationRowPage,
    TranslationRowView,
};
use serde::{Deserialize, Serialize};

use crate::error::CommandError;

/// Opaque identity for an active source-package generation job.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePackageJobDto {
    pub job_id: String,
}

/// A source occurrence coordinate accepted and returned by desktop commands.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBindingDto {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
    pub column_index: u32,
}

impl From<&SourceBinding> for SourceBindingDto {
    fn from(binding: &SourceBinding) -> Self {
        Self {
            sheet_name: binding.sheet_name().to_owned(),
            row_id: binding.row_id(),
            subrow_id: binding.subrow_id(),
            column_index: binding.column_index(),
        }
    }
}

impl From<SourceBindingDto> for SourceBinding {
    fn from(binding: SourceBindingDto) -> Self {
        Self::new(
            binding.sheet_name,
            binding.row_id,
            binding.subrow_id,
            binding.column_index,
        )
    }
}

/// The review-state values supported by the desktop protocol.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewStateDto {
    Draft,
    Reviewed,
    NeedsReview,
}

impl From<ReviewState> for ReviewStateDto {
    fn from(state: ReviewState) -> Self {
        match state {
            ReviewState::Draft => Self::Draft,
            ReviewState::Reviewed => Self::Reviewed,
            ReviewState::NeedsReview => Self::NeedsReview,
        }
    }
}

impl From<ReviewStateDto> for ReviewState {
    fn from(state: ReviewStateDto) -> Self {
        match state {
            ReviewStateDto::Draft => Self::Draft,
            ReviewStateDto::Reviewed => Self::Reviewed,
            ReviewStateDto::NeedsReview => Self::NeedsReview,
        }
    }
}

/// Metadata for one verified HXS sheet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSheetDto {
    pub name: String,
    pub effective_language: String,
    pub row_count: u64,
    pub translatable_cell_count: usize,
}

/// Owned metadata for the currently active project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryDto {
    pub repository_root: String,
    pub source_package_path: String,
    pub source_package_id: String,
    pub source_language: String,
    pub target_language: String,
    pub source_content_id: String,
    pub source_snapshot_id: String,
    pub game_version: String,
    pub scope: String,
    pub sheets: Vec<ProjectSheetDto>,
    pub detached_unit_count: usize,
}

impl ProjectSummaryDto {
    pub(crate) fn from_session(session: &ProjectSession) -> Self {
        let source_metadata = session.source().metadata();
        let workspace_metadata = session.workspace().metadata();
        let sheets = session
            .source()
            .sheets()
            .into_iter()
            .map(|sheet| {
                let translatable_cell_count = session
                    .source_package()
                    .guidance_index()
                    .translatable_cell_count(&sheet.name);
                ProjectSheetDto {
                    name: sheet.name,
                    effective_language: sheet.effective_language,
                    row_count: sheet.row_count,
                    translatable_cell_count,
                }
            })
            .collect();

        Self {
            repository_root: session.repository_root().to_string_lossy().into_owned(),
            source_package_path: session.source_package_path().to_string_lossy().into_owned(),
            source_package_id: session.source_package().package_id().to_owned(),
            source_language: workspace_metadata.source_language().to_owned(),
            target_language: workspace_metadata.target_language().to_owned(),
            source_content_id: workspace_metadata.source_content_id().to_owned(),
            source_snapshot_id: source_metadata.snapshot_id,
            game_version: source_metadata.game_version,
            scope: source_metadata.scope,
            sheets,
            detached_unit_count: session.detached_units().count(),
        }
    }
}

/// Workspace coverage for one sheet. Counts are bounded by the sheet's
/// `translatableCellCount`; sheets without translations are omitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetProgressDto {
    pub sheet_name: String,
    pub translated: usize,
    pub reviewed: usize,
    pub needs_review: usize,
}

impl From<SheetTranslationProgress> for SheetProgressDto {
    fn from(progress: SheetTranslationProgress) -> Self {
        Self {
            sheet_name: progress.sheet_name,
            translated: progress.translated,
            reviewed: progress.reviewed,
            needs_review: progress.needs_review,
        }
    }
}

/// The result of a successful project launch. Local recent-project persistence
/// is convenience state, so its failure is returned as a warning while the
/// active project remains usable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenResultDto {
    pub project: ProjectSummaryDto,
    pub warning: Option<CommandError>,
    /// Present when opening applied a source update.
    pub source_update: Option<SourceUpdateReportDto>,
}

/// Why a translation unit is detached from the current source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DetachReasonDto {
    SheetRemoved,
    SheetUnavailable,
    RowRemoved,
    CellRemoved,
    ColumnUnresolved,
    NotTranslatable,
    BindingConflict,
}

impl From<DetachReason> for DetachReasonDto {
    fn from(reason: DetachReason) -> Self {
        match reason {
            DetachReason::SheetRemoved => Self::SheetRemoved,
            DetachReason::SheetUnavailable => Self::SheetUnavailable,
            DetachReason::RowRemoved => Self::RowRemoved,
            DetachReason::CellRemoved => Self::CellRemoved,
            DetachReason::ColumnUnresolved => Self::ColumnUnresolved,
            DetachReason::NotTranslatable => Self::NotTranslatable,
            DetachReason::BindingConflict => Self::BindingConflict,
        }
    }
}

/// A sheet whose managed units were bound in another schema generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetSchemaUpdateDto {
    pub sheet_name: String,
    pub removed: bool,
    /// The sheet still exists in the game but the source could not read it.
    pub unavailable: bool,
    pub mapped_columns: usize,
    pub unresolved_columns: usize,
}

impl From<&SheetSchemaUpdate> for SheetSchemaUpdateDto {
    fn from(update: &SheetSchemaUpdate) -> Self {
        let mapped_columns = update
            .columns
            .iter()
            .filter(|column| column.column.is_some())
            .count();
        Self {
            sheet_name: update.sheet_name.clone(),
            removed: update.schema_hash.is_none() && !update.unavailable,
            unavailable: update.unavailable,
            mapped_columns,
            unresolved_columns: update.columns.len() - mapped_columns,
        }
    }
}

/// Counts and schema changes of a previewed or applied source update.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUpdateReportDto {
    pub previous_content_id: String,
    pub content_id: String,
    pub game_version: String,
    pub previous_format_version: u8,
    pub unchanged: usize,
    pub encoding_changed: usize,
    pub source_changed: usize,
    pub detached: usize,
    pub newly_detached: usize,
    pub reattached: usize,
    pub column_mapped: usize,
    pub row_moved: usize,
    pub changed_units: usize,
    pub sheet_schema_updates: Vec<SheetSchemaUpdateDto>,
}

impl From<&SourceUpdateReport> for SourceUpdateReportDto {
    fn from(report: &SourceUpdateReport) -> Self {
        let plan = &report.plan;
        let summary = plan.summary;
        Self {
            previous_content_id: plan.previous_content_id.clone(),
            content_id: plan.source_snapshot.content_id.clone(),
            game_version: plan.source_snapshot.game_version.clone(),
            previous_format_version: report.previous_format_version,
            unchanged: summary.unchanged,
            encoding_changed: summary.encoding_changed,
            source_changed: summary.source_changed,
            detached: summary.detached,
            newly_detached: summary.newly_detached,
            reattached: summary.reattached,
            column_mapped: summary.column_mapped,
            row_moved: summary.row_moved,
            changed_units: summary.changed_units,
            sheet_schema_updates: plan.sheet_schema_updates.iter().map(Into::into).collect(),
        }
    }
}

/// A translation unit preserved without a current source occurrence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachedUnitDto {
    pub translation_unit_id: String,
    pub last_source_binding: SourceBindingDto,
    pub reason: DetachReasonDto,
    pub target_macro: String,
    pub review_state: ReviewStateDto,
    pub translator_note: Option<String>,
}

impl DetachedUnitDto {
    pub(crate) fn from_unit(unit: &TranslationUnit) -> Option<Self> {
        let SourceStatus::Detached(reason) = unit.source_status() else {
            return None;
        };
        Some(Self {
            translation_unit_id: unit.id().to_string(),
            last_source_binding: unit.source_binding().into(),
            reason: reason.into(),
            target_macro: unit.target_macro().to_owned(),
            review_state: unit.review_state().into(),
            translator_note: unit.translator_note().map(str::to_owned),
        })
    }
}

/// Cheap filesystem-only presentation state for one recent project.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecentProjectAvailability {
    Ready,
    RepositoryMissing,
    SourcePackageMissing,
    RepositoryAndSourceMissing,
}

/// Frontend-facing metadata for a local recent project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectDto {
    pub id: String,
    pub repository_root: String,
    pub source_package_path: String,
    pub source_package_id: String,
    pub source_language: String,
    pub target_language: String,
    pub game_version: String,
    pub last_opened_at_unix_ms: u64,
    pub availability: RecentProjectAvailability,
}

impl RecentProjectDto {
    pub(crate) fn from_registry_entry(entry: RegistryEntry) -> Self {
        let repository_exists = Path::new(&entry.repository_root).is_dir();
        let source_exists = Path::new(&entry.source_package_path).is_file();
        let availability = match (repository_exists, source_exists) {
            (true, true) => RecentProjectAvailability::Ready,
            (false, true) => RecentProjectAvailability::RepositoryMissing,
            (true, false) => RecentProjectAvailability::SourcePackageMissing,
            (false, false) => RecentProjectAvailability::RepositoryAndSourceMissing,
        };
        Self {
            id: entry.id,
            repository_root: entry.repository_root,
            source_package_path: entry.source_package_path,
            source_package_id: entry.source_package_id,
            source_language: entry.source_language,
            target_language: entry.target_language,
            game_version: entry.game_version,
            last_opened_at_unix_ms: entry.last_opened_at_unix_ms,
            availability,
        }
    }
}

/// A row/subrow cursor for the bounded desktop translation read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRowCursorDto {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
}

impl From<&TranslationRowCursor> for TranslationRowCursorDto {
    fn from(cursor: &TranslationRowCursor) -> Self {
        Self {
            sheet_name: cursor.sheet_name().to_owned(),
            row_id: cursor.row_id(),
            subrow_id: cursor.subrow_id(),
        }
    }
}

impl From<TranslationRowCursorDto> for TranslationRowCursor {
    fn from(cursor: TranslationRowCursorDto) -> Self {
        Self::new(cursor.sheet_name, cursor.row_id, cursor.subrow_id)
    }
}

/// One read-only technical context cell in a logical source row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationContextCellDto {
    pub column_index: u32,
    pub source_macro: String,
}

impl From<TranslationContextCellView> for TranslationContextCellDto {
    fn from(cell: TranslationContextCellView) -> Self {
        Self {
            column_index: cell.column_index,
            source_macro: cell.source_macro,
        }
    }
}

/// One translatable String cell and its optional translation overlay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationCellDto {
    pub source_binding: SourceBindingDto,
    pub source_macro: String,
    /// The source has no letters outside protected structure.
    pub formatting_only: bool,
    pub translation: Option<TranslationOverlayDto>,
}

impl From<TranslationCellView> for TranslationCellDto {
    fn from(cell: TranslationCellView) -> Self {
        Self {
            source_binding: (&cell.source_binding).into(),
            source_macro: cell.source_macro,
            formatting_only: cell.formatting_only,
            translation: cell.translation.map(Into::into),
        }
    }
}

/// One logical source row for the desktop editor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRowDto {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
    pub context: Vec<TranslationContextCellDto>,
    pub cells: Vec<TranslationCellDto>,
}

impl From<TranslationRowView> for TranslationRowDto {
    fn from(row: TranslationRowView) -> Self {
        Self {
            sheet_name: row.sheet_name,
            row_id: row.row_id,
            subrow_id: row.subrow_id,
            context: row.context.into_iter().map(Into::into).collect(),
            cells: row.cells.into_iter().map(Into::into).collect(),
        }
    }
}

/// One bounded page of logical source rows and optional workspace overlays.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRowPageDto {
    pub rows: Vec<TranslationRowDto>,
    pub next_after: Option<TranslationRowCursorDto>,
}

impl From<TranslationRowPage> for TranslationRowPageDto {
    fn from(page: TranslationRowPage) -> Self {
        Self {
            rows: page.rows.into_iter().map(Into::into).collect(),
            next_after: page.next_after.as_ref().map(Into::into),
        }
    }
}

/// The sparse workspace data overlaid on one source occurrence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationOverlayDto {
    pub translation_unit_id: String,
    pub target_macro: String,
    pub review_state: ReviewStateDto,
    pub translator_note: Option<String>,
}

impl From<TranslationOverlayView> for TranslationOverlayDto {
    fn from(overlay: TranslationOverlayView) -> Self {
        Self {
            translation_unit_id: overlay.translation_unit_id.to_string(),
            target_macro: overlay.target_macro,
            review_state: overlay.review_state.into(),
            translator_note: overlay.translator_note,
        }
    }
}

/// The canonical textual translation-unit ID returned by target mutations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationUnitIdDto {
    pub translation_unit_id: String,
}

impl From<TranslationUnitId> for TranslationUnitIdDto {
    fn from(id: TranslationUnitId) -> Self {
        Self {
            translation_unit_id: id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use aeria_projects::RegistryEntry;

    #[test]
    fn source_binding_mapping_preserves_all_coordinates() {
        let binding = SourceBinding::new("Synthetic", 42, 3, 7);

        let dto = SourceBindingDto::from(&binding);
        assert_eq!(SourceBinding::from(dto), binding);
    }

    #[test]
    fn translation_row_mapping_preserves_context_cells_and_explicit_empty_overlays() {
        let translated_id = TranslationUnitId::from_bytes([0xab; 32]);
        let translated_binding = SourceBinding::new("Synthetic", 42, 0, 0);
        let empty_binding = SourceBinding::new("Synthetic", 7, 0, 0);
        let page = TranslationRowPage {
            rows: vec![TranslationRowView {
                sheet_name: "Synthetic".to_owned(),
                row_id: 42,
                subrow_id: 0,
                context: vec![TranslationContextCellView {
                    column_index: 3,
                    source_macro: "Context field".to_owned(),
                }],
                cells: vec![
                    TranslationCellView {
                        source_binding: translated_binding,
                        source_macro: "source".to_owned(),
                        formatting_only: false,
                        translation: Some(TranslationOverlayView {
                            translation_unit_id: translated_id,
                            target_macro: String::new(),
                            review_state: ReviewState::NeedsReview,
                            translator_note: Some("check later".to_owned()),
                        }),
                    },
                    TranslationCellView {
                        source_binding: empty_binding,
                        source_macro: "...".to_owned(),
                        formatting_only: true,
                        translation: None,
                    },
                ],
            }],
            next_after: Some(TranslationRowCursor::new("Synthetic", 7, 0)),
        };

        let dto = TranslationRowPageDto::from(page);
        assert_eq!(dto.rows[0].row_id, 42);
        assert_eq!(dto.rows[0].context[0].source_macro, "Context field");
        let overlay = dto.rows[0].cells[0]
            .translation
            .as_ref()
            .expect("explicit empty target remains an overlay");
        assert_eq!(
            overlay.translation_unit_id,
            "tu1:abababababababababababababababababababababababababababababababab"
        );
        assert_eq!(overlay.target_macro, "");
        assert_eq!(overlay.review_state, ReviewStateDto::NeedsReview);
        assert_eq!(overlay.translator_note.as_deref(), Some("check later"));
        assert!(dto.rows[0].cells[1].translation.is_none());
        assert!(!dto.rows[0].cells[0].formatting_only);
        assert!(dto.rows[0].cells[1].formatting_only);
        assert_eq!(dto.next_after.expect("cursor").row_id, 7);
    }

    #[test]
    fn only_detached_units_map_to_detached_dtos() {
        let unit = TranslationUnit::new(
            TranslationUnitId::from_bytes([0xcd; 32]),
            SourceBinding::new("Addon", 4021, 0, 2),
            aeria_core::SourceFingerprint::new(
                aeria_core::Sha256Hash::from_bytes([1; 32]),
                None,
                aeria_core::Sha256Hash::from_bytes([2; 32]),
            ),
            "Готово",
        );
        assert!(DetachedUnitDto::from_unit(&unit).is_none());

        let mut detached = unit;
        detached.set_review_state(ReviewState::Reviewed);
        detached.detach(DetachReason::ColumnUnresolved);
        let dto = DetachedUnitDto::from_unit(&detached).expect("detached unit");
        assert_eq!(dto.reason, DetachReasonDto::ColumnUnresolved);
        assert_eq!(dto.last_source_binding.row_id, 4021);
        assert_eq!(dto.last_source_binding.column_index, 2);
        assert_eq!(dto.target_macro, "Готово");
        assert_eq!(dto.review_state, ReviewStateDto::Reviewed);
    }

    #[test]
    fn review_state_mapping_is_explicit() {
        assert_eq!(
            ReviewStateDto::from(ReviewState::Draft),
            ReviewStateDto::Draft
        );
        assert_eq!(
            ReviewState::from(ReviewStateDto::Reviewed),
            ReviewState::Reviewed
        );
        assert_eq!(
            ReviewStateDto::from(ReviewState::NeedsReview),
            ReviewStateDto::NeedsReview
        );
    }

    #[test]
    fn recent_availability_checks_only_filesystem_presence() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let source = repository.join("test-fixtures/synthetic.hxs");
        let base = RegistryEntry {
            id: "local-id".to_owned(),
            repository_root: repository.to_string_lossy().into_owned(),
            source_package_path: source.to_string_lossy().into_owned(),
            source_package_id:
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
            source_language: "en".to_owned(),
            target_language: "fr".to_owned(),
            game_version: "test".to_owned(),
            last_opened_at_unix_ms: 1,
        };
        assert_eq!(
            RecentProjectDto::from_registry_entry(base.clone()).availability,
            RecentProjectAvailability::Ready
        );

        let mut missing_repository = base.clone();
        missing_repository.repository_root = repository
            .join("missing-repository")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            RecentProjectDto::from_registry_entry(missing_repository).availability,
            RecentProjectAvailability::RepositoryMissing
        );

        let mut missing_source = base.clone();
        missing_source.source_package_path = repository
            .join("missing-source.hsp")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            RecentProjectDto::from_registry_entry(missing_source).availability,
            RecentProjectAvailability::SourcePackageMissing
        );

        let mut both_missing = base;
        both_missing.repository_root = repository
            .join("missing-repository")
            .to_string_lossy()
            .into_owned();
        both_missing.source_package_path = repository
            .join("missing-source.hsp")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            RecentProjectDto::from_registry_entry(both_missing).availability,
            RecentProjectAvailability::RepositoryAndSourceMissing
        );
    }
}
