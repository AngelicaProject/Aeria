export type CommandError = {
  code: string;
  message: string;
};

export type ReviewState = "draft" | "reviewed" | "needsReview";

export type SourceBinding = {
  sheetName: string;
  rowId: number;
  subrowId: number;
  columnIndex: number;
};

export type ProjectSheetDto = {
  name: string;
  effectiveLanguage: string;
  rowCount: number;
};

export type ProjectSummaryDto = {
  repositoryRoot: string;
  sourcePath: string;
  sourceLanguage: string;
  targetLanguage: string;
  sourceContentId: string;
  sourceSnapshotId: string;
  gameVersion: string;
  scope: string;
  sheets: ProjectSheetDto[];
};

export type TranslationOverlayDto = {
  translationUnitId: string;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type TranslationEntryDto = {
  sourceBinding: SourceBinding;
  sourceMacro: string;
  translation: TranslationOverlayDto | null;
};

export type TranslationEntryPageDto = {
  entries: TranslationEntryDto[];
  nextAfter: SourceBinding | null;
};

export type TranslationUnitIdDto = {
  translationUnitId: string;
};
