use aeria_core::{ReviewState, SourceBinding, TranslationUnitId};
use aeria_workspace::{
    ProjectSession, TranslationEntryPage, TranslationEntryView, TranslationOverlayView,
};
use serde::{Deserialize, Serialize};

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
}

/// Owned metadata for the currently active project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryDto {
    pub repository_root: String,
    pub source_path: String,
    pub source_language: String,
    pub target_language: String,
    pub source_content_id: String,
    pub source_snapshot_id: String,
    pub game_version: String,
    pub scope: String,
    pub sheets: Vec<ProjectSheetDto>,
}

impl ProjectSummaryDto {
    pub(crate) fn from_session(session: &ProjectSession) -> Self {
        let source_metadata = session.source().metadata();
        let workspace_metadata = session.workspace().metadata();
        let sheets = session
            .source()
            .sheets()
            .into_iter()
            .map(|sheet| ProjectSheetDto {
                name: sheet.name,
                effective_language: sheet.effective_language,
                row_count: sheet.row_count,
            })
            .collect();

        Self {
            repository_root: session.repository_root().to_string_lossy().into_owned(),
            source_path: session.source_path().to_string_lossy().into_owned(),
            source_language: workspace_metadata.source_language().to_owned(),
            target_language: workspace_metadata.target_language().to_owned(),
            source_content_id: workspace_metadata.source_content_id().to_owned(),
            source_snapshot_id: workspace_metadata.source_snapshot_id().to_owned(),
            game_version: source_metadata.game_version,
            scope: source_metadata.scope,
            sheets,
        }
    }
}

/// One bounded page of source occurrences and optional workspace overlays.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationEntryPageDto {
    pub entries: Vec<TranslationEntryDto>,
    pub next_after: Option<SourceBindingDto>,
}

impl From<TranslationEntryPage> for TranslationEntryPageDto {
    fn from(page: TranslationEntryPage) -> Self {
        Self {
            entries: page.entries.into_iter().map(Into::into).collect(),
            next_after: page.next_after.as_ref().map(Into::into),
        }
    }
}

/// One source occurrence and its optional translation overlay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationEntryDto {
    pub source_binding: SourceBindingDto,
    pub source_macro: String,
    pub translation: Option<TranslationOverlayDto>,
}

impl From<TranslationEntryView> for TranslationEntryDto {
    fn from(entry: TranslationEntryView) -> Self {
        Self {
            source_binding: (&entry.source_binding).into(),
            source_macro: entry.source_macro,
            translation: entry.translation.map(Into::into),
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
    use super::*;

    #[test]
    fn source_binding_mapping_preserves_all_coordinates() {
        let binding = SourceBinding::new("Synthetic", 42, 3, 7);

        let dto = SourceBindingDto::from(&binding);
        assert_eq!(SourceBinding::from(dto), binding);
    }

    #[test]
    fn translation_page_mapping_preserves_missing_and_explicit_empty_overlays() {
        let translated_id = TranslationUnitId::from_bytes([0xab; 32]);
        let translated_binding = SourceBinding::new("Synthetic", 42, 0, 0);
        let empty_binding = SourceBinding::new("Synthetic", 7, 0, 0);
        let page = TranslationEntryPage {
            entries: vec![
                TranslationEntryView {
                    source_binding: translated_binding,
                    source_macro: "source".to_owned(),
                    translation: Some(TranslationOverlayView {
                        translation_unit_id: translated_id,
                        target_macro: String::new(),
                        review_state: ReviewState::NeedsReview,
                        translator_note: Some("check later".to_owned()),
                    }),
                },
                TranslationEntryView {
                    source_binding: empty_binding.clone(),
                    source_macro: "untranslated".to_owned(),
                    translation: None,
                },
            ],
            next_after: Some(empty_binding),
        };

        let dto = TranslationEntryPageDto::from(page);
        assert_eq!(dto.entries[0].source_binding.row_id, 42);
        let overlay = dto.entries[0]
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
        assert!(dto.entries[1].translation.is_none());
        assert_eq!(dto.next_after.expect("cursor").row_id, 7);
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
}
