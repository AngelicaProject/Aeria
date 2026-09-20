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
  sourcePackagePath: string;
  sourcePackageId: string;
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

export type TranslationRowCursorDto = {
  sheetName: string;
  rowId: number;
  subrowId: number;
};

export type TranslationContextCellDto = {
  columnIndex: number;
  sourceMacro: string;
};

export type TranslationCellDto = {
  sourceBinding: SourceBinding;
  sourceMacro: string;
  translation: TranslationOverlayDto | null;
};

export type TranslationRowDto = {
  sheetName: string;
  rowId: number;
  subrowId: number;
  context: TranslationContextCellDto[];
  cells: TranslationCellDto[];
};

export type TranslationRowPageDto = {
  rows: TranslationRowDto[];
  nextAfter: TranslationRowCursorDto | null;
};

export type TranslationUnitIdDto = {
  translationUnitId: string;
};
