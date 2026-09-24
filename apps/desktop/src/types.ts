export type CommandError = {
  code: string;
  message: string;
};

export type AtlasEvent =
  | { type: "started"; protocolVersion: number; language: string | null }
  | { type: "phase"; protocolVersion: number; phase: string }
  | {
      type: "progress";
      protocolVersion: number;
      phase: string;
      sheet: string | null;
      language: string | null;
      sheetIndex: number | null;
      sheetCount: number | null;
      rowsProcessed: number | null;
      sheetCompleted: boolean | null;
    }
  | {
      type: "completed";
      protocolVersion: number;
      packageId: string;
      outputPath: string | null;
      metadata: Record<string, unknown>;
    }
  | {
      type: "failed";
      protocolVersion: number;
      code: string | null;
      message: string | null;
    };

export type SourcePackageEventPayload = {
  jobId: string;
  event: AtlasEvent;
};

export type SourcePackageJobDto = {
  jobId: string;
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
  translatableCellCount: number;
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
  /** Translations preserved without a current source occurrence. */
  detachedUnitCount: number;
};

/** Workspace coverage for one sheet; sheets without translations are omitted. */
export type SheetProgressDto = {
  sheetName: string;
  translated: number;
  reviewed: number;
  needsReview: number;
};

export type ProjectOpenResultDto = {
  project: ProjectSummaryDto;
  warning: CommandError | null;
  /** Present when opening applied a source update. */
  sourceUpdate: SourceUpdateReportDto | null;
};

/** Why a translation is preserved without a current source occurrence. */
export type DetachReason =
  | "sheetRemoved"
  | "sheetUnavailable"
  | "rowRemoved"
  | "cellRemoved"
  | "columnUnresolved"
  | "notTranslatable"
  | "bindingConflict";

export type SheetSchemaUpdateDto = {
  sheetName: string;
  removed: boolean;
  /** The sheet still exists in the game but the source could not read it. */
  unavailable: boolean;
  mappedColumns: number;
  unresolvedColumns: number;
};

/** Counts of a previewed or applied deterministic source update. */
export type SourceUpdateReportDto = {
  previousContentId: string;
  contentId: string;
  gameVersion: string;
  previousFormatVersion: number;
  unchanged: number;
  encodingChanged: number;
  sourceChanged: number;
  detached: number;
  newlyDetached: number;
  reattached: number;
  columnMapped: number;
  rowMoved: number;
  changedUnits: number;
  sheetSchemaUpdates: SheetSchemaUpdateDto[];
};

export type DetachedUnitDto = {
  translationUnitId: string;
  lastSourceBinding: SourceBinding;
  reason: DetachReason;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type RecentProjectAvailability =
  | "ready"
  | "repositoryMissing"
  | "sourcePackageMissing"
  | "repositoryAndSourceMissing";

export type RecentProjectDto = {
  id: string;
  repositoryRoot: string;
  sourcePackagePath: string;
  sourcePackageId: string;
  sourceLanguage: string;
  targetLanguage: string;
  gameVersion: string;
  lastOpenedAtUnixMs: number;
  availability: RecentProjectAvailability;
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
  /** The source has no letters outside macros: punctuation, digits, or number formatting. */
  formattingOnly: boolean;
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

export type GitFileKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "copied"
  | "typeChanged"
  | "untracked"
  | "conflicted";

export type GitFileDto = {
  path: string;
  originalPath: string | null;
  kind: GitFileKind;
  staged: boolean;
  translationData: boolean;
};

export type GitStatusDto = {
  branch: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  mergeInProgress: boolean;
  hasTranslationChanges: boolean;
  files: GitFileDto[];
};

export type GitConfigScope = "system" | "global" | "repository" | "worktree" | "command";

/** The translator identity is the Git author identity. */
export type TranslatorIdentityDto = {
  name: string | null;
  email: string | null;
  nameScope: GitConfigScope | null;
  emailScope: GitConfigScope | null;
};

export type GitRemoteDto = {
  name: string;
  url: string;
};

export type GitRuntimeDto = {
  /** `git --version` output, or null when Git cannot be run. */
  version: string | null;
  origin: "system" | "bundled" | "override";
};

export type CollaborationPolicy = "direct" | "pullRequest";

export type CollaborationDto = {
  policy: CollaborationPolicy;
  mainBranch: string | null;
};

export type ContributionDto = {
  mainBranch: string;
  /** Null while on the main branch. */
  branch: string | null;
  published: boolean;
  unmergedCommits: number;
};

export type GitOverviewDto = {
  runtime: GitRuntimeDto;
  repository: GitStatusDto | null;
  identity: TranslatorIdentityDto | null;
  remotes: GitRemoteDto[];
  collaboration: CollaborationDto | null;
  contribution: ContributionDto | null;
};

export type GitBranchDto = {
  name: string;
  remote: boolean;
  current: boolean;
  upstream: string | null;
};

export type AttributionDto = {
  commit: string;
  authorName: string;
  authorEmail: string;
  authoredAt: number;
};

export type UnitAttributionDto = {
  translationUnitId: string;
  translatedBy: AttributionDto | null;
  reviewedBy: AttributionDto | null;
  lastChangedBy: AttributionDto | null;
};

export type GitCommitDto = {
  id: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  /** Seconds since the Unix epoch. */
  authoredAt: number;
  subject: string;
};

export type UnitVersionDto = {
  sourceBinding: SourceBinding;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type UnitChangeKind = "added" | "modified" | "removed";

export type UnitChangeDto = {
  translationUnitId: string;
  kind: UnitChangeKind;
  before: UnitVersionDto | null;
  after: UnitVersionDto | null;
  targetChanged: boolean;
  reviewChanged: boolean;
  noteChanged: boolean;
};

export type RecordVersionDto =
  | { state: "absent" }
  | { state: "valid"; unit: UnitVersionDto }
  | { state: "invalid"; message: string };

export type UnitRevisionDto = {
  commit: GitCommitDto;
  kind: UnitChangeKind;
  before: RecordVersionDto;
  after: RecordVersionDto;
};

export type UnitHistoryDto = {
  translationUnitId: string;
  pending: UnitChangeDto | null;
  revisions: UnitRevisionDto[];
  truncated: boolean;
  translatedBy: AttributionDto | null;
  reviewedBy: AttributionDto | null;
};

export type GitCommitChangesDto = {
  commit: GitCommitDto;
  changes: UnitChangeDto[];
  branchCreated: string | null;
};

export type ContributorDto = {
  name: string;
  email: string;
  translated: number;
  reviewed: number;
  lastAuthoredAt: number;
};

export type UnitConflictDto = {
  translationUnitId: string;
  base: UnitVersionDto | null;
  ours: UnitVersionDto | null;
  theirs: UnitVersionDto | null;
};

export type ConflictResolution = "ours" | "theirs";

export type UnitResolutionDto = {
  translationUnitId: string;
  resolution: ConflictResolution;
};

export type GitIntegration = "upToDate" | "fastForward" | "merged";

export type GitSyncDto = {
  integration: GitIntegration;
  pushed: boolean;
  workspaceChanged: boolean;
  /** When non-empty nothing was integrated; sync again with resolutions. */
  conflicts: UnitConflictDto[];
};

export type GitFinishDto = {
  integration: GitIntegration;
  deletedBranch: string | null;
};

export type AiProviderKind = "openCodeGo" | "openRouter" | "custom";
export type ReasoningEffort = "minimal" | "low" | "medium" | "high";
export type ApiKeyState = "stored" | "missing" | "unavailable";

export type AiModelConfig = {
  id: string;
  contextWindow: number | null;
  reasoningEfforts: ReasoningEffort[];
};

export type AiProviderDto = {
  id: string;
  kind: AiProviderKind;
  name: string;
  baseUrl: string;
  models: AiModelConfig[];
  apiKey: ApiKeyState;
};

export type AiProviderPresetDto = {
  kind: AiProviderKind;
  name: string;
  baseUrl: string | null;
  models: AiModelConfig[];
};

export type AiModelSelection = {
  providerId: string;
  modelId: string;
  effort: ReasoningEffort | null;
};

export type AiSettingsDto = {
  providers: AiProviderDto[];
  agentModel: AiModelSelection | null;
  presets: AiProviderPresetDto[];
};

export type AiProviderInput = {
  id: string | null;
  kind: AiProviderKind;
  name: string;
  baseUrl: string;
  models: AiModelConfig[];
};

export type AiConnectionCheckDto = {
  latencyMs: number;
  model: string | null;
};
